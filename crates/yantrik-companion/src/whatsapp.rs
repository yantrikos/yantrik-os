//! WhatsApp Business API — sync, ureq-based messaging.
//!
//! Provides `send_message` and `send_template` for the WhatsApp Cloud API.
//! Requires a Meta Business account with WhatsApp API access.
//!
//! Config:
//! ```yaml
//! whatsapp:
//!   enabled: true
//!   phone_number_id: "123456789"
//!   access_token: "EAAx..."
//!   recipient: "+1234567890"
//! ```

use crate::config::WhatsAppConfig;

/// Send a text message via WhatsApp Cloud API.
pub fn send_message(config: &WhatsAppConfig, to: &str, text: &str) -> Result<(), String> {
    let phone_number_id = config.phone_number_id.as_deref()
        .ok_or("No phone_number_id configured")?;
    let access_token = config.access_token.as_deref()
        .ok_or("No access_token configured")?;

    let url = format!(
        "https://graph.facebook.com/v21.0/{}/messages",
        phone_number_id
    );

    // Truncate to WhatsApp's ~4096 char limit
    let text = if text.len() > 4096 {
        &text[..text.floor_char_boundary(4096)]
    } else {
        text
    };

    let body = serde_json::json!({
        "messaging_product": "whatsapp",
        "to": to,
        "type": "text",
        "text": {
            "body": text
        }
    });

    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", access_token))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("WhatsApp API error: {e}"))?;

    let status = response.status();
    if status >= 200 && status < 300 {
        Ok(())
    } else {
        let body = response.into_string().unwrap_or_default();
        Err(format!("WhatsApp API returned {status}: {body}"))
    }
}

/// Send a template message (for initiating conversations outside 24h window).
pub fn send_template(
    config: &WhatsAppConfig,
    to: &str,
    template_name: &str,
    language: &str,
) -> Result<(), String> {
    let phone_number_id = config.phone_number_id.as_deref()
        .ok_or("No phone_number_id configured")?;
    let access_token = config.access_token.as_deref()
        .ok_or("No access_token configured")?;

    let url = format!(
        "https://graph.facebook.com/v21.0/{}/messages",
        phone_number_id
    );

    let body = serde_json::json!({
        "messaging_product": "whatsapp",
        "to": to,
        "type": "template",
        "template": {
            "name": template_name,
            "language": {
                "code": language
            }
        }
    });

    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", access_token))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("WhatsApp API error: {e}"))?;

    let status = response.status();
    if status >= 200 && status < 300 {
        Ok(())
    } else {
        let body = response.into_string().unwrap_or_default();
        Err(format!("WhatsApp API returned {status}: {body}"))
    }
}
