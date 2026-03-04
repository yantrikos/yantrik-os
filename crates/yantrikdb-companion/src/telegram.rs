//! Telegram Bot API — sync, curl-based (no extra deps).
//!
//! Provides `send_message`, `get_updates`, and voice message support for
//! bidirectional chat including Jarvis-style voice interactions.
//! Uses `std::process::Command::new("curl")` — same pattern as home_assistant.rs.

use crate::config::TelegramConfig;

/// Voice message metadata from a Telegram update.
pub struct TelegramVoice {
    pub file_id: String,
    pub duration: u64,
}

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

/// Represents a received Telegram message (text and/or voice).
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message_id: i64,
    pub text: String,
    pub chat_id: String,
    pub voice: Option<TelegramVoice>,
}

/// Send a "typing..." indicator to the chat.
pub fn send_typing(config: &TelegramConfig) -> Result<(), String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;
    let chat_id = config.chat_id.as_deref().ok_or("No chat_id configured")?;

    let url = format!("https://api.telegram.org/bot{}/sendChatAction", token);
    let body = serde_json::json!({
        "chat_id": chat_id,
        "action": "typing",
    });

    std::process::Command::new("curl")
        .arg("-s")
        .arg("-X").arg("POST")
        .arg("-H").arg("Content-Type: application/json")
        .arg("--connect-timeout").arg("3")
        .arg("--max-time").arg("5")
        .arg("-d").arg(body.to_string())
        .arg(&url)
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    Ok(())
}

/// React to a message with an emoji (e.g. eyes when reading).
pub fn set_reaction(config: &TelegramConfig, message_id: i64, emoji: &str) -> Result<(), String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;
    let chat_id = config.chat_id.as_deref().ok_or("No chat_id configured")?;

    let url = format!("https://api.telegram.org/bot{}/setMessageReaction", token);
    let body = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "reaction": [{"type": "emoji", "emoji": emoji}],
    });

    std::process::Command::new("curl")
        .arg("-s")
        .arg("-X").arg("POST")
        .arg("-H").arg("Content-Type: application/json")
        .arg("--connect-timeout").arg("3")
        .arg("--max-time").arg("5")
        .arg("-d").arg(body.to_string())
        .arg(&url)
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    Ok(())
}

/// Remove a reaction from a message (clear the eyes emoji after responding).
pub fn clear_reaction(config: &TelegramConfig, message_id: i64) -> Result<(), String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;
    let chat_id = config.chat_id.as_deref().ok_or("No chat_id configured")?;

    let url = format!("https://api.telegram.org/bot{}/setMessageReaction", token);
    let body = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "reaction": [],
    });

    std::process::Command::new("curl")
        .arg("-s")
        .arg("-X").arg("POST")
        .arg("-H").arg("Content-Type: application/json")
        .arg("--connect-timeout").arg("3")
        .arg("--max-time").arg("5")
        .arg("-d").arg(body.to_string())
        .arg(&url)
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    Ok(())
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

        // Parse voice message if present
        let voice = message.get("voice").and_then(|v| {
            let file_id = v.get("file_id").and_then(|f| f.as_str())?;
            let duration = v.get("duration").and_then(|d| d.as_u64()).unwrap_or(0);
            Some(TelegramVoice {
                file_id: file_id.to_string(),
                duration,
            })
        });

        // Skip if neither text nor voice
        if text.is_empty() && voice.is_none() {
            continue;
        }

        let message_id = message.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0);

        result.push(TelegramUpdate {
            update_id,
            message_id,
            text: text.to_string(),
            chat_id: msg_chat_id,
            voice,
        });
    }

    Ok(result)
}

/// Get the file path for a Telegram file_id (needed before downloading).
pub fn get_file(config: &TelegramConfig, file_id: &str) -> Result<String, String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;

    let url = format!("https://api.telegram.org/bot{}/getFile", token);
    let body = serde_json::json!({ "file_id": file_id });

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
    let resp: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("Invalid JSON: {e}"))?;

    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let desc = resp.get("description").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(format!("getFile error: {desc}"));
    }

    resp.get("result")
        .and_then(|r| r.get("file_path"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No file_path in getFile response".into())
}

/// Download a file from Telegram servers to a local path.
pub fn download_file(config: &TelegramConfig, file_path: &str, local_path: &str) -> Result<(), String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;

    let url = format!("https://api.telegram.org/file/bot{}/{}", token, file_path);

    let output = std::process::Command::new("curl")
        .arg("-s")
        .arg("--connect-timeout").arg("5")
        .arg("--max-time").arg("30")
        .arg("-o").arg(local_path)
        .arg(&url)
        .output()
        .map_err(|e| format!("curl download failed: {e}"))?;

    if !output.status.success() {
        return Err(format!("curl download returned status {}", output.status));
    }

    // Verify the file exists and has content
    let meta = std::fs::metadata(local_path)
        .map_err(|e| format!("Downloaded file not found: {e}"))?;
    if meta.len() == 0 {
        return Err("Downloaded file is empty".into());
    }

    Ok(())
}

/// Send a voice message (OGG/Opus file) to the configured chat.
pub fn send_voice(config: &TelegramConfig, ogg_path: &str) -> Result<(), String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;
    let chat_id = config.chat_id.as_deref().ok_or("No chat_id configured")?;

    let url = format!("https://api.telegram.org/bot{}/sendVoice", token);

    let output = std::process::Command::new("curl")
        .arg("-s")
        .arg("--connect-timeout").arg("5")
        .arg("--max-time").arg("30")
        .arg("-F").arg(format!("chat_id={}", chat_id))
        .arg("-F").arg(format!("voice=@{}", ogg_path))
        .arg(&url)
        .output()
        .map_err(|e| format!("curl sendVoice failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(());
        }
        let desc = resp.get("description").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(format!("sendVoice error: {desc}"));
    }

    if !output.status.success() {
        return Err(format!("curl returned status {}", output.status));
    }

    Ok(())
}

/// Send "record_voice" chat action (shows recording indicator in chat).
pub fn send_recording_voice(config: &TelegramConfig) -> Result<(), String> {
    let token = config.bot_token.as_deref().ok_or("No bot_token configured")?;
    let chat_id = config.chat_id.as_deref().ok_or("No chat_id configured")?;

    let url = format!("https://api.telegram.org/bot{}/sendChatAction", token);
    let body = serde_json::json!({
        "chat_id": chat_id,
        "action": "record_voice",
    });

    std::process::Command::new("curl")
        .arg("-s")
        .arg("-X").arg("POST")
        .arg("-H").arg("Content-Type: application/json")
        .arg("--connect-timeout").arg("3")
        .arg("--max-time").arg("5")
        .arg("-d").arg(body.to_string())
        .arg(&url)
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    Ok(())
}
