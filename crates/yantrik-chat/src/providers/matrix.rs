//! Matrix provider — Client-Server API with /sync long-poll.
//!
//! Uses ureq for HTTP. Supports rooms, DMs, threads.
//! Unencrypted only for now (E2EE requires olm/megolm which is complex).

use std::time::Duration;

use crate::model::*;
use crate::provider::*;

pub struct MatrixProvider {
    homeserver: String,
    access_token: String,
    /// Only process messages from these room IDs. Empty = all joined rooms.
    allowed_rooms: Vec<String>,
    health: ProviderHealth,
    /// Sync token for incremental updates.
    since: Option<String>,
    /// Our own user ID (to skip our own messages).
    user_id: Option<String>,
    /// Timeout for /sync long-poll (seconds).
    sync_timeout: u64,
}

impl MatrixProvider {
    pub fn new(homeserver: String, access_token: String, allowed_rooms: Vec<String>) -> Self {
        // Normalize: strip trailing slash
        let homeserver = homeserver.trim_end_matches('/').to_string();
        Self {
            homeserver,
            access_token,
            allowed_rooms,
            health: ProviderHealth::Disconnected,
            since: None,
            user_id: None,
            sync_timeout: 30,
        }
    }

    fn api_get(&self, path: &str) -> Result<serde_json::Value, ChatError> {
        let url = format!("{}/_matrix/client/v3{}", self.homeserver, path);
        let resp = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", self.access_token))
            .timeout(Duration::from_secs(self.sync_timeout + 10))
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(429, _) => ChatError::RateLimited(5),
                ureq::Error::Status(code, _) => ChatError::Provider(format!("HTTP {code}")),
                e => ChatError::Network(format!("{e}")),
            })?;

        let text = resp.into_string()
            .map_err(|e| ChatError::Network(format!("read: {e}")))?;
        serde_json::from_str(&text)
            .map_err(|e| ChatError::Provider(format!("invalid JSON: {e}")))
    }

    fn api_put(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, ChatError> {
        let url = format!("{}/_matrix/client/v3{}", self.homeserver, path);
        let resp = ureq::put(&url)
            .set("Authorization", &format!("Bearer {}", self.access_token))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
            .map_err(|e| match e {
                ureq::Error::Status(429, _) => ChatError::RateLimited(5),
                ureq::Error::Status(code, _) => ChatError::Provider(format!("HTTP {code}")),
                e => ChatError::Network(format!("{e}")),
            })?;

        let text = resp.into_string()
            .map_err(|e| ChatError::Network(format!("read: {e}")))?;
        serde_json::from_str(&text)
            .map_err(|e| ChatError::Provider(format!("invalid JSON: {e}")))
    }
}

impl ChatProvider for MatrixProvider {
    fn id(&self) -> &'static str { "matrix" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::Polling,
            media: true,
            voice: false,
            reactions: true,
            typing: true,
            threads: true,
            groups: true,
            channels: true,
            read_receipts: true,
            edits: true,
            deletes: false,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        // Verify token by calling /whoami
        let resp = self.api_get("/account/whoami")?;
        self.user_id = resp.get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        tracing::info!(user = ?self.user_id, "Matrix connected");
        self.health = ProviderHealth::Connected;
        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), ChatError> {
        self.health = ProviderHealth::Disconnected;
        Ok(())
    }

    fn health(&self) -> ProviderHealth {
        self.health
    }

    fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
        // Build /sync URL
        let mut url = format!(
            "/sync?timeout={}&filter={{\"room\":{{\"timeline\":{{\"limit\":50}}}}}}",
            self.sync_timeout * 1000,
        );
        if let Some(ref since) = self.since {
            url.push_str(&format!("&since={}", since));
        }

        let resp = self.api_get(&url)?;

        // Update sync token
        if let Some(next) = resp.get("next_batch").and_then(|v| v.as_str()) {
            self.since = Some(next.to_string());
        }

        let mut events = Vec::new();

        // Process joined rooms
        let rooms = resp.get("rooms")
            .and_then(|r| r.get("join"))
            .and_then(|j| j.as_object());

        if let Some(rooms) = rooms {
            for (room_id, room_data) in rooms {
                // Filter by allowed rooms
                if !self.allowed_rooms.is_empty() && !self.allowed_rooms.contains(room_id) {
                    continue;
                }

                let timeline = room_data.get("timeline")
                    .and_then(|t| t.get("events"))
                    .and_then(|e| e.as_array());

                if let Some(timeline_events) = timeline {
                    for event in timeline_events {
                        if let Some(inbound) = self.parse_timeline_event(room_id, event) {
                            events.push(inbound);
                        }
                    }
                }
            }
        }

        Ok(events)
    }

    fn send(
        &mut self,
        target: &ConversationRef,
        msg: &OutboundMessage,
    ) -> Result<SendReceipt, ChatError> {
        let room_id = &target.id;

        match &msg.content {
            OutboundContent::Text(text) => {
                let txn_id = format!("yantrik_{}", chrono::Utc::now().timestamp_millis());
                let body = serde_json::json!({
                    "msgtype": "m.text",
                    "body": text,
                });

                let path = format!(
                    "/rooms/{}/send/m.room.message/{}",
                    urlencoding_simple(room_id),
                    txn_id,
                );
                let resp = self.api_put(&path, &body)?;
                let event_id = resp.get("event_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&txn_id)
                    .to_string();

                Ok(SendReceipt {
                    message: MessageRef { provider: "matrix".into(), id: event_id },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }

    fn send_typing(&mut self, target: &ConversationRef) -> Result<(), ChatError> {
        let path = format!(
            "/rooms/{}/typing/{}",
            urlencoding_simple(&target.id),
            urlencoding_simple(self.user_id.as_deref().unwrap_or("")),
        );
        let body = serde_json::json!({ "typing": true, "timeout": 5000 });
        self.api_put(&path, &body)?;
        Ok(())
    }

    fn mark_read(
        &mut self,
        target: &ConversationRef,
        up_to: Option<&MessageRef>,
    ) -> Result<(), ChatError> {
        if let Some(msg) = up_to {
            let path = format!(
                "/rooms/{}/read_markers",
                urlencoding_simple(&target.id),
            );
            let body = serde_json::json!({
                "m.fully_read": msg.id,
                "m.read": msg.id,
            });
            let url = format!("{}/_matrix/client/v3{}", self.homeserver, path);
            ureq::post(&url)
                .set("Authorization", &format!("Bearer {}", self.access_token))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string())
                .map_err(|e| ChatError::Network(format!("{e}")))?;
        }
        Ok(())
    }
}

impl MatrixProvider {
    fn parse_timeline_event(
        &self,
        room_id: &str,
        event: &serde_json::Value,
    ) -> Option<InboundEvent> {
        let event_type = event.get("type").and_then(|v| v.as_str())?;

        if event_type != "m.room.message" {
            return None;
        }

        let sender = event.get("sender").and_then(|v| v.as_str())?;

        // Skip our own messages
        if Some(sender) == self.user_id.as_deref() {
            return None;
        }

        let event_id = event.get("event_id").and_then(|v| v.as_str())?;
        let content = event.get("content")?;
        let msgtype = content.get("msgtype").and_then(|v| v.as_str())?;
        let origin_ts = event.get("origin_server_ts").and_then(|v| v.as_i64()).unwrap_or(0);

        let msg_content = match msgtype {
            "m.text" => {
                let body = content.get("body").and_then(|v| v.as_str())?;
                MessageContent::Text { text: body.to_string() }
            }
            "m.image" => {
                let url = content.get("url").and_then(|v| v.as_str()).unwrap_or("");
                MessageContent::Image {
                    media: MediaRef {
                        provider: "matrix".into(),
                        id: url.to_string(),
                        mime_type: content.get("info")
                            .and_then(|i| i.get("mimetype"))
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string()),
                        size_bytes: content.get("info")
                            .and_then(|i| i.get("size"))
                            .and_then(|s| s.as_u64()),
                        filename: content.get("body").and_then(|b| b.as_str()).map(|s| s.to_string()),
                    },
                    caption: None,
                }
            }
            "m.file" | "m.audio" | "m.video" => {
                let url = content.get("url").and_then(|v| v.as_str()).unwrap_or("");
                MessageContent::File {
                    media: MediaRef {
                        provider: "matrix".into(),
                        id: url.to_string(),
                        mime_type: content.get("info")
                            .and_then(|i| i.get("mimetype"))
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string()),
                        size_bytes: content.get("info")
                            .and_then(|i| i.get("size"))
                            .and_then(|s| s.as_u64()),
                        filename: content.get("body").and_then(|b| b.as_str()).map(|s| s.to_string()),
                    },
                    caption: None,
                }
            }
            "m.location" => {
                let geo_uri = content.get("geo_uri").and_then(|v| v.as_str()).unwrap_or("");
                // Parse geo:lat,lon
                let coords: Vec<f64> = geo_uri
                    .strip_prefix("geo:")
                    .unwrap_or("")
                    .split(',')
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if coords.len() >= 2 {
                    MessageContent::Location {
                        lat: coords[0],
                        lon: coords[1],
                        label: content.get("body").and_then(|b| b.as_str()).map(|s| s.to_string()),
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        // Extract display name from sender (strip @user:server → user)
        let display_name = sender
            .strip_prefix('@')
            .and_then(|s| s.split(':').next())
            .unwrap_or(sender)
            .to_string();

        // Determine conversation kind (heuristic: <= 2 members = DM)
        let kind = ConversationKind::Room;

        // Check for thread relation
        let reply_to = content.get("m.relates_to")
            .and_then(|r| r.get("m.in_reply_to"))
            .and_then(|r| r.get("event_id"))
            .and_then(|e| e.as_str())
            .map(|id| MessageRef { provider: "matrix".into(), id: id.to_string() });

        Some(InboundEvent::Message(InboundMessage {
            event_id: event_id.to_string(),
            conversation: ConversationRef {
                provider: "matrix".into(),
                kind,
                id: room_id.to_string(),
                parent_id: None,
                title: None,
            },
            message: MessageRef {
                provider: "matrix".into(),
                id: event_id.to_string(),
            },
            sender: ActorRef {
                id: sender.to_string(),
                display_name,
                is_bot: false,
            },
            timestamp_ms: origin_ts,
            content: msg_content,
            reply_to,
            mentions_ai: false, // TODO: check for mention in body
            raw: Some(event.to_string()),
        }))
    }
}

fn urlencoding_simple(s: &str) -> String {
    let mut result = String::new();
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{:02X}", byte));
        }
    }
    result
}
