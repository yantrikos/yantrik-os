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

        let config = crate::config::WhatsAppConfig {
            enabled: true,
            phone_number_id: Some(self.phone_number_id.clone()),
            access_token: Some(self.access_token.clone()),
            recipient: Some(to.to_string()),
        };

        match crate::whatsapp::send_message(&config, to, text) {
            Ok(()) => format!("WhatsApp message sent to {to}."),
            Err(e) => format!("Failed to send WhatsApp message: {e}"),
        }
    }
}
