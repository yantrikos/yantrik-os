//! Telegram Bot API — sync, curl-based (no extra deps).
//!
//! Provides `send_message` and `get_updates` for bidirectional chat.
//! Uses `std::process::Command::new("curl")` — same pattern as home_assistant.rs.

use crate::config::TelegramConfig;

/// Send a message to the configured Telegram chat.
/// Uses HTML parse_mode for basic formatting.
pub fn send_message(config: &TelegramConfig, text: &str) -> Result<(), String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;
    let chat_id = config.chat_id.as_deref().ok_or("No chat_id configured")?;

    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    // Truncate to Telegram's 4096 char limit
    let text = if text.len() > 4096 { &text[..4096] } else { text };

    // Escape HTML entities so LLM output with <, >, & doesn't break formatting
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": escaped,
        "parse_mode": "HTML",
    });

    let output = std::process::Command::new("curl")
        .arg("-s")
        .arg("-X").arg("POST")
        .arg("-H").arg("Content-Type: application/json")
        .arg("--connect-timeout").arg("5")
        .arg("--max-time").arg("10")
        .arg("-d").arg(body.to_string())
        .arg(&url)
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check for Telegram API errors
    if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(());
        }
        let desc = resp.get("description").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(format!("Telegram API error: {desc}"));
    }

    if !output.status.success() {
        return Err(format!("curl returned status {}", output.status));
    }

    Ok(())
}

/// Represents a received Telegram message.
pub struct TelegramUpdate {
    pub update_id: i64,
    pub text: String,
    pub chat_id: String,
}

/// Long-poll for updates from Telegram.
/// Returns new messages (only from the configured chat_id for security).
/// `offset` should be last_update_id + 1 to acknowledge previous updates.
pub fn get_updates(
    config: &TelegramConfig,
    offset: i64,
    timeout: u64,
) -> Result<Vec<TelegramUpdate>, String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;
    let allowed_chat_id = config.chat_id.as_deref().ok_or("No chat_id configured")?;

    let url = format!(
        "https://api.telegram.org/bot{}/getUpdates?offset={}&timeout={}",
        token, offset, timeout,
    );

    let output = std::process::Command::new("curl")
        .arg("-s")
        .arg("--connect-timeout").arg("5")
        .arg("--max-time").arg(format!("{}", timeout + 5))
        .arg(&url)
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Empty response — curl timeout or network glitch. Return no updates.
    if stdout.trim().is_empty() {
        return Ok(vec![]);
    }

    let resp: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("Invalid JSON from Telegram: {e}"))?;

    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let desc = resp.get("description").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(format!("Telegram API error: {desc}"));
    }

    let updates = resp.get("result").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut result = Vec::new();

    for update in &updates {
        let update_id = update.get("update_id").and_then(|v| v.as_i64()).unwrap_or(0);
        let message = match update.get("message") {
            Some(m) => m,
            None => continue,
        };

        let chat = match message.get("chat") {
            Some(c) => c,
            None => continue,
        };

        let msg_chat_id = chat.get("id").and_then(|v| v.as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default();

        // Security: only accept messages from the configured chat
        if msg_chat_id != allowed_chat_id {
            tracing::warn!(
                msg_chat_id,
                allowed = allowed_chat_id,
                "Ignoring Telegram message from unauthorized chat"
            );
            continue;
        }

        let text = message.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        if text.is_empty() {
            continue;
        }

        result.push(TelegramUpdate {
            update_id,
            text: text.to_string(),
            chat_id: msg_chat_id,
        });
    }

    Ok(result)
}
