//! Slack provider — Socket Mode WebSocket + Web API REST.
//!
//! Socket Mode avoids public webhooks: Slack sends events over a WebSocket.
//! Requires both a Bot Token and an App-Level Token.
//! Uses tungstenite for WebSocket, ureq for REST API.

use std::net::TcpStream;
use std::time::{Duration, Instant};

use tungstenite::{connect, Message as WsMessage, WebSocket};
use tungstenite::stream::MaybeTlsStream;

use crate::model::*;
use crate::provider::*;

const API_BASE: &str = "https://slack.com/api";

pub struct SlackProvider {
    bot_token: String,
    app_token: String,
    /// Only process messages from these channel IDs. Empty = all.
    allowed_channels: Vec<String>,
    health: ProviderHealth,
    ws: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    bot_user_id: Option<String>,
}

impl SlackProvider {
    pub fn new(bot_token: String, app_token: String, allowed_channels: Vec<String>) -> Self {
        Self {
            bot_token,
            app_token,
            allowed_channels,
            health: ProviderHealth::Disconnected,
            ws: None,
            bot_user_id: None,
        }
    }

    /// Call Slack Web API using ureq.
    fn api_call(&self, method: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value, ChatError> {
        let url = format!("{}/{}", API_BASE, method);

        let resp = if let Some(body) = body {
            ureq::post(&url)
                .set("Authorization", &format!("Bearer {}", self.bot_token))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string())
        } else {
            ureq::post(&url)
                .set("Authorization", &format!("Bearer {}", self.bot_token))
                .set("Content-Type", "application/json")
                .send_string("{}")
        };

        match resp {
            Ok(r) => {
                let text = r.into_string()
                    .map_err(|e| ChatError::Network(format!("read: {e}")))?;
                let val: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| ChatError::Provider(format!("JSON: {e}")))?;

                if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                    let err = val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                    return Err(ChatError::Provider(format!("Slack API: {err}")));
                }
                Ok(val)
            }
            Err(ureq::Error::Status(429, r)) => {
                let retry = r.header("Retry-After")
                    .and_then(|h| h.parse().ok())
                    .unwrap_or(5);
                Err(ChatError::RateLimited(retry))
            }
            Err(ureq::Error::Status(code, _)) => {
                Err(ChatError::Provider(format!("HTTP {code}")))
            }
            Err(e) => Err(ChatError::Network(format!("{e}"))),
        }
    }

    /// Open a Socket Mode WebSocket connection.
    fn connect_socket_mode(&mut self) -> Result<(), ChatError> {
        // Use app-level token to get WebSocket URL
        let url = format!("{}/apps.connections.open", API_BASE);
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.app_token))
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string("")
            .map_err(|e| ChatError::Network(format!("connections.open: {e}")))?;

        let text = resp.into_string()
            .map_err(|e| ChatError::Network(format!("read: {e}")))?;
        let val: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ChatError::Provider(format!("JSON: {e}")))?;

        if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
            return Err(ChatError::Auth(format!("Socket Mode auth failed: {err}")));
        }

        let ws_url = val.get("url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| ChatError::Provider("no WebSocket URL in response".into()))?;

        let (ws, _) = connect(ws_url)
            .map_err(|e| ChatError::Network(format!("WS connect: {e}")))?;

        self.ws = Some(ws);
        Ok(())
    }

    /// Read one message from the WebSocket.
    fn read_ws(&mut self) -> Result<Option<serde_json::Value>, ChatError> {
        let ws = match &mut self.ws {
            Some(ws) => ws,
            None => return Err(ChatError::Network("not connected".into())),
        };

        if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        }

        match ws.read() {
            Ok(WsMessage::Text(text)) => {
                serde_json::from_str(&text).ok().map(|v| Ok(Some(v))).unwrap_or(Ok(None))
            }
            Ok(WsMessage::Ping(data)) => {
                let _ = ws.send(WsMessage::Pong(data));
                Ok(None)
            }
            Ok(WsMessage::Close(_)) => {
                Err(ChatError::Network("WebSocket closed".into()))
            }
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                Ok(None)
            }
            Err(e) => Err(ChatError::Network(format!("WS read: {e}"))),
        }
    }

    /// Acknowledge a Socket Mode envelope.
    fn ack_envelope(&mut self, envelope_id: &str) {
        if let Some(ws) = &mut self.ws {
            let ack = serde_json::json!({ "envelope_id": envelope_id });
            let _ = ws.send(WsMessage::Text(ack.to_string()));
        }
    }
}

impl ChatProvider for SlackProvider {
    fn id(&self) -> &'static str { "slack" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::WebSocket,
            media: true,
            voice: false,
            reactions: true,
            typing: false, // Slack doesn't support bot typing indicators well
            threads: true,
            groups: true,
            channels: true,
            read_receipts: false,
            edits: true,
            deletes: true,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        // Get bot user ID
        let resp = self.api_call("auth.test", None)?;
        self.bot_user_id = resp.get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Open Socket Mode connection
        self.connect_socket_mode()?;

        tracing::info!(bot_id = ?self.bot_user_id, "Slack Socket Mode connected");
        self.health = ProviderHealth::Connected;
        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), ChatError> {
        if let Some(ws) = &mut self.ws {
            let _ = ws.close(None);
        }
        self.ws = None;
        self.health = ProviderHealth::Disconnected;
        Ok(())
    }

    fn health(&self) -> ProviderHealth {
        self.health
    }

    fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
        let mut events = Vec::new();

        for _ in 0..50 {
            let msg = match self.read_ws() {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(e) => return Err(e),
            };

            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match msg_type {
                "events_api" => {
                    // Acknowledge the envelope
                    if let Some(envelope_id) = msg.get("envelope_id").and_then(|v| v.as_str()) {
                        self.ack_envelope(envelope_id);
                    }

                    // Extract the event
                    let payload = match msg.get("payload") {
                        Some(p) => p,
                        None => continue,
                    };

                    let event = match payload.get("event") {
                        Some(e) => e,
                        None => continue,
                    };

                    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

                    if event_type == "message" {
                        // Skip subtypes (bot messages, edits, etc.)
                        if event.get("subtype").is_some() {
                            continue;
                        }

                        if let Some(inbound) = self.parse_slack_message(event) {
                            events.push(inbound);
                        }
                    } else if event_type == "app_mention" {
                        if let Some(inbound) = self.parse_slack_message(event) {
                            // Force mentions_ai = true
                            if let InboundEvent::Message(mut m) = inbound {
                                m.mentions_ai = true;
                                events.push(InboundEvent::Message(m));
                            }
                        }
                    }
                }
                "disconnect" => {
                    tracing::warn!("Slack Socket Mode disconnect requested");
                    return Err(ChatError::Network("Slack requested disconnect".into()));
                }
                "hello" => {
                    tracing::debug!("Slack Socket Mode hello received");
                }
                _ => {}
            }
        }

        Ok(events)
    }

    fn send(
        &mut self,
        target: &ConversationRef,
        msg: &OutboundMessage,
    ) -> Result<SendReceipt, ChatError> {
        match &msg.content {
            OutboundContent::Text(text) => {
                let mut body = serde_json::json!({
                    "channel": target.id,
                    "text": text,
                });

                // Reply in thread if specified
                if let Some(ref reply_to) = msg.reply_to {
                    body["thread_ts"] = serde_json::json!(reply_to.id);
                }

                // Check meta for thread_ts
                if let Some(ts) = msg.meta.get("thread_ts").and_then(|v| v.as_str()) {
                    body["thread_ts"] = serde_json::json!(ts);
                }

                let resp = self.api_call("chat.postMessage", Some(&body))?;
                let ts = resp.get("ts")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();

                Ok(SendReceipt {
                    message: MessageRef { provider: "slack".into(), id: ts },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }

    fn send_reaction(&mut self, target: &MessageRef, emoji: &str) -> Result<(), ChatError> {
        // target.provider contains the channel_id (set in parse)
        let body = serde_json::json!({
            "channel": target.provider,
            "timestamp": target.id,
            "name": emoji.trim_matches(':'),
        });
        self.api_call("reactions.add", Some(&body))?;
        Ok(())
    }
}

impl SlackProvider {
    fn parse_slack_message(&self, event: &serde_json::Value) -> Option<InboundEvent> {
        let user_id = event.get("user").and_then(|v| v.as_str())?;

        // Skip our own messages
        if Some(user_id) == self.bot_user_id.as_deref() {
            return None;
        }

        let channel = event.get("channel").and_then(|v| v.as_str())?;
        let ts = event.get("ts").and_then(|v| v.as_str())?;
        let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");

        // Filter by allowed channels
        if !self.allowed_channels.is_empty() && !self.allowed_channels.contains(&channel.to_string()) {
            return None;
        }

        if text.is_empty() {
            return None;
        }

        // Determine kind (heuristic: channels start with C, DMs with D, groups with G)
        let kind = if channel.starts_with('D') {
            ConversationKind::Direct
        } else if channel.starts_with('G') {
            ConversationKind::Group
        } else {
            ConversationKind::Channel
        };

        // Thread support
        let thread_ts = event.get("thread_ts").and_then(|v| v.as_str());
        let reply_to = thread_ts.map(|tts| MessageRef {
            provider: channel.to_string(), // Store channel for reaction API
            id: tts.to_string(),
        });

        // Convert Slack ts to milliseconds
        let timestamp_ms = ts.parse::<f64>()
            .map(|f| (f * 1000.0) as i64)
            .unwrap_or_else(|_| chrono::Utc::now().timestamp_millis());

        // Check if bot was mentioned
        let mentions_ai = self.bot_user_id.as_ref()
            .map(|id| text.contains(&format!("<@{id}>")))
            .unwrap_or(false);

        Some(InboundEvent::Message(InboundMessage {
            event_id: ts.to_string(),
            conversation: ConversationRef {
                provider: "slack".into(),
                kind,
                id: channel.to_string(),
                parent_id: thread_ts.map(|s| s.to_string()),
                title: None,
            },
            message: MessageRef {
                provider: channel.to_string(), // Store channel for reaction API
                id: ts.to_string(),
            },
            sender: ActorRef {
                id: user_id.to_string(),
                display_name: user_id.to_string(), // Would need users.info for real name
                is_bot: false,
            },
            timestamp_ms,
            content: MessageContent::Text { text: text.to_string() },
            reply_to,
            mentions_ai,
            raw: Some(event.to_string()),
        }))
    }
}
