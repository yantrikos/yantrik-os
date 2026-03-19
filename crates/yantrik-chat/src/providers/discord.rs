//! Discord provider — Gateway WebSocket + REST API.
//!
//! Uses tungstenite for the blocking WebSocket gateway connection.
//! No public ingress needed — all inbound via persistent WebSocket.
//! Outbound via REST API using ureq.

use std::io::Read as IoRead;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use tungstenite::{connect, Message as WsMessage, WebSocket};
use tungstenite::stream::MaybeTlsStream;

use crate::model::*;
use crate::provider::*;

const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const API_BASE: &str = "https://discord.com/api/v10";

// Gateway opcodes
const OP_DISPATCH: u64 = 0;
const OP_HEARTBEAT: u64 = 1;
const OP_IDENTIFY: u64 = 2;
const OP_HELLO: u64 = 10;
const OP_HEARTBEAT_ACK: u64 = 11;

/// Discord bot intents we need.
const INTENTS: u64 = (1 << 0)  // GUILDS
    | (1 << 9)  // GUILD_MESSAGES
    | (1 << 12) // DIRECT_MESSAGES
    | (1 << 15); // MESSAGE_CONTENT

pub struct DiscordProvider {
    bot_token: String,
    /// Only process messages from these guild IDs. Empty = all guilds.
    allowed_guilds: Vec<String>,
    health: ProviderHealth,
    ws: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    session_id: Option<String>,
    sequence: Option<u64>,
    heartbeat_interval_ms: u64,
    last_heartbeat: Instant,
    last_ack: bool,
    bot_user_id: Option<String>,
}

impl DiscordProvider {
    pub fn new(bot_token: String, allowed_guilds: Vec<String>) -> Self {
        Self {
            bot_token,
            allowed_guilds,
            health: ProviderHealth::Disconnected,
            ws: None,
            session_id: None,
            sequence: None,
            heartbeat_interval_ms: 45000,
            last_heartbeat: Instant::now(),
            last_ack: true,
            bot_user_id: None,
        }
    }

    fn send_ws(&mut self, payload: &serde_json::Value) -> Result<(), ChatError> {
        if let Some(ws) = &mut self.ws {
            ws.send(WsMessage::Text(payload.to_string()))
                .map_err(|e| ChatError::Network(format!("WS send: {e}")))?;
        }
        Ok(())
    }

    fn send_heartbeat(&mut self) -> Result<(), ChatError> {
        let payload = serde_json::json!({
            "op": OP_HEARTBEAT,
            "d": self.sequence,
        });
        self.send_ws(&payload)?;
        self.last_heartbeat = Instant::now();
        self.last_ack = false;
        Ok(())
    }

    fn send_identify(&mut self) -> Result<(), ChatError> {
        let payload = serde_json::json!({
            "op": OP_IDENTIFY,
            "d": {
                "token": self.bot_token,
                "intents": INTENTS,
                "properties": {
                    "os": "linux",
                    "browser": "yantrik",
                    "device": "yantrik",
                },
            },
        });
        self.send_ws(&payload)
    }

    /// REST API call using ureq.
    fn api_call(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, ChatError> {
        let url = format!("{}{}", API_BASE, path);
        let auth = format!("Bot {}", self.bot_token);

        let resp = match method {
            "GET" => {
                ureq::get(&url)
                    .set("Authorization", &auth)
                    .call()
            }
            "POST" => {
                let req = ureq::post(&url)
                    .set("Authorization", &auth)
                    .set("Content-Type", "application/json");
                if let Some(b) = body {
                    req.send_string(&b.to_string())
                } else {
                    req.send_string("{}")
                }
            }
            _ => return Err(ChatError::Internal(format!("unsupported method: {method}"))),
        };

        match resp {
            Ok(r) => {
                let text = r.into_string()
                    .map_err(|e| ChatError::Network(format!("read body: {e}")))?;
                serde_json::from_str(&text)
                    .map_err(|e| ChatError::Provider(format!("invalid JSON: {e}")))
            }
            Err(ureq::Error::Status(429, r)) => {
                let retry = r.into_string().ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v.get("retry_after")?.as_f64())
                    .map(|f| f.ceil() as u64)
                    .unwrap_or(5);
                Err(ChatError::RateLimited(retry))
            }
            Err(ureq::Error::Status(code, _)) => {
                Err(ChatError::Provider(format!("HTTP {code}")))
            }
            Err(e) => Err(ChatError::Network(format!("{e}"))),
        }
    }

    /// Read one message from the WebSocket (non-blocking-ish with timeout).
    fn read_ws_message(&mut self) -> Result<Option<serde_json::Value>, ChatError> {
        let ws = match &mut self.ws {
            Some(ws) => ws,
            None => return Err(ChatError::Network("not connected".into())),
        };

        // Set read timeout so we don't block forever
        if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        }

        match ws.read() {
            Ok(WsMessage::Text(text)) => {
                match serde_json::from_str(&text) {
                    Ok(v) => Ok(Some(v)),
                    Err(_) => Ok(None),
                }
            }
            Ok(WsMessage::Close(_)) => {
                Err(ChatError::Network("WebSocket closed by server".into()))
            }
            Ok(WsMessage::Ping(data)) => {
                let _ = ws.send(WsMessage::Pong(data));
                Ok(None)
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
}

impl ChatProvider for DiscordProvider {
    fn id(&self) -> &'static str { "discord" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::WebSocket,
            media: true,
            voice: false,
            reactions: true,
            typing: true,
            threads: true,
            groups: true,
            channels: true,
            read_receipts: false,
            edits: true,
            deletes: true,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        let (ws, _response) = connect(GATEWAY_URL)
            .map_err(|e| ChatError::Network(format!("WS connect: {e}")))?;
        self.ws = Some(ws);

        // Read HELLO to get heartbeat interval
        if let Some(hello) = self.read_ws_message()? {
            if hello.get("op").and_then(|v| v.as_u64()) == Some(OP_HELLO) {
                self.heartbeat_interval_ms = hello
                    .get("d")
                    .and_then(|d| d.get("heartbeat_interval"))
                    .and_then(|h| h.as_u64())
                    .unwrap_or(45000);
            }
        }

        // Send IDENTIFY
        self.send_identify()?;

        // Wait for READY event
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(msg) = self.read_ws_message()? {
                if let Some(seq) = msg.get("s").and_then(|v| v.as_u64()) {
                    self.sequence = Some(seq);
                }
                let op = msg.get("op").and_then(|v| v.as_u64()).unwrap_or(0);
                if op == OP_DISPATCH {
                    let event_name = msg.get("t").and_then(|v| v.as_str()).unwrap_or("");
                    if event_name == "READY" {
                        self.session_id = msg.get("d")
                            .and_then(|d| d.get("session_id"))
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        self.bot_user_id = msg.get("d")
                            .and_then(|d| d.get("user"))
                            .and_then(|u| u.get("id"))
                            .and_then(|i| i.as_str())
                            .map(|s| s.to_string());
                        tracing::info!(
                            session = ?self.session_id,
                            bot_id = ?self.bot_user_id,
                            "Discord gateway READY"
                        );
                        self.health = ProviderHealth::Connected;
                        self.last_heartbeat = Instant::now();
                        self.last_ack = true;
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        Err(ChatError::Network("Timed out waiting for READY".into()))
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
        // Send heartbeat if needed
        let elapsed = self.last_heartbeat.elapsed().as_millis() as u64;
        if elapsed >= self.heartbeat_interval_ms {
            if !self.last_ack {
                return Err(ChatError::Network("heartbeat ACK timeout — zombied connection".into()));
            }
            self.send_heartbeat()?;
        }

        let mut events = Vec::new();

        // Read up to 50 messages per poll cycle to avoid blocking too long
        for _ in 0..50 {
            let msg = match self.read_ws_message() {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(e) => return Err(e),
            };

            // Track sequence
            if let Some(seq) = msg.get("s").and_then(|v| v.as_u64()) {
                self.sequence = Some(seq);
            }

            let op = msg.get("op").and_then(|v| v.as_u64()).unwrap_or(0);

            match op {
                OP_HEARTBEAT => {
                    // Server requests immediate heartbeat
                    self.send_heartbeat()?;
                }
                OP_HEARTBEAT_ACK => {
                    self.last_ack = true;
                }
                OP_DISPATCH => {
                    let event_name = msg.get("t").and_then(|v| v.as_str()).unwrap_or("");
                    let data = msg.get("d").cloned().unwrap_or(serde_json::Value::Null);

                    match event_name {
                        "MESSAGE_CREATE" => {
                            if let Some(event) = self.parse_message_create(&data) {
                                events.push(event);
                            }
                        }
                        "MESSAGE_UPDATE" => {
                            if let Some(event) = self.parse_message_update(&data) {
                                events.push(event);
                            }
                        }
                        _ => {} // Ignore other events for now
                    }
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
        let channel_id = &target.id;

        match &msg.content {
            OutboundContent::Text(text) => {
                // Discord limit is 2000 chars
                let text = if text.len() > 2000 {
                    &text[..text.floor_char_boundary(2000)]
                } else {
                    text.as_str()
                };

                let mut body = serde_json::json!({ "content": text });

                if let Some(ref reply_to) = msg.reply_to {
                    body["message_reference"] = serde_json::json!({
                        "message_id": reply_to.id,
                    });
                }

                let resp = self.api_call("POST", &format!("/channels/{channel_id}/messages"), Some(&body))?;
                let msg_id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("0").to_string();

                Ok(SendReceipt {
                    message: MessageRef { provider: "discord".into(), id: msg_id },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            OutboundContent::Reaction { target, emoji } => {
                // URL-encode emoji for the API path
                let encoded = urlencoding_simple(emoji);
                self.api_call(
                    "PUT_NO_BODY",
                    &format!("/channels/{}/messages/{}/reactions/{}/@me", target.provider, target.id, encoded),
                    None,
                ).ok();
                Ok(SendReceipt {
                    message: target.clone(),
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }

    fn send_typing(&mut self, target: &ConversationRef) -> Result<(), ChatError> {
        self.api_call("POST", &format!("/channels/{}/typing", target.id), None)?;
        Ok(())
    }

    fn send_reaction(&mut self, target: &MessageRef, emoji: &str) -> Result<(), ChatError> {
        let encoded = urlencoding_simple(emoji);
        // We need the channel_id but MessageRef doesn't have it.
        // For now, use the provider field which we set to channel_id in parse.
        let channel_id = &target.provider; // overloaded — see parse_message_create
        self.api_call(
            "PUT_NO_BODY",
            &format!("/channels/{}/messages/{}/reactions/{}/@me", channel_id, target.id, encoded),
            None,
        ).ok();
        Ok(())
    }
}

impl DiscordProvider {
    fn parse_message_create(&self, data: &serde_json::Value) -> Option<InboundEvent> {
        let author = data.get("author")?;
        let author_id = author.get("id").and_then(|v| v.as_str())?;

        // Skip messages from ourselves
        if Some(author_id) == self.bot_user_id.as_deref() {
            return None;
        }

        let is_bot = author.get("bot").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_bot {
            return None;
        }

        let channel_id = data.get("channel_id").and_then(|v| v.as_str())?;
        let message_id = data.get("id").and_then(|v| v.as_str())?;
        let guild_id = data.get("guild_id").and_then(|v| v.as_str());

        // Filter by allowed guilds
        if !self.allowed_guilds.is_empty() {
            if let Some(gid) = guild_id {
                if !self.allowed_guilds.iter().any(|g| g == gid) {
                    return None;
                }
            }
        }

        let username = author.get("username").and_then(|v| v.as_str()).unwrap_or("Unknown");
        let display = author.get("global_name")
            .and_then(|v| v.as_str())
            .unwrap_or(username);

        let kind = if guild_id.is_some() {
            ConversationKind::Channel
        } else {
            ConversationKind::Direct
        };

        // Parse content
        let text = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let content = if text.is_empty() {
            // Check for attachments
            let attachments = data.get("attachments").and_then(|v| v.as_array());
            if let Some(atts) = attachments {
                if let Some(att) = atts.first() {
                    let ct = att.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
                    let file_id = att.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if ct.starts_with("image/") {
                        MessageContent::Image {
                            media: MediaRef {
                                provider: "discord".into(),
                                id: file_id.to_string(),
                                mime_type: Some(ct.to_string()),
                                size_bytes: att.get("size").and_then(|v| v.as_u64()),
                                filename: att.get("filename").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            },
                            caption: None,
                        }
                    } else {
                        MessageContent::File {
                            media: MediaRef {
                                provider: "discord".into(),
                                id: file_id.to_string(),
                                mime_type: Some(ct.to_string()),
                                size_bytes: att.get("size").and_then(|v| v.as_u64()),
                                filename: att.get("filename").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            },
                            caption: None,
                        }
                    }
                } else {
                    return None;
                }
            } else {
                return None; // No text, no attachments
            }
        } else {
            MessageContent::Text { text: text.to_string() }
        };

        // Check if bot was mentioned
        let mentions_ai = data.get("mentions")
            .and_then(|v| v.as_array())
            .map(|mentions| {
                mentions.iter().any(|m| {
                    m.get("id").and_then(|i| i.as_str()) == self.bot_user_id.as_deref()
                })
            })
            .unwrap_or(false);

        let reply_to = data.get("referenced_message")
            .and_then(|r| r.get("id"))
            .and_then(|i| i.as_str())
            .map(|id| MessageRef { provider: channel_id.to_string(), id: id.to_string() });

        let timestamp = data.get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

        Some(InboundEvent::Message(InboundMessage {
            event_id: message_id.to_string(),
            conversation: ConversationRef {
                provider: "discord".into(),
                kind,
                id: channel_id.to_string(),
                parent_id: guild_id.map(|s| s.to_string()),
                title: None,
            },
            message: MessageRef {
                provider: channel_id.to_string(), // Store channel_id for reaction API
                id: message_id.to_string(),
            },
            sender: ActorRef {
                id: author_id.to_string(),
                display_name: display.to_string(),
                is_bot: false,
            },
            timestamp_ms: timestamp,
            content,
            reply_to,
            mentions_ai,
            raw: Some(data.to_string()),
        }))
    }

    fn parse_message_update(&self, data: &serde_json::Value) -> Option<InboundEvent> {
        let channel_id = data.get("channel_id").and_then(|v| v.as_str())?;
        let message_id = data.get("id").and_then(|v| v.as_str())?;
        let text = data.get("content").and_then(|v| v.as_str())?;

        let kind = if data.get("guild_id").is_some() {
            ConversationKind::Channel
        } else {
            ConversationKind::Direct
        };

        Some(InboundEvent::MessageEdited(MessageEditEvent {
            event_id: format!("edit_{message_id}"),
            conversation: ConversationRef {
                provider: "discord".into(),
                kind,
                id: channel_id.to_string(),
                parent_id: None,
                title: None,
            },
            message: MessageRef { provider: "discord".into(), id: message_id.to_string() },
            new_content: MessageContent::Text { text: text.to_string() },
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        }))
    }
}

/// Simple URL encoding for emoji characters.
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
