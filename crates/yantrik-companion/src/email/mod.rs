//! Email service — IMAP fetch, SMTP send, SQLite cache.
//!
//! Handles OAuth2 token refresh transparently before IMAP/SMTP calls.

pub mod imap_client;
pub mod smtp_client;
pub mod db;

use crate::config::EmailAccountConfig;

const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

fn google_client_id() -> String {
    std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default()
}

fn google_client_secret() -> String {
    std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default()
}

/// Ensure the OAuth2 access token is fresh. Refreshes in-place if expired.
///
/// If the token is expired or will expire within 5 minutes, refreshes it
/// and optionally updates the config file on disk.
pub fn ensure_fresh_token(account: &mut EmailAccountConfig, config_path: Option<&str>) -> Result<(), String> {
    if account.auth_method.as_deref() != Some("oauth2") {
        return Ok(());
    }

    let expiry = account.oauth_token_expiry.unwrap_or(0.0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    // Token still fresh — skip refresh
    if now < expiry - 300.0 {
        return Ok(());
    }

    let refresh_token = account.oauth_refresh_token.as_deref().unwrap_or("");
    if refresh_token.is_empty() {
        return Err("OAuth2 token expired and no refresh token available".to_string());
    }

    tracing::info!("Refreshing OAuth2 access token for {}", account.email);

    let body = format!(
        "refresh_token={}&client_id={}&client_secret={}&grant_type=refresh_token",
        urlencoding(refresh_token),
        urlencoding(&google_client_id()),
        urlencoding(&google_client_secret()),
    );

    let resp: serde_json::Value = ureq::post(GOOGLE_TOKEN_URL)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
        .map_err(|e| format!("Token refresh request failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Token refresh parse failed: {}", e))?;

    let new_token = resp["access_token"].as_str()
        .ok_or("No access_token in refresh response")?
        .to_string();
    let expires_in = resp["expires_in"].as_u64().unwrap_or(3600);

    account.oauth_access_token = Some(new_token);
    account.oauth_token_expiry = Some(now + expires_in as f64);

    // Persist refreshed token to config file
    if let Some(path) = config_path {
        if let Err(e) = update_config_token(
            path,
            &account.email,
            account.oauth_access_token.as_deref().unwrap_or(""),
            account.oauth_token_expiry.unwrap_or(0.0),
        ) {
            tracing::warn!("Failed to persist refreshed token: {}", e);
        }
    }

    tracing::info!("OAuth2 token refreshed for {}, expires in {}s", account.email, expires_in);
    Ok(())
}

/// Update the OAuth token in config.yaml for a specific email account.
fn update_config_token(config_path: &str, email: &str, new_token: &str, new_expiry: f64) -> Result<(), String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Read config: {}", e))?;

    let mut in_target_account = false;
    let mut result = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect target account by email field
        if trimmed.contains(email) && (trimmed.starts_with("email:") || trimmed.starts_with("- email:")) {
            in_target_account = true;
        } else if in_target_account && !trimmed.is_empty() && !trimmed.starts_with('#')
            && !trimmed.starts_with('-') && !line.starts_with(' ') && !line.starts_with('\t')
        {
            // Left the indented account block
            in_target_account = false;
        }

        if in_target_account && trimmed.starts_with("oauth_access_token:") {
            let indent = &line[..line.len() - line.trim_start().len()];
            result.push(format!("{}oauth_access_token: \"{}\"", indent, new_token));
        } else if in_target_account && trimmed.starts_with("oauth_token_expiry:") {
            let indent = &line[..line.len() - line.trim_start().len()];
            result.push(format!("{}oauth_token_expiry: {}", indent, new_expiry));
        } else {
            result.push(line.to_string());
        }
    }

    std::fs::write(config_path, result.join("\n"))
        .map_err(|e| format!("Write config: {}", e))?;
    Ok(())
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                String::from(b as char)
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

/// Resolve IMAP server from provider name.
pub fn resolve_imap_server(account: &EmailAccountConfig) -> (String, u16) {
    if let Some(ref server) = account.imap_server {
        return (server.clone(), account.imap_port);
    }
    match account.provider.as_str() {
        "gmail" => ("imap.gmail.com".to_string(), 993),
        "outlook" => ("outlook.office365.com".to_string(), 993),
        "yahoo" => ("imap.mail.yahoo.com".to_string(), 993),
        _ => ("".to_string(), account.imap_port),
    }
}

/// Resolve SMTP server from provider name.
pub fn resolve_smtp_server(account: &EmailAccountConfig) -> (String, u16) {
    if let Some(ref server) = account.smtp_server {
        return (server.clone(), account.smtp_port);
    }
    match account.provider.as_str() {
        "gmail" => ("smtp.gmail.com".to_string(), 587),
        "outlook" => ("smtp.office365.com".to_string(), 587),
        "yahoo" => ("smtp.mail.yahoo.com".to_string(), 587),
        _ => ("".to_string(), account.smtp_port),
    }
}
