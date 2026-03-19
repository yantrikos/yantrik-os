//! WhatsApp provider — Cloud API outbound + webhook inbound.
//!
//! Outbound: ureq to Cloud API (existing pattern).
//! Inbound: handle_webhook() parses Meta webhook payloads.
//! Requires cloudflared tunnel for local-first webhook reception.

use crate::model::*;
use crate::provider::*;

pub struct WhatsAppProvider {
    phone_number_id: String,
    access_token: String,
    /// Default recipient for outbound.
    default_recipient: Option<String>,
    /// Webhook verify token (for Meta's challenge handshake).
    verify_token: String,
    health: ProviderHealth,
}

impl WhatsAppProvider {
    pub fn new(
        phone_number_id: String,
        access_token: String,
        default_recipient: Option<String>,
        verify_token: String,
    ) -> Self {
        Self {
            phone_number_id,
            access_token,
            default_recipient,
            verify_token,
            health: ProviderHealth::Disconnected,
        }
    }

    fn api_url(&self) -> String {
        format!("https://graph.facebook.com/v21.0/{}/messages", self.phone_number_id)
    }
}

impl ChatProvider for WhatsAppProvider {
    fn id(&self) -> &'static str { "whatsapp" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::Webhook,
            media: true,
            voice: true,
            reactions: true,
            typing: false,
            threads: false,
            groups: true,
            channels: false,
            read_receipts: true,
            edits: false,
            deletes: false,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        // Verify token by calling the phone number endpoint
        let url = format!("https://graph.facebook.com/v21.0/{}", self.phone_number_id);
        let resp = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", self.access_token))
            .call()
            .map_err(|e| ChatError::Auth(format!("WhatsApp auth check: {e}")))?;

        let text = resp.into_string()
            .map_err(|e| ChatError::Network(format!("read: {e}")))?;
        let val: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ChatError::Provider(format!("JSON: {e}")))?;

        if val.get("error").is_some() {
            let msg = val.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(ChatError::Auth(format!("WhatsApp: {msg}")));
        }

        tracing::info!(phone_id = %self.phone_number_id, "WhatsApp connected");
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

    // WhatsApp uses webhooks, not polling
    fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
        // Webhook-based: events come through handle_webhook
        // Sleep briefly to avoid busy-looping in the manager
        std::thread::sleep(std::time::Duration::from_secs(5));
        Ok(vec![])
    }

    fn handle_webhook(
        &mut self,
        body: &[u8],
        headers: &[(String, String)],
    ) -> Result<(u16, Vec<u8>, Vec<InboundEvent>), ChatError> {
        let body_str = std::str::from_utf8(body)
            .map_err(|e| ChatError::Provider(format!("invalid UTF-8: {e}")))?;

        // Handle verification challenge (GET request encoded as special header)
        for (key, value) in headers {
            if key == "hub.mode" && value == "subscribe" {
                // This is a verification request
                let token = headers.iter()
                    .find(|(k, _)| k == "hub.verify_token")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                if token == self.verify_token {
                    let challenge = headers.iter()
                        .find(|(k, _)| k == "hub.challenge")
                        .map(|(_, v)| v.as_bytes().to_vec())
                        .unwrap_or_default();
                    return Ok((200, challenge, vec![]));
                } else {
                    return Ok((403, b"Forbidden".to_vec(), vec![]));
                }
            }
        }

        // Parse webhook payload
        let payload: serde_json::Value = serde_json::from_str(body_str)
            .map_err(|e| ChatError::Provider(format!("invalid JSON: {e}")))?;

        let mut events = Vec::new();

        // Navigate Meta's nested structure
        let entries = payload.get("entry").and_then(|e| e.as_array());
        if let Some(entries) = entries {
            for entry in entries {
                let changes = entry.get("changes").and_then(|c| c.as_array());
                if let Some(changes) = changes {
                    for change in changes {
                        let value = match change.get("value") {
                            Some(v) => v,
                            None => continue,
                        };

                        let messages = value.get("messages").and_then(|m| m.as_array());
                        if let Some(messages) = messages {
                            // Get contact info for display names
                            let contacts = value.get("contacts").and_then(|c| c.as_array());

                            for msg in messages {
                                if let Some(event) = parse_whatsapp_message(msg, contacts) {
                                    events.push(event);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok((200, b"OK".to_vec(), events))
    }

    fn send(
        &mut self,
        target: &ConversationRef,
        msg: &OutboundMessage,
    ) -> Result<SendReceipt, ChatError> {
        let to = if target.id.is_empty() {
            self.default_recipient.as_deref()
                .ok_or(ChatError::Provider("no recipient specified".into()))?
        } else {
            &target.id
        };

        match &msg.content {
            OutboundContent::Text(text) => {
                let text = if text.len() > 4096 {
                    &text[..text.floor_char_boundary(4096)]
                } else {
                    text.as_str()
                };

                let body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": to,
                    "type": "text",
                    "text": { "body": text },
                });

                let resp = ureq::post(&self.api_url())
                    .set("Authorization", &format!("Bearer {}", self.access_token))
                    .set("Content-Type", "application/json")
                    .send_string(&body.to_string())
                    .map_err(|e| ChatError::Network(format!("send: {e}")))?;

                let text = resp.into_string()
                    .map_err(|e| ChatError::Network(format!("read: {e}")))?;
                let val: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| ChatError::Provider(format!("JSON: {e}")))?;

                let msg_id = val.get("messages")
                    .and_then(|m| m.as_array())
                    .and_then(|a| a.first())
                    .and_then(|m| m.get("id"))
                    .and_then(|i| i.as_str())
                    .unwrap_or("0")
                    .to_string();

                Ok(SendReceipt {
                    message: MessageRef { provider: "whatsapp".into(), id: msg_id },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            OutboundContent::Reaction { target, emoji } => {
                let body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": target.provider, // phone number stored here
                    "type": "reaction",
                    "reaction": {
                        "message_id": target.id,
                        "emoji": emoji,
                    },
                });

                ureq::post(&self.api_url())
                    .set("Authorization", &format!("Bearer {}", self.access_token))
                    .set("Content-Type", "application/json")
                    .send_string(&body.to_string())
                    .map_err(|e| ChatError::Network(format!("reaction: {e}")))?;

                Ok(SendReceipt {
                    message: target.clone(),
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }

    fn mark_read(
        &mut self,
        _target: &ConversationRef,
        up_to: Option<&MessageRef>,
    ) -> Result<(), ChatError> {
        if let Some(msg) = up_to {
            let body = serde_json::json!({
                "messaging_product": "whatsapp",
                "status": "read",
                "message_id": msg.id,
            });

            ureq::post(&self.api_url())
                .set("Authorization", &format!("Bearer {}", self.access_token))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string())
                .map_err(|e| ChatError::Network(format!("mark_read: {e}")))?;
        }
        Ok(())
    }
}

fn parse_whatsapp_message(
    msg: &serde_json::Value,
    contacts: Option<&Vec<serde_json::Value>>,
) -> Option<InboundEvent> {
    let from = msg.get("from").and_then(|v| v.as_str())?;
    let msg_id = msg.get("id").and_then(|v| v.as_str())?;
    let timestamp = msg.get("timestamp").and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0) * 1000;
    let msg_type = msg.get("type").and_then(|v| v.as_str())?;

    // Get display name from contacts
    let display_name = contacts
        .and_then(|cs| {
            cs.iter().find(|c| {
                c.get("wa_id").and_then(|w| w.as_str()) == Some(from)
            })
        })
        .and_then(|c| c.get("profile"))
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or(from)
        .to_string();

    let content = match msg_type {
        "text" => {
            let body = msg.get("text")
                .and_then(|t| t.get("body"))
                .and_then(|b| b.as_str())?;
            MessageContent::Text { text: body.to_string() }
        }
        "image" => {
            let image = msg.get("image")?;
            let media_id = image.get("id").and_then(|i| i.as_str())?;
            let caption = image.get("caption").and_then(|c| c.as_str()).map(|s| s.to_string());
            MessageContent::Image {
                media: MediaRef {
                    provider: "whatsapp".into(),
                    id: media_id.to_string(),
                    mime_type: image.get("mime_type").and_then(|m| m.as_str()).map(|s| s.to_string()),
                    size_bytes: None,
                    filename: None,
                },
                caption,
            }
        }
        "audio" => {
            let audio = msg.get("audio")?;
            let media_id = audio.get("id").and_then(|i| i.as_str())?;
            MessageContent::Voice {
                media: MediaRef {
                    provider: "whatsapp".into(),
                    id: media_id.to_string(),
                    mime_type: audio.get("mime_type").and_then(|m| m.as_str()).map(|s| s.to_string()),
                    size_bytes: None,
                    filename: None,
                },
                duration_ms: None,
            }
        }
        "document" => {
            let doc = msg.get("document")?;
            let media_id = doc.get("id").and_then(|i| i.as_str())?;
            let caption = doc.get("caption").and_then(|c| c.as_str()).map(|s| s.to_string());
            MessageContent::File {
                media: MediaRef {
                    provider: "whatsapp".into(),
                    id: media_id.to_string(),
                    mime_type: doc.get("mime_type").and_then(|m| m.as_str()).map(|s| s.to_string()),
                    size_bytes: None,
                    filename: doc.get("filename").and_then(|f| f.as_str()).map(|s| s.to_string()),
                },
                caption,
            }
        }
        "location" => {
            let loc = msg.get("location")?;
            let lat = loc.get("latitude").and_then(|v| v.as_f64())?;
            let lon = loc.get("longitude").and_then(|v| v.as_f64())?;
            let label = loc.get("name").and_then(|n| n.as_str()).map(|s| s.to_string());
            MessageContent::Location { lat, lon, label }
        }
        "sticker" => {
            let sticker = msg.get("sticker")?;
            let media_id = sticker.get("id").and_then(|i| i.as_str())?;
            MessageContent::Sticker {
                media: MediaRef {
                    provider: "whatsapp".into(),
                    id: media_id.to_string(),
                    mime_type: sticker.get("mime_type").and_then(|m| m.as_str()).map(|s| s.to_string()),
                    size_bytes: None,
                    filename: None,
                },
            }
        }
        _ => return None,
    };

    // Check for context (reply-to)
    let reply_to = msg.get("context")
        .and_then(|c| c.get("id"))
        .and_then(|i| i.as_str())
        .map(|id| MessageRef { provider: from.to_string(), id: id.to_string() });

    Some(InboundEvent::Message(InboundMessage {
        event_id: msg_id.to_string(),
        conversation: ConversationRef {
            provider: "whatsapp".into(),
            kind: ConversationKind::Direct,
            id: from.to_string(),
            parent_id: None,
            title: None,
        },
        message: MessageRef {
            provider: from.to_string(), // Store phone number for reactions
            id: msg_id.to_string(),
        },
        sender: ActorRef {
            id: from.to_string(),
            display_name,
            is_bot: false,
        },
        timestamp_ms: timestamp,
        content,
        reply_to,
        mentions_ai: true, // WhatsApp DMs are always directed at the bot
        raw: Some(msg.to_string()),
    }))
}
