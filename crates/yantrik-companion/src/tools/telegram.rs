//! Telegram tool — lets the LLM send messages to the user's Telegram.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

/// Register the telegram_send tool (only if Telegram is enabled).
pub fn register(reg: &mut ToolRegistry, bot_token: &str, chat_id: &str) {
    reg.register(Box::new(TelegramSendTool {
        bot_token: bot_token.to_string(),
        chat_id: chat_id.to_string(),
    }));
}

struct TelegramSendTool {
    bot_token: String,
    chat_id: String,
}

impl Tool for TelegramSendTool {
    fn name(&self) -> &'static str { "telegram_send" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "telegram" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "telegram_send",
                "description": "Send a Telegram message",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "Message text to send (max 4096 characters)."
                        }
                    },
                    "required": ["text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        if text.is_empty() {
            return "Error: text is required".to_string();
        }

        let config = crate::config::TelegramConfig {
            enabled: true,
            bot_token: Some(self.bot_token.clone()),
            chat_id: Some(self.chat_id.clone()),
            poll_interval_secs: 3,
            forward_proactive: true,
        };

        match crate::telegram::send_message(&config, text) {
            Ok(()) => "Message sent to Telegram.".to_string(),
            Err(e) => format!("Failed to send Telegram message: {e}"),
        }
    }
}
