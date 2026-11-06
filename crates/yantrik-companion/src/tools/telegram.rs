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

        // A message to a third party's service, in the person's name: asked about first, whole
        // (#1054). It goes to the one chat the bot is set up for.
        let chat = format!("the person's Telegram chat ({})", self.chat_id);
        yantrik_companion_tools::outward::acting(
            &yantrik_companion_tools::outward::OutwardAct { tool: "telegram_send", to: &chat, what: text, more: "" },
            || {
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
            },
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    //! `telegram_send` messages a third party's service in the person's name (#1054): it asks
    //! first, with the whole text, and a no sends nothing.
    use super::*;
    use yantrik_companion_tools::outward_testing::{actions, first_card, installed, shown, with_ctx};

    fn tool() -> TelegramSendTool {
        TelegramSendTool { bot_token: "fake-token-not-a-secret".into(), chat_id: "4242".into() }
    }

    #[test]
    fn a_message_raises_a_card_with_the_whole_text_and_a_no_sends_nothing() {
        let (_shell, heard) = installed("tg-no", "pending", "denied");
        let said = with_ctx(|ctx| tool().execute(ctx, &serde_json::json!({ "text": "On my way; to: nobody" })));

        assert!(said.contains("said no") && said.contains("Nothing was done"), "{said}");
        assert!(!said.contains("Message sent") && !said.contains("Failed to send"), "a send was attempted: {said}");
        let card = first_card(&heard);
        assert_eq!(card["args_json"]["tool"], "telegram_send");
        assert!(shown(&card, "to").contains("4242"));
        assert_eq!(shown(&card, "what"), "On my way; to: nobody");
        assert!(!actions(&heard).contains(&"consume_approval".to_string()));
    }

    #[test]
    fn silence_sends_nothing_either() {
        let (_shell, _heard) = installed("tg-silent", "pending", "pending");
        let said = with_ctx(|ctx| tool().execute(ctx, &serde_json::json!({ "text": "hi" })));
        assert!(said.contains("Nobody answered") && !said.contains("Message sent"), "{said}");
    }
}
