//! Telegram provider — bidirectional via Bot API long-polling.
//!
//! Uses `curl` for HTTP (same pattern as existing telegram.rs in companion).
//! Implements: poll (long-poll), send (text, voice), typing, reactions, media download.

use crate::model::*;
use crate::provider::*;

/// Telegram Bot API provider.
pub struct TelegramProvider {
    bot_token: String,
    chat_id: String,
    last_update_id: i64,
    health: ProviderHealth,
    /// Long-poll timeout in seconds.
    poll_timeout: u64,
}

impl TelegramProvider {
    pub fn new(bot_token: String, chat_id: String) -> Self {
        Self {
            bot_token,
            chat_id,
            last_update_id: 0,
            health: ProviderHealth::Disconnected,
            poll_timeout: 30,
        }
    }

    /// Execute a Telegram Bot API method via curl.
    fn api_call(&self, method: &str, body: &serde_json::Value) -> Result<serde_json::Value, ChatError> {
        let url = format!("https://api.telegram.org/bot{}/{}", self.bot_token, method);

        let output = std::process::Command::new("curl")
            .arg("-s")
            .arg("-X").arg("POST")
            .arg("-H").arg("Content-Type: application/json")
            .arg("--connect-timeout").arg("5")
            .arg("--max-time").arg(if method == "getUpdates" {
                format!("{}", self.poll_timeout + 5)
            } else {
                "10".into()
            })
            .arg("-d").arg(body.to_string())
            .arg(&url)
            .output()
            .map_err(|e| ChatError::Network(format!("curl failed: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Err(ChatError::Network("empty response from Telegram".into()));
        }

        let resp: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| ChatError::Provider(format!("invalid JSON: {e}")))?;

        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let desc = resp.get("description").and_then(|v| v.as_str()).unwrap_or("unknown");
            let code = resp.get("error_code").and_then(|v| v.as_u64()).unwrap_or(0);
            if code == 429 {
                let retry = resp.get("parameters")
                    .and_then(|p| p.get("retry_after"))
                    .and_then(|r| r.as_u64())
                    .unwrap_or(5);
                return Err(ChatError::RateLimited(retry));
            }
            return Err(ChatError::Provider(format!("Telegram API: {desc}")));
        }

        Ok(resp)
    }

    /// Upload and send a voice message (OGG/Opus).
    fn send_voice_file(&self, ogg_path: &str) -> Result<SendReceipt, ChatError> {
        let url = format!("https://api.telegram.org/bot{}/sendVoice", self.bot_token);

        let output = std::process::Command::new("curl")
            .arg("-s")
            .arg("--connect-timeout").arg("5")
            .arg("--max-time").arg("30")
            .arg("-F").arg(format!("chat_id={}", self.chat_id))
            .arg("-F").arg(format!("voice=@{}", ogg_path))
            .arg(&url)
            .output()
            .map_err(|e| ChatError::Network(format!("curl sendVoice: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let resp: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| ChatError::Provider(format!("invalid JSON: {e}")))?;

        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let desc = resp.get("description").and_then(|v| v.as_str()).unwrap_or("unknown");
            return Err(ChatError::Provider(format!("sendVoice: {desc}")));
        }

        let msg_id = resp.get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|m| m.as_i64())
            .unwrap_or(0);

        Ok(SendReceipt {
            message: MessageRef {
                provider: "telegram".into(),
                id: msg_id.to_string(),
            },
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        })
    }
}

impl ChatProvider for TelegramProvider {
    fn id(&self) -> &'static str { "telegram" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            ingress: IngressMode::Polling,
            media: true,
            voice: true,
            reactions: true,
            typing: true,
            threads: false,
            groups: true,
            channels: true,
            read_receipts: false,
            edits: false,
            deletes: false,
        }
    }

    fn connect(&mut self) -> Result<(), ChatError> {
        // Verify token by calling getMe
        let resp = self.api_call("getMe", &serde_json::json!({}))?;
        let bot_name = resp.get("result")
            .and_then(|r| r.get("username"))
            .and_then(|u| u.as_str())
            .unwrap_or("unknown");
        tracing::info!(bot = bot_name, "Telegram bot connected");
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
        let body = serde_json::json!({
            "offset": self.last_update_id + 1,
            "timeout": self.poll_timeout,
            "allowed_updates": ["message", "edited_message", "message_reaction"],
        });

        let resp = self.api_call("getUpdates", &body)?;

        let updates = resp.get("result")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut events = Vec::new();

        for update in &updates {
            let update_id = update.get("update_id").and_then(|v| v.as_i64()).unwrap_or(0);
            if update_id > self.last_update_id {
                self.last_update_id = update_id;
            }

            // Handle message
            if let Some(message) = update.get("message") {
                if let Some(event) = parse_telegram_message(message, &self.chat_id, update_id) {
                    events.push(event);
                }
            }

            // Handle edited message
            if let Some(edited) = update.get("edited_message") {
                if let Some(event) = parse_telegram_edit(edited, &self.chat_id, update_id) {
                    events.push(event);
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
        let chat_id = &target.id;

        match &msg.content {
            OutboundContent::Text(text) => {
                // Truncate to 4096 chars
                let text = if text.len() > 4096 {
                    &text[..text.floor_char_boundary(4096)]
                } else {
                    text.as_str()
                };

                // Escape HTML
                let escaped = text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");

                let mut body = serde_json::json!({
                    "chat_id": chat_id,
                    "text": escaped,
                    "parse_mode": "HTML",
                });

                // Reply to specific message if requested
                if let Some(ref reply_to) = msg.reply_to {
                    body["reply_to_message_id"] = serde_json::json!(
                        reply_to.id.parse::<i64>().unwrap_or(0)
                    );
                }

                let resp = self.api_call("sendMessage", &body)?;
                let msg_id = resp.get("result")
                    .and_then(|r| r.get("message_id"))
                    .and_then(|m| m.as_i64())
                    .unwrap_or(0);

                Ok(SendReceipt {
                    message: MessageRef {
                        provider: "telegram".into(),
                        id: msg_id.to_string(),
                    },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            OutboundContent::Voice { path } => {
                self.send_voice_file(path)
            }
            OutboundContent::Image { path, caption } => {
                let url = format!("https://api.telegram.org/bot{}/sendPhoto", self.bot_token);
                let mut cmd = std::process::Command::new("curl");
                cmd.arg("-s")
                    .arg("--connect-timeout").arg("5")
                    .arg("--max-time").arg("30")
                    .arg("-F").arg(format!("chat_id={}", chat_id))
                    .arg("-F").arg(format!("photo=@{}", path));
                if let Some(cap) = caption {
                    cmd.arg("-F").arg(format!("caption={}", cap));
                }
                cmd.arg(&url);

                let output = cmd.output()
                    .map_err(|e| ChatError::Network(format!("curl sendPhoto: {e}")))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let resp: serde_json::Value = serde_json::from_str(&stdout)
                    .map_err(|e| ChatError::Provider(format!("invalid JSON: {e}")))?;
                let msg_id = resp.get("result")
                    .and_then(|r| r.get("message_id"))
                    .and_then(|m| m.as_i64())
                    .unwrap_or(0);

                Ok(SendReceipt {
                    message: MessageRef { provider: "telegram".into(), id: msg_id.to_string() },
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            OutboundContent::Reaction { target, emoji } => {
                self.send_reaction(target, emoji)?;
                Ok(SendReceipt {
                    message: target.clone(),
                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                })
            }
            _ => Err(ChatError::Unsupported),
        }
    }

    fn send_typing(&mut self, target: &ConversationRef) -> Result<(), ChatError> {
        let body = serde_json::json!({
            "chat_id": target.id,
            "action": "typing",
        });
        self.api_call("sendChatAction", &body)?;
        Ok(())
    }

    fn send_reaction(&mut self, target: &MessageRef, emoji: &str) -> Result<(), ChatError> {
        let body = serde_json::json!({
            "chat_id": self.chat_id,
            "message_id": target.id.parse::<i64>().unwrap_or(0),
            "reaction": [{"type": "emoji", "emoji": emoji}],
        });
        self.api_call("setMessageReaction", &body)?;
        Ok(())
    }

    fn download_media(&mut self, media: &MediaRef) -> Result<Vec<u8>, ChatError> {
        // Step 1: getFile to get file_path
        let body = serde_json::json!({ "file_id": media.id });
        let resp = self.api_call("getFile", &body)?;

        let file_path = resp.get("result")
            .and_then(|r| r.get("file_path"))
            .and_then(|p| p.as_str())
            .ok_or_else(|| ChatError::Provider("no file_path in response".into()))?;

        // Step 2: Download the file
        let url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.bot_token, file_path
        );

        let output = std::process::Command::new("curl")
            .arg("-s")
            .arg("--connect-timeout").arg("5")
            .arg("--max-time").arg("30")
            .arg(&url)
            .output()
            .map_err(|e| ChatError::Network(format!("curl download: {e}")))?;

        if !output.status.success() {
            return Err(ChatError::Network(format!("download failed: {}", output.status)));
        }

        Ok(output.stdout)
    }
}

/// Parse a Telegram message object into an InboundEvent.
fn parse_telegram_message(
    message: &serde_json::Value,
    allowed_chat_id: &str,
    update_id: i64,
) -> Option<InboundEvent> {
    let chat = message.get("chat")?;
    let chat_id = chat.get("id").and_then(|v| v.as_i64())?.to_string();

    // Security: only accept from configured chat
    if chat_id != allowed_chat_id {
        tracing::warn!(chat_id, allowed = allowed_chat_id, "Ignoring unauthorized Telegram chat");
        return None;
    }

    let from = message.get("from")?;
    let sender_id = from.get("id").and_then(|v| v.as_i64())?.to_string();
    let first_name = from.get("first_name").and_then(|v| v.as_str()).unwrap_or("Unknown");
    let last_name = from.get("last_name").and_then(|v| v.as_str()).unwrap_or("");
    let display_name = if last_name.is_empty() {
        first_name.to_string()
    } else {
        format!("{first_name} {last_name}")
    };
    let is_bot = from.get("is_bot").and_then(|v| v.as_bool()).unwrap_or(false);

    let message_id = message.get("message_id").and_then(|v| v.as_i64())?;
    let timestamp = message.get("date").and_then(|v| v.as_i64()).unwrap_or(0) * 1000;

    // Determine conversation kind from chat type
    let chat_type = chat.get("type").and_then(|v| v.as_str()).unwrap_or("private");
    let kind = match chat_type {
        "private" => ConversationKind::Direct,
        "group" | "supergroup" => ConversationKind::Group,
        "channel" => ConversationKind::Channel,
        _ => ConversationKind::Direct,
    };
    let title = chat.get("title").and_then(|v| v.as_str()).map(|s| s.to_string());

    // Parse content
    let content = if let Some(voice) = message.get("voice") {
        let file_id = voice.get("file_id").and_then(|v| v.as_str())?;
        let duration = voice.get("duration").and_then(|v| v.as_u64()).unwrap_or(0);
        MessageContent::Voice {
            media: MediaRef {
                provider: "telegram".into(),
                id: file_id.to_string(),
                mime_type: Some("audio/ogg".into()),
                size_bytes: voice.get("file_size").and_then(|v| v.as_u64()),
                filename: None,
            },
            duration_ms: Some(duration * 1000),
        }
    } else if let Some(photo) = message.get("photo").and_then(|v| v.as_array()) {
        // Take the largest photo
        let largest = photo.last()?;
        let file_id = largest.get("file_id").and_then(|v| v.as_str())?;
        let caption = message.get("caption").and_then(|v| v.as_str()).map(|s| s.to_string());
        MessageContent::Image {
            media: MediaRef {
                provider: "telegram".into(),
                id: file_id.to_string(),
                mime_type: Some("image/jpeg".into()),
                size_bytes: largest.get("file_size").and_then(|v| v.as_u64()),
                filename: None,
            },
            caption,
        }
    } else if let Some(doc) = message.get("document") {
        let file_id = doc.get("file_id").and_then(|v| v.as_str())?;
        let caption = message.get("caption").and_then(|v| v.as_str()).map(|s| s.to_string());
        MessageContent::File {
            media: MediaRef {
                provider: "telegram".into(),
                id: file_id.to_string(),
                mime_type: doc.get("mime_type").and_then(|v| v.as_str()).map(|s| s.to_string()),
                size_bytes: doc.get("file_size").and_then(|v| v.as_u64()),
                filename: doc.get("file_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
            },
            caption,
        }
    } else if let Some(loc) = message.get("location") {
        let lat = loc.get("latitude").and_then(|v| v.as_f64())?;
        let lon = loc.get("longitude").and_then(|v| v.as_f64())?;
        MessageContent::Location { lat, lon, label: None }
    } else if let Some(text) = message.get("text").and_then(|v| v.as_str()) {
        if text.is_empty() {
            return None;
        }
        MessageContent::Text { text: text.to_string() }
    } else if let Some(sticker) = message.get("sticker") {
        let file_id = sticker.get("file_id").and_then(|v| v.as_str())?;
        MessageContent::Sticker {
            media: MediaRef {
                provider: "telegram".into(),
                id: file_id.to_string(),
                mime_type: Some("image/webp".into()),
                size_bytes: sticker.get("file_size").and_then(|v| v.as_u64()),
                filename: None,
            },
        }
    } else {
        return None; // Skip unsupported message types
    };

    // Check if bot was mentioned (in group chats)
    let mentions_ai = message.get("entities")
        .and_then(|v| v.as_array())
        .map(|entities| {
            entities.iter().any(|e| {
                e.get("type").and_then(|t| t.as_str()) == Some("mention") ||
                e.get("type").and_then(|t| t.as_str()) == Some("bot_command")
            })
        })
        .unwrap_or(false);

    // Check reply_to
    let reply_to = message.get("reply_to_message")
        .and_then(|r| r.get("message_id"))
        .and_then(|m| m.as_i64())
        .map(|id| MessageRef {
            provider: "telegram".into(),
            id: id.to_string(),
        });

    Some(InboundEvent::Message(InboundMessage {
        event_id: update_id.to_string(),
        conversation: ConversationRef {
            provider: "telegram".into(),
            kind,
            id: chat_id,
            parent_id: None,
            title,
        },
        message: MessageRef {
            provider: "telegram".into(),
            id: message_id.to_string(),
        },
        sender: ActorRef {
            id: sender_id,
            display_name,
            is_bot,
        },
        timestamp_ms: timestamp,
        content,
        reply_to,
        mentions_ai,
        raw: Some(message.to_string()),
    }))
}

/// Parse an edited message into a MessageEdited event.
fn parse_telegram_edit(
    edited: &serde_json::Value,
    allowed_chat_id: &str,
    update_id: i64,
) -> Option<InboundEvent> {
    let chat = edited.get("chat")?;
    let chat_id = chat.get("id").and_then(|v| v.as_i64())?.to_string();
    if chat_id != allowed_chat_id {
        return None;
    }

    let message_id = edited.get("message_id").and_then(|v| v.as_i64())?;
    let text = edited.get("text").and_then(|v| v.as_str())?;
    let timestamp = edited.get("edit_date").and_then(|v| v.as_i64()).unwrap_or(0) * 1000;

    let chat_type = chat.get("type").and_then(|v| v.as_str()).unwrap_or("private");
    let kind = match chat_type {
        "private" => ConversationKind::Direct,
        "group" | "supergroup" => ConversationKind::Group,
        "channel" => ConversationKind::Channel,
        _ => ConversationKind::Direct,
    };

    Some(InboundEvent::MessageEdited(MessageEditEvent {
        event_id: format!("edit_{update_id}"),
        conversation: ConversationRef {
            provider: "telegram".into(),
            kind,
            id: chat_id,
            parent_id: None,
            title: None,
        },
        message: MessageRef {
            provider: "telegram".into(),
            id: message_id.to_string(),
        },
        new_content: MessageContent::Text { text: text.to_string() },
        timestamp_ms: timestamp,
    }))
}
