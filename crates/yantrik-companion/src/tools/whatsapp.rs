//! WhatsApp tools — send messages via WhatsApp Business API.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry, phone_number_id: &str, access_token: &str, default_recipient: &str) {
    reg.register(Box::new(WhatsAppSendTool {
        phone_number_id: phone_number_id.to_string(),
        access_token: access_token.to_string(),
        default_recipient: default_recipient.to_string(),
    }));
}

struct WhatsAppSendTool {
    phone_number_id: String,
    access_token: String,
    default_recipient: String,
}

impl Tool for WhatsAppSendTool {
    fn name(&self) -> &'static str { "whatsapp_send" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "whatsapp" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "whatsapp_send",
                "description": "Send a WhatsApp message",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "Message text to send (max 4096 characters)."
                        },
                        "to": {
                            "type": "string",
                            "description": "Phone number in international format (e.g. +1234567890). Omit to send to the default recipient."
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

        let to = args.get("to")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_recipient);

        if to.is_empty() {
            return "Error: no recipient specified and no default recipient configured".to_string();
        }

        // A message to a phone number, in the person's name: asked about first, whole (#1054).
        yantrik_companion_tools::outward::acting(
            &yantrik_companion_tools::outward::OutwardAct { tool: "whatsapp_send", to, what: text, more: "" },
            || {
                let config = crate::config::WhatsAppConfig {
                    enabled: true,
                    phone_number_id: Some(self.phone_number_id.clone()),
                    access_token: Some(self.access_token.clone()),
                    recipient: Some(to.to_string()),
                    verify_token: None,
                };
                match crate::whatsapp::send_message(&config, to, text) {
                    Ok(()) => format!("WhatsApp message sent to {to}."),
                    Err(e) => format!("Failed to send WhatsApp message: {e}"),
                }
            },
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    //! `whatsapp_send` messages a phone number in the person's name (#1054): it asks first, with
    //! the number and the whole text, and a no sends nothing.
    use super::*;
    use yantrik_companion_tools::outward_testing::{actions, first_card, installed, shown, with_ctx};

    fn tool() -> WhatsAppSendTool {
        WhatsAppSendTool {
            phone_number_id: "1".into(),
            access_token: "fake-token-not-a-secret".into(),
            default_recipient: "+15550100".into(),
        }
    }

    #[test]
    fn a_message_raises_a_card_with_the_number_and_the_whole_text_and_a_no_sends_nothing() {
        let (_shell, heard) = installed("wa-no", "pending", "denied");
        let said = with_ctx(|ctx| tool().execute(ctx, &serde_json::json!({ "text": "Pay me; now", "to": "+15550123" })));

        assert!(said.contains("said no") && said.contains("Nothing was done"), "{said}");
        assert!(!said.contains("WhatsApp message sent") && !said.contains("Failed to send"), "a send was attempted: {said}");
        let card = first_card(&heard);
        assert_eq!(card["args_json"]["tool"], "whatsapp_send");
        assert_eq!(shown(&card, "to"), "+15550123");
        assert_eq!(shown(&card, "what"), "Pay me; now");
        assert!(!actions(&heard).contains(&"consume_approval".to_string()));
    }

    #[test]
    fn the_default_recipient_is_on_the_card_when_none_is_named() {
        let (_shell, heard) = installed("wa-default", "pending", "denied");
        with_ctx(|ctx| tool().execute(ctx, &serde_json::json!({ "text": "hi" })));
        assert_eq!(shown(&first_card(&heard), "to"), "+15550100");
    }
}
