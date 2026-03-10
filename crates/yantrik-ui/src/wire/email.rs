//! Email client wire module — IMAP fetch, SMTP send, OAuth2, AI integration.
//!
//! Account setup saves to config.yaml.
//! Supports both App Password and OAuth2 (Google) authentication.
//! After save, IMAP fetch populates inbox.
//! AI summarize/draft/enhance via companion bridge.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, EmailAttachmentData, EmailDetailData, EmailFolderData, EmailListItem, EmailSignatureData, EmailThreadMessage};

// ── Google OAuth2 Constants ──
const GOOGLE_CLIENT_ID: &str = "REDACTED_GOOGLE_CLIENT_ID";
const GOOGLE_CLIENT_SECRET: &str = "REDACTED_GOOGLE_CLIENT_SECRET";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_SCOPES: &str = "https://mail.google.com/ https://www.googleapis.com/auth/calendar email profile";

/// Resolved email server settings.
struct EmailServer {
    provider: String,
    imap_server: String,
    imap_port: u16,
    smtp_server: String,
    smtp_port: u16,
}

/// Known provider server settings.
fn provider_servers(provider: &str) -> Option<EmailServer> {
    match provider {
        "gmail" => Some(EmailServer {
            provider: "gmail".into(),
            imap_server: "imap.gmail.com".into(),
            imap_port: 993,
            smtp_server: "smtp.gmail.com".into(),
            smtp_port: 587,
        }),
        "outlook" => Some(EmailServer {
            provider: "outlook".into(),
            imap_server: "outlook.office365.com".into(),
            imap_port: 993,
            smtp_server: "smtp.office365.com".into(),
            smtp_port: 587,
        }),
        "yahoo" => Some(EmailServer {
            provider: "yahoo".into(),
            imap_server: "imap.mail.yahoo.com".into(),
            imap_port: 993,
            smtp_server: "smtp.mail.yahoo.com".into(),
            smtp_port: 587,
        }),
        "icloud" => Some(EmailServer {
            provider: "icloud".into(),
            imap_server: "imap.mail.me.com".into(),
            imap_port: 993,
            smtp_server: "smtp.mail.me.com".into(),
            smtp_port: 587,
        }),
        _ => None,
    }
}

/// Resolve server from provider + manual fields + domain fallback.
fn resolve_server(
    email: &str,
    provider: &str,
    manual_imap: &str,
    manual_imap_port: &str,
    manual_smtp: &str,
    manual_smtp_port: &str,
) -> EmailServer {
    if let Some(srv) = provider_servers(provider) {
        return srv;
    }
    if provider == "advanced" && !manual_imap.is_empty() {
        let imap_port = manual_imap_port.parse().unwrap_or(993);
        let smtp_port = manual_smtp_port.parse().unwrap_or(587);
        let smtp = if manual_smtp.is_empty() {
            manual_imap.replace("imap", "smtp")
        } else {
            manual_smtp.to_string()
        };
        return EmailServer {
            provider: "imap".into(),
            imap_server: manual_imap.to_string(),
            imap_port,
            smtp_server: smtp,
            smtp_port,
        };
    }
    let domain = email.rsplit('@').next().unwrap_or("");
    EmailServer {
        provider: "imap".into(),
        imap_server: format!("imap.{}", domain),
        imap_port: 993,
        smtp_server: format!("smtp.{}", domain),
        smtp_port: 587,
    }
}

/// Cached account credentials (read from config).
#[derive(Clone, Default)]
struct AccountInfo {
    email: String,
    password: String,
    display_name: String,
    provider: String,
    imap_server: String,
    imap_port: u16,
    smtp_server: String,
    smtp_port: u16,
    // OAuth2 fields
    auth_method: String,         // "password" or "oauth2"
    oauth_access_token: String,
    oauth_refresh_token: String,
    oauth_token_expiry: f64,     // unix timestamp
}

impl AccountInfo {
    fn is_oauth(&self) -> bool {
        self.auth_method == "oauth2"
    }

    /// Get a valid access token, refreshing if expired. Returns (token, was_refreshed).
    fn get_valid_token(&self) -> Result<(String, bool), String> {
        if !self.is_oauth() {
            return Err("Not an OAuth account".to_string());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        // Refresh if expired or within 60 seconds of expiry
        if now >= self.oauth_token_expiry - 60.0 {
            if self.oauth_refresh_token.is_empty() {
                return Err("No refresh token — need to re-authenticate".to_string());
            }
            let tokens = oauth2_refresh_token(&self.oauth_refresh_token)?;
            Ok((tokens.access_token, true))
        } else {
            Ok((self.oauth_access_token.clone(), false))
        }
    }
}

/// Read account info from config file.
fn read_account_from_config(ctx: &AppContext) -> Option<AccountInfo> {
    let path = ctx.config_path.as_ref()?;
    let content = std::fs::read_to_string(path).ok()?;
    if !content.contains("email:") || !content.contains("accounts:") {
        return None;
    }

    let mut info = AccountInfo::default();
    let mut in_accounts = false;
    for line in content.lines() {
        let t = line.trim();
        // Strip leading YAML list marker "- " for cleaner matching
        let field = if t.starts_with("- ") { &t[2..] } else { t };

        if t == "accounts:" {
            in_accounts = true;
        } else if in_accounts && !t.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') && !t.starts_with('#') && t != "accounts:" {
            // Left the accounts block (new top-level key at column 0)
            in_accounts = false;
        } else if in_accounts {
            if field.starts_with("email:") {
                info.email = field.trim_start_matches("email:").trim().trim_matches('"').to_string();
            } else if field.starts_with("name:") {
                info.display_name = field.trim_start_matches("name:").trim().trim_matches('"').to_string();
            } else if field.starts_with("password:") {
                info.password = field.trim_start_matches("password:").trim().trim_matches('"').to_string();
            } else if field.starts_with("provider:") {
                info.provider = field.trim_start_matches("provider:").trim().trim_matches('"').to_string();
            } else if field.starts_with("imap_server:") {
                info.imap_server = field.trim_start_matches("imap_server:").trim().trim_matches('"').to_string();
            } else if field.starts_with("imap_port:") {
                info.imap_port = field.trim_start_matches("imap_port:").trim().parse().unwrap_or(993);
            } else if field.starts_with("smtp_server:") {
                info.smtp_server = field.trim_start_matches("smtp_server:").trim().trim_matches('"').to_string();
            } else if field.starts_with("smtp_port:") {
                info.smtp_port = field.trim_start_matches("smtp_port:").trim().parse().unwrap_or(587);
            } else if field.starts_with("auth_method:") {
                info.auth_method = field.trim_start_matches("auth_method:").trim().trim_matches('"').to_string();
            } else if field.starts_with("oauth_access_token:") {
                info.oauth_access_token = field.trim_start_matches("oauth_access_token:").trim().trim_matches('"').to_string();
            } else if field.starts_with("oauth_refresh_token:") {
                info.oauth_refresh_token = field.trim_start_matches("oauth_refresh_token:").trim().trim_matches('"').to_string();
            } else if field.starts_with("oauth_token_expiry:") {
                info.oauth_token_expiry = field.trim_start_matches("oauth_token_expiry:").trim().parse().unwrap_or(0.0);
            }
        }
    }

    if info.email.is_empty() {
        return None;
    }
    // OAuth accounts don't need a password
    if info.auth_method != "oauth2" && info.password.is_empty() {
        return None;
    }
    // Resolve server if not explicit
    if info.imap_server.is_empty() {
        let srv = resolve_server(&info.email, &info.provider, "", "", "", "");
        info.imap_server = srv.imap_server;
        info.imap_port = srv.imap_port;
        info.smtp_server = srv.smtp_server;
        info.smtp_port = srv.smtp_port;
    }
    Some(info)
}

/// Fetched email header for UI.
#[derive(Clone)]
struct FetchedEmail {
    id: i32,
    from_name: String,
    from_addr: String,
    to_addr: String,
    subject: String,
    date_text: String,
    preview: String,
    body: String,
    is_read: bool,
}

/// IMAP fetch emails from a folder (blocking).
/// Supports both password and OAuth2 (XOAUTH2) authentication.
fn imap_fetch_emails(
    server: &str,
    port: u16,
    email: &str,
    password: &str,
    folder: &str,
    max_count: u32,
    oauth_token: Option<&str>,
) -> Result<Vec<FetchedEmail>, String> {
    use std::net::TcpStream;

    let addr = format!("{}:{}", server, port);
    let sock_addr = std::net::ToSocketAddrs::to_socket_addrs(&addr.as_str())
        .map_err(|e| format!("DNS failed for {}: {}", addr, e))?
        .next()
        .ok_or_else(|| format!("No addresses for {}", addr))?;

    let tcp = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(15))
        .map_err(|e| format!("Connect failed: {}", e))?;

    let tls = native_tls::TlsConnector::new().map_err(|e| format!("TLS: {}", e))?;
    let tls_stream = tls.connect(server, tcp).map_err(|e| format!("TLS: {}", e))?;

    let client = imap::Client::new(tls_stream);

    let mut session = if let Some(token) = oauth_token {
        // XOAUTH2 authentication
        let auth = XOAuth2 { user: email.to_string(), token: token.to_string() };
        client.authenticate("XOAUTH2", &auth).map_err(|e| {
            let err = format!("{}", e.0);
            if err.contains("AUTHENTICATIONFAILED") || err.contains("Invalid credentials") {
                format!("OAuth token rejected — try signing in again")
            } else {
                format!("XOAUTH2 login: {}", err)
            }
        })?
    } else {
        // Password authentication
        client.login(email, password).map_err(|e| {
            let err = format!("{}", e.0);
            if err.contains("AUTHENTICATIONFAILED") || err.contains("Invalid credentials") || err.contains("authentication failed") {
                format!("Auth failed — use an App Password, not your regular password")
            } else {
                format!("Login: {}", err)
            }
        })?
    };

    let mailbox = session.select(folder).map_err(|e| format!("Select {}: {}", folder, e))?;
    let total = mailbox.exists;
    if total == 0 {
        let _ = session.logout();
        return Ok(Vec::new());
    }

    // Fetch last N emails
    let start = if total > max_count { total - max_count + 1 } else { 1 };
    let range = format!("{}:{}", start, total);
    let fetch = session.fetch(&range, "(ENVELOPE BODY.PEEK[] FLAGS)")
        .map_err(|e| format!("Fetch: {}", e))?;

    let mut emails = Vec::new();
    for (idx, msg) in fetch.iter().enumerate() {
        let envelope = match msg.envelope() {
            Some(e) => e,
            None => continue,
        };

        let subject = envelope.subject
            .as_ref()
            .map(|s| decode_header_value(s))
            .unwrap_or_default();

        let (from_name, from_addr) = envelope.from.as_ref()
            .and_then(|addrs| addrs.first())
            .map(|a| {
                let name = a.name.as_ref().map(|n| decode_header_value(n)).unwrap_or_default();
                let mailbox = a.mailbox.as_ref().map(|m| String::from_utf8_lossy(m).to_string()).unwrap_or_default();
                let host = a.host.as_ref().map(|h| String::from_utf8_lossy(h).to_string()).unwrap_or_default();
                let addr = if !mailbox.is_empty() && !host.is_empty() {
                    format!("{}@{}", mailbox, host)
                } else {
                    mailbox
                };
                (name, addr)
            })
            .unwrap_or_default();

        let to_addr = envelope.to.as_ref()
            .and_then(|addrs| addrs.first())
            .map(|a| {
                let mailbox = a.mailbox.as_ref().map(|m| String::from_utf8_lossy(m).to_string()).unwrap_or_default();
                let host = a.host.as_ref().map(|h| String::from_utf8_lossy(h).to_string()).unwrap_or_default();
                if !mailbox.is_empty() && !host.is_empty() { format!("{}@{}", mailbox, host) } else { mailbox }
            })
            .unwrap_or_default();

        let date_text = envelope.date
            .as_ref()
            .map(|d| {
                let s = String::from_utf8_lossy(d).to_string();
                // Simplify: take first 16 chars or so
                if s.len() > 22 { truncate_utf8(&s, 22) } else { s }
            })
            .unwrap_or_default();

        let body = msg.body()
            .map(|b| extract_body_text(b))
            .unwrap_or_default();
        // Preview: first meaningful non-empty line, truncated
        let preview = body.lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && l.len() > 3)
            .unwrap_or("")
            .to_string();
        let preview = if preview.len() > 100 {
            format!("{}...", truncate_utf8(&preview, 100))
        } else {
            preview
        };

        let flags = msg.flags();
        let is_read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));

        emails.push(FetchedEmail {
            id: (start as i32) + idx as i32,
            from_name,
            from_addr,
            to_addr,
            subject,
            date_text,
            preview,
            body,
            is_read,
        });
    }

    let _ = session.logout();
    // Show newest first
    emails.reverse();
    Ok(emails)
}

/// Decode IMAP header value (handles UTF-8 and RFC 2047 encoded words).
fn decode_header_value(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes).to_string();
    // Basic RFC 2047 decoding for =?UTF-8?B?...?= and =?UTF-8?Q?...?=
    if s.contains("=?") && s.contains("?=") {
        if let Some(decoded) = try_decode_rfc2047(&s) {
            return decoded;
        }
    }
    s
}

fn try_decode_rfc2047(s: &str) -> Option<String> {
    use std::str;
    let mut result = String::new();
    let mut remaining = s;
    while let Some(start) = remaining.find("=?") {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + 2..];
        let q1 = after.find('?')?;
        let charset = &after[..q1];
        let after = &after[q1 + 1..];
        let q2 = after.find('?')?;
        let encoding = &after[..q2];
        let after = &after[q2 + 1..];
        let end = after.find("?=")?;
        let encoded = &after[..end];
        remaining = &after[end + 2..];

        let decoded_bytes = match encoding.to_uppercase().as_str() {
            "B" => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(encoded).ok()?
            }
            "Q" => {
                let mut bytes = Vec::new();
                let mut chars = encoded.bytes();
                while let Some(b) = chars.next() {
                    if b == b'=' {
                        let h1 = chars.next()?;
                        let h2 = chars.next()?;
                        let hex = format!("{}{}", h1 as char, h2 as char);
                        bytes.push(u8::from_str_radix(&hex, 16).ok()?);
                    } else if b == b'_' {
                        bytes.push(b' ');
                    } else {
                        bytes.push(b);
                    }
                }
                bytes
            }
            _ => return None,
        };

        let text = if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("utf8") {
            String::from_utf8_lossy(&decoded_bytes).to_string()
        } else {
            String::from_utf8_lossy(&decoded_bytes).to_string()
        };
        result.push_str(&text);
    }
    result.push_str(remaining);
    Some(result)
}

/// Basic HTML tag stripper.
/// Truncate a string to at most `max_bytes` bytes at a valid UTF-8 char boundary.
fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Convert HTML to clean readable plain text using html2text with email-optimized config.
///
/// Uses raw_mode (flattens table-based layouts), no_table_borders, and cleans up
/// excessive whitespace that marketing emails tend to produce.
fn html_to_clean_text(html: &str) -> String {
    let config = html2text::config::plain()
        .raw_mode(true)         // Flatten table layouts (emails use tables for formatting)
        .no_table_borders()     // No ASCII table borders
        .allow_width_overflow(); // Don't error on wide content

    let text = match config.string_from_read(html.as_bytes(), 100) {
        Ok(t) => t,
        Err(_) => {
            // Fallback to basic from_read
            html2text::from_read(html.as_bytes(), 100)
        }
    };

    // Clean up excessive blank lines (marketing emails produce many)
    let mut result = String::with_capacity(text.len());
    let mut blank_count = 0u32;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            // Skip lines that are just decoration (sequences of -, =, _, *)
            if trimmed.len() > 3 && trimmed.chars().all(|c| c == '-' || c == '=' || c == '_' || c == '*' || c == ' ') {
                continue;
            }
            // Skip lines that are just "[image]" or similar placeholders
            if trimmed == "[image]" || trimmed == "[ ]" {
                continue;
            }
            result.push_str(trimmed);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

/// Extract readable plain text from a MIME email body.
///
/// Prefers text/plain part. Falls back to converting text/html via html2text.
fn extract_body_text(raw: &[u8]) -> String {
    if let Ok(parsed) = mailparse::parse_mail(raw) {
        let (plain, html) = extract_from_parsed(&parsed);
        if let Some(p) = plain {
            return p;
        }
        if let Some(h) = html {
            return html_to_clean_text(&h);
        }

        // mailparse parsed but didn't find text/plain or text/html via our recursive extract.
        // Try get_body() on the top-level message directly — handles single-part messages
        // where the content-type might not match our expected patterns.
        if let Ok(body) = parsed.get_body() {
            let body = body.trim().to_string();
            if !body.is_empty() {
                if body.contains("<!") || body.contains("<html") || body.contains("<div") || body.contains("<p>") {
                    return html_to_clean_text(&body);
                }
                return body;
            }
        }
    }

    // Fallback: raw bytes — decode QP if present, then process
    let s = String::from_utf8_lossy(raw).to_string();

    // Detect and decode quoted-printable manually if mailparse failed
    let decoded = if s.contains("=3D") || s.contains("=\r\n") || s.contains("=\n") {
        decode_qp_basic(&s)
    } else {
        s
    };

    if decoded.contains('<') && (decoded.contains("</") || decoded.contains("/>")) {
        html_to_clean_text(&decoded)
    } else {
        strip_html_basic(&decoded)
    }
}

/// Basic quoted-printable decoder for fallback when mailparse fails.
fn decode_qp_basic(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '=' {
            // Soft line break: =\r\n or =\n
            if chars.peek() == Some(&'\r') {
                chars.next();
                if chars.peek() == Some(&'\n') { chars.next(); }
                continue;
            }
            if chars.peek() == Some(&'\n') {
                chars.next();
                continue;
            }
            // Hex encoded byte: =XX
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(a), Some(b)) = (h1, h2) {
                let hex = format!("{}{}", a, b);
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
                // Not valid hex — keep as-is
                result.push('=');
                result.push(a);
                result.push(b);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Recursively extract text/plain and text/html from a parsed MIME message.
/// Returns (Option<plain_text>, Option<html_body>).
fn extract_from_parsed(mail: &mailparse::ParsedMail) -> (Option<String>, Option<String>) {
    let ctype = mail.ctype.mimetype.to_lowercase();

    if mail.subparts.is_empty() {
        let body = match mail.get_body() {
            Ok(b) => b,
            Err(_) => return (None, None),
        };
        if ctype == "text/plain" {
            let trimmed = body.trim().to_string();
            if !trimmed.is_empty() {
                return (Some(trimmed), None);
            }
        } else if ctype == "text/html" {
            let trimmed = body.trim().to_string();
            if !trimmed.is_empty() {
                return (None, Some(trimmed));
            }
        }
        return (None, None);
    }

    // Multipart — collect both text/plain and text/html
    let mut plain = None;
    let mut html = None;
    for part in &mail.subparts {
        let sub_ctype = part.ctype.mimetype.to_lowercase();
        if sub_ctype.starts_with("multipart/") {
            let (p, h) = extract_from_parsed(part);
            if plain.is_none() { plain = p; }
            if html.is_none() { html = h; }
        } else if sub_ctype == "text/plain" {
            if let Ok(body) = part.get_body() {
                let trimmed = body.trim().to_string();
                if !trimmed.is_empty() && plain.is_none() {
                    plain = Some(trimmed);
                }
            }
        } else if sub_ctype == "text/html" {
            if let Ok(body) = part.get_body() {
                let trimmed = body.trim().to_string();
                if !trimmed.is_empty() && html.is_none() {
                    html = Some(trimmed);
                }
            }
        }
    }

    (plain, html)
}

/// Basic HTML tag stripper (fallback when MIME parsing fails).
fn strip_html_basic(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    // Collapse whitespace
    let mut collapsed = String::new();
    let mut last_space = false;
    for ch in result.chars() {
        if ch.is_whitespace() {
            if !last_space {
                collapsed.push(' ');
                last_space = true;
            }
        } else {
            collapsed.push(ch);
            last_space = false;
        }
    }
    collapsed.trim().to_string()
}

// ── OAuth2 Flow ──

/// OAuth2 token response from Google.
#[derive(Clone, Default)]
struct OAuthTokens {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

/// Start a localhost HTTP server, open Google OAuth URL, wait for redirect with auth code.
/// Returns (authorization_code, port_used).
/// Also sends (auth_url, port) on the provided channel so the UI can display it.
fn oauth2_authorize_google(
    url_tx: Option<std::sync::mpsc::Sender<(String, u16)>>,
) -> Result<(String, u16), String> {
    use std::io::{Read as IoRead, Write as IoWrite};
    use std::net::TcpListener;

    // Bind first (port 0 = OS picks a free port), then open browser
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Cannot bind listener: {}", e))?;
    let port = listener.local_addr()
        .map_err(|e| format!("Local addr: {}", e))?.port();

    let redirect_uri = format!("http://localhost:{}", port);
    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        GOOGLE_AUTH_URL,
        urlencoding(GOOGLE_CLIENT_ID),
        urlencoding(&redirect_uri),
        urlencoding(GOOGLE_SCOPES),
    );

    // Send URL to UI so it can be shown to user
    if let Some(tx) = url_tx {
        let _ = tx.send((auth_url.clone(), port));
    }

    // Launch Chromium with the OAuth URL.
    // Use direct process launch (not CDP) — CDP's /json/new endpoint
    // mangles URLs containing & by treating them as query param separators.
    let mut browser_opened = false;
    tracing::info!("OAuth: launching Chromium with auth URL");
    match std::process::Command::new("chromium")
        .args([
            "--ozone-platform=wayland",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            &auth_url,
        ])
        .env("WAYLAND_DISPLAY", "wayland-0")
        .env("XDG_RUNTIME_DIR", "/run/user/1000")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => {
            browser_opened = true;
            tracing::info!("OAuth: Chromium process spawned");
        }
        Err(e) => {
            tracing::error!("OAuth: Chromium spawn failed: {}", e);
        }
    }

    // Fallback for non-Alpine platforms
    if !browser_opened {
        let _ = std::process::Command::new("xdg-open")
            .arg(&auth_url)
            .spawn()
            .or_else(|_| std::process::Command::new("open").arg(&auth_url).spawn())
            .or_else(|_| std::process::Command::new("cmd").args(["/c", "start", &auth_url]).spawn());
    }

    // Accept with 180-second timeout
    let accept_timeout = Duration::from_secs(180);
    let start = std::time::Instant::now();

    // Accept one connection (non-blocking with short sleeps)
    let (mut stream, _) = loop {
        listener.set_nonblocking(true).ok();
        match listener.accept() {
            Ok(conn) => break conn,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > accept_timeout {
                    return Err("Timed out waiting for Google sign-in. Open the URL shown in the status bar in your browser.".to_string());
                }
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(e) => return Err(format!("Accept failed: {}", e)),
        }
    };

    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).map_err(|e| format!("Read: {}", e))?;
    let request = String::from_utf8_lossy(&buf[..n]).to_string();

    // Extract code from GET /?code=AUTH_CODE&scope=...
    let code = request.split_whitespace()
        .nth(1) // the URI path
        .and_then(|uri| {
            uri.split('?').nth(1)
        })
        .and_then(|query| {
            query.split('&').find_map(|param| {
                let mut kv = param.splitn(2, '=');
                match (kv.next(), kv.next()) {
                    (Some("code"), Some(v)) => Some(v.to_string()),
                    _ => None,
                }
            })
        })
        .ok_or_else(|| {
            // Check for error
            let err = request.split_whitespace()
                .nth(1)
                .and_then(|uri| uri.split('?').nth(1))
                .and_then(|q| q.split('&').find_map(|p| {
                    let mut kv = p.splitn(2, '=');
                    match (kv.next(), kv.next()) {
                        (Some("error"), Some(v)) => Some(v.to_string()),
                        _ => None,
                    }
                }))
                .unwrap_or_else(|| "unknown".to_string());
            format!("OAuth error: {}", err)
        })?;

    // Send a nice response to the browser
    let response_body = "<html><body style='background:#0c0b10;color:#5ac8d4;font-family:monospace;display:flex;justify-content:center;align-items:center;height:100vh;margin:0'><div style='text-align:center'><h1>Signed in!</h1><p>You can close this tab and return to Yantrik.</p></div></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body,
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    Ok((code, port))
}

/// Exchange authorization code for access + refresh tokens.
fn oauth2_exchange_code(code: &str, redirect_uri: &str) -> Result<OAuthTokens, String> {
    let resp: serde_json::Value = ureq::post(GOOGLE_TOKEN_URL)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&format!(
            "code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
            urlencoding(code),
            urlencoding(GOOGLE_CLIENT_ID),
            urlencoding(GOOGLE_CLIENT_SECRET),
            urlencoding(redirect_uri),
        ))
        .map_err(|e| format!("Token exchange failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Parse token response: {}", e))?;

    if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
        let desc = resp.get("error_description").and_then(|d| d.as_str()).unwrap_or("");
        return Err(format!("Token error: {} — {}", err, desc));
    }

    Ok(OAuthTokens {
        access_token: resp["access_token"].as_str().unwrap_or("").to_string(),
        refresh_token: resp["refresh_token"].as_str().unwrap_or("").to_string(),
        expires_in: resp["expires_in"].as_u64().unwrap_or(3600),
    })
}

/// Refresh an expired access token using the refresh token.
fn oauth2_refresh_token(refresh_token: &str) -> Result<OAuthTokens, String> {
    let resp: serde_json::Value = ureq::post(GOOGLE_TOKEN_URL)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&format!(
            "refresh_token={}&client_id={}&client_secret={}&grant_type=refresh_token",
            urlencoding(refresh_token),
            urlencoding(GOOGLE_CLIENT_ID),
            urlencoding(GOOGLE_CLIENT_SECRET),
        ))
        .map_err(|e| format!("Token refresh failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Parse refresh response: {}", e))?;

    if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
        return Err(format!("Refresh error: {}", err));
    }

    Ok(OAuthTokens {
        access_token: resp["access_token"].as_str().unwrap_or("").to_string(),
        refresh_token: refresh_token.to_string(), // refresh token stays the same
        expires_in: resp["expires_in"].as_u64().unwrap_or(3600),
    })
}

/// Get user's email from Google userinfo endpoint.
fn oauth2_get_email(access_token: &str) -> Result<(String, String), String> {
    let resp: serde_json::Value = ureq::get("https://www.googleapis.com/oauth2/v2/userinfo")
        .set("Authorization", &format!("Bearer {}", access_token))
        .call()
        .map_err(|e| format!("Userinfo failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Parse userinfo: {}", e))?;

    let email = resp["email"].as_str().unwrap_or("").to_string();
    let name = resp["name"].as_str().unwrap_or("").to_string();
    if email.is_empty() {
        return Err("Could not get email from Google".to_string());
    }
    Ok((email, name))
}

/// Minimal URL encoding (percent-encode).
fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

/// XOAUTH2 SASL authenticator for IMAP.
struct XOAuth2 {
    user: String,
    token: String,
}

impl imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&self, _challenge: &[u8]) -> Self::Response {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

/// SMTP send email (blocking). Supports password and OAuth2 (XOAUTH2).
fn smtp_send_email(
    server: &str,
    port: u16,
    from_email: &str,
    from_name: &str,
    password: &str,
    to: &str,
    cc: &str,
    bcc: &str,
    subject: &str,
    body: &str,
    oauth_token: Option<&str>,
) -> Result<String, String> {
    use lettre::message::header::ContentType;
    use lettre::message::{MultiPart, SinglePart};
    use lettre::transport::smtp::authentication::{Credentials, Mechanism};
    use lettre::{Message, SmtpTransport, Transport};

    let mut builder = Message::builder()
        .from(format!("{} <{}>", from_name, from_email).parse()
            .map_err(|e| format!("Bad from: {}", e))?)
        .subject(subject);

    // Add To recipients
    for addr in to.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        builder = builder.to(addr.parse().map_err(|e| format!("Bad To '{}': {}", addr, e))?);
    }
    // Add CC
    for addr in cc.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        builder = builder.cc(addr.parse().map_err(|e| format!("Bad Cc '{}': {}", addr, e))?);
    }
    // Add BCC
    for addr in bcc.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        builder = builder.bcc(addr.parse().map_err(|e| format!("Bad Bcc '{}': {}", addr, e))?);
    }

    // Build multipart/alternative email with both plain text and HTML
    let html_body = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8">
<style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
           font-size: 14px; line-height: 1.6; color: #333; }}
    a {{ color: #0066cc; }}
</style></head><body>{}</body></html>"#,
        body.replace('\n', "<br>\n")
    );

    let email = builder
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(body.to_string())
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html_body)
                )
        )
        .map_err(|e| format!("Build email: {}", e))?;

    let mailer = if let Some(token) = oauth_token {
        // OAuth2: use XOAUTH2 mechanism with access token as password
        let creds = Credentials::new(from_email.to_string(), token.to_string());
        SmtpTransport::starttls_relay(server)
            .map_err(|e| format!("SMTP relay: {}", e))?
            .port(port)
            .credentials(creds)
            .authentication(vec![Mechanism::Xoauth2])
            .timeout(Some(Duration::from_secs(15)))
            .build()
    } else {
        let creds = Credentials::new(from_email.to_string(), password.to_string());
        SmtpTransport::starttls_relay(server)
            .map_err(|e| format!("SMTP relay: {}", e))?
            .port(port)
            .credentials(creds)
            .timeout(Some(Duration::from_secs(15)))
            .build()
    };

    mailer.send(&email).map_err(|e| format!("Send failed: {}", e))?;
    Ok("Email sent!".to_string())
}

/// Test IMAP connection (blocking).
fn test_imap_connection(server: &str, port: u16, email: &str, password: &str) -> Result<String, String> {
    use std::net::TcpStream;

    let addr = format!("{}:{}", server, port);
    let sock_addr = std::net::ToSocketAddrs::to_socket_addrs(&addr.as_str())
        .map_err(|e| format!("DNS failed for {}: {}", addr, e))?
        .next()
        .ok_or_else(|| format!("No addresses for {}", addr))?;

    let tcp = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(10))
        .map_err(|e| format!("Cannot reach {}:{} — {}", server, port, e))?;

    let tls = native_tls::TlsConnector::new().map_err(|e| format!("TLS: {}", e))?;
    let tls_stream = tls.connect(server, tcp).map_err(|e| format!("TLS: {}", e))?;

    let mut client = imap::Client::new(tls_stream);
    let mut session = client.login(email, password).map_err(|e| {
        let err = format!("{}", e.0);
        if err.contains("AUTHENTICATIONFAILED") || err.contains("Invalid credentials") || err.contains("authentication failed") {
            format!("Auth failed. For Gmail/Yahoo/iCloud, use an App Password (not your regular password). Go to account security settings to generate one.")
        } else {
            format!("Login failed: {}", err)
        }
    })?;
    let mailbox = session.select("INBOX").map_err(|e| format!("INBOX: {}", e))?;
    let count = mailbox.exists;
    let _ = session.logout();

    Ok(format!("Connected! {} emails in INBOX.", count))
}

/// Wire email callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();

    // Read account from config
    let account = read_account_from_config(ctx);
    let has_account = account.is_some();
    ui.set_email_has_account(has_account);

    let account_info = Rc::new(RefCell::new(account.clone().unwrap_or_default()));

    if let Some(ref acct) = account {
        let display = if acct.display_name.is_empty() { &acct.email } else { &acct.display_name };
        ui.set_email_account_name(display.clone().into());
    }

    // Initialize folders
    let default_folders = Rc::new(VecModel::from(vec![
        EmailFolderData { name: "Inbox".into(), icon: "\u{1F4E5}".into(), unread_count: 0, total_count: 0, is_selected: true, folder_type: "inbox".into() },
        EmailFolderData { name: "Starred".into(), icon: "\u{2B50}".into(), unread_count: 0, total_count: 0, is_selected: false, folder_type: "starred".into() },
        EmailFolderData { name: "Sent".into(), icon: "\u{1F4E4}".into(), unread_count: 0, total_count: 0, is_selected: false, folder_type: "sent".into() },
        EmailFolderData { name: "Drafts".into(), icon: "\u{1F4DD}".into(), unread_count: 0, total_count: 0, is_selected: false, folder_type: "drafts".into() },
        EmailFolderData { name: "Archive".into(), icon: "\u{1F4E6}".into(), unread_count: 0, total_count: 0, is_selected: false, folder_type: "archive".into() },
        EmailFolderData { name: "Trash".into(), icon: "\u{1F5D1}".into(), unread_count: 0, total_count: 0, is_selected: false, folder_type: "trash".into() },
    ]));
    ui.set_email_folders(ModelRc::from(default_folders.clone()));

    let email_model = Rc::new(VecModel::<EmailListItem>::default());
    ui.set_email_list(ModelRc::from(email_model.clone()));

    let current_folder = Rc::new(RefCell::new("INBOX".to_string()));

    // Cache fetched email bodies for detail view
    let email_cache: Rc<RefCell<Vec<FetchedEmail>>> = Rc::new(RefCell::new(Vec::new()));

    // ── Auto-fetch on startup if account exists ──
    if has_account {
        let acct = account.clone().unwrap();
        let ui_weak = ui.as_weak();
        let model = email_model.clone();
        let cache = email_cache.clone();
        let folders = default_folders.clone();
        // Delay slightly so UI can render first
        let t = Timer::default();
        t.start(TimerMode::SingleShot, Duration::from_millis(500), move || {
            fetch_and_populate(acct.clone(), "INBOX", ui_weak.clone(), model.clone(), cache.clone(), folders.clone());
        });
        std::mem::forget(t);
    }

    // ── Save Account ──
    {
        let config_path = ctx.config_path.clone();
        let ui_weak = ui.as_weak();
        let acct_info = account_info.clone();
        ui.on_email_save_account(move |email, password, display_name, provider, imap_srv_in, imap_port_in, smtp_srv_in, smtp_port_in| {
            let email_str = email.to_string();
            let password_str = password.to_string();
            let display_name_str = display_name.to_string();
            let provider_str = provider.to_string();

            if email_str.is_empty() || password_str.is_empty() {
                if let Some(ui) = ui_weak.upgrade() { ui.set_email_setup_status("Please enter email and password.".into()); }
                return;
            }
            if !email_str.contains('@') {
                if let Some(ui) = ui_weak.upgrade() { ui.set_email_setup_status("Please enter a valid email address.".into()); }
                return;
            }
            if provider_str.is_empty() {
                if let Some(ui) = ui_weak.upgrade() { ui.set_email_setup_status("Please select a provider.".into()); }
                return;
            }

            let srv = resolve_server(
                &email_str, &provider_str,
                &imap_srv_in.to_string(), &imap_port_in.to_string(),
                &smtp_srv_in.to_string(), &smtp_port_in.to_string(),
            );

            let name = if display_name_str.is_empty() { email_str.clone() } else { display_name_str };

            // Update cached account info
            {
                let mut info = acct_info.borrow_mut();
                info.email = email_str.clone();
                info.password = password_str.clone();
                info.display_name = name.clone();
                info.provider = srv.provider.clone();
                info.imap_server = srv.imap_server.clone();
                info.imap_port = srv.imap_port;
                info.smtp_server = srv.smtp_server.clone();
                info.smtp_port = srv.smtp_port;
            }

            let email_yaml = format!(
                r#"
email:
  enabled: true
  poll_interval_minutes: 5
  notify_important: true
  accounts:
    - name: "{name}"
      email: "{email}"
      provider: "{provider}"
      imap_server: "{imap_srv}"
      imap_port: {imap_port}
      smtp_server: "{smtp_srv}"
      smtp_port: {smtp_port}
      password: "{password}"
"#,
                name = name, email = email_str, provider = srv.provider,
                imap_srv = srv.imap_server, imap_port = srv.imap_port,
                smtp_srv = srv.smtp_server, smtp_port = srv.smtp_port,
                password = password_str,
            );

            if let Some(ref path) = config_path {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        let new_content = if content.contains("\nemail:") {
                            let mut lines: Vec<&str> = content.lines().collect();
                            let mut start = None;
                            let mut end = None;
                            for (i, line) in lines.iter().enumerate() {
                                if line.starts_with("email:") { start = Some(i); }
                                else if start.is_some() && end.is_none() && !line.is_empty()
                                    && !line.starts_with(' ') && !line.starts_with('#') {
                                    end = Some(i);
                                }
                            }
                            if let Some(s) = start {
                                let e = end.unwrap_or(lines.len());
                                lines.drain(s..e);
                                format!("{}\n{}", lines.join("\n").trim_end(), email_yaml.trim())
                            } else {
                                format!("{}\n{}", content.trim_end(), email_yaml.trim())
                            }
                        } else {
                            format!("{}\n{}", content.trim_end(), email_yaml.trim())
                        };

                        match std::fs::write(path, &new_content) {
                            Ok(_) => {
                                tracing::info!(email = %email_str, "Email account saved");
                                if let Some(ui) = ui_weak.upgrade() {
                                    ui.set_email_setup_status(format!("Saved! Connected to {}:{}", srv.imap_server, srv.imap_port).into());
                                    ui.set_email_has_account(true);
                                    ui.set_email_account_name(name.into());
                                }
                            }
                            Err(e) => {
                                if let Some(ui) = ui_weak.upgrade() { ui.set_email_setup_status(format!("Save failed: {}", e).into()); }
                            }
                        }
                    }
                    Err(e) => {
                        if let Some(ui) = ui_weak.upgrade() { ui.set_email_setup_status(format!("Cannot read config: {}", e).into()); }
                    }
                }
            }
        });
    }

    // ── Test Connection ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_test_connection(move |email, password, provider, imap_srv_in, imap_port_in| {
            let email_str = email.to_string();
            let password_str = password.to_string();
            let provider_str = provider.to_string();

            if email_str.is_empty() || password_str.is_empty() {
                if let Some(ui) = ui_weak.upgrade() { ui.set_email_setup_status("Enter email and password first.".into()); }
                return;
            }
            if provider_str.is_empty() {
                if let Some(ui) = ui_weak.upgrade() { ui.set_email_setup_status("Please select a provider first.".into()); }
                return;
            }

            let srv = resolve_server(&email_str, &provider_str, &imap_srv_in.to_string(), &imap_port_in.to_string(), "", "");
            let imap_srv = srv.imap_server;
            let imap_port = srv.imap_port;

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_setup_testing(true);
                ui.set_email_setup_status(format!("Connecting to {}:{}...", imap_srv, imap_port).into());
            }

            let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
            std::thread::spawn(move || {
                let result = test_imap_connection(&imap_srv, imap_port, &email_str, &password_str);
                let _ = tx.send(result);
            });

            let weak = ui_weak.clone();
            let t = Timer::default();
            t.start(TimerMode::Repeated, Duration::from_millis(100), move || {
                if let Ok(result) = rx.try_recv() {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_email_setup_testing(false);
                        match result {
                            Ok(msg) => ui.set_email_setup_status(msg.into()),
                            Err(msg) => ui.set_email_setup_status(msg.into()),
                        }
                    }
                }
            });
            std::mem::forget(t);
        });
    }

    // ── Sign in with Google (OAuth2) ──
    {
        let config_path = ctx.config_path.clone();
        let ui_weak = ui.as_weak();
        let acct_info = account_info.clone();
        let model = email_model.clone();
        let cache = email_cache.clone();
        let folders = default_folders.clone();
        ui.on_email_oauth_google(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_setup_status("Opening Google sign-in...".into());
                ui.set_email_setup_testing(true);
            }

            let (tx, rx) = std::sync::mpsc::channel::<Result<(OAuthTokens, String, String), String>>();
            let (url_tx, url_rx) = std::sync::mpsc::channel::<(String, u16)>();

            std::thread::spawn(move || {
                let result = (|| {
                    let (code, port) = oauth2_authorize_google(Some(url_tx))?;
                    let redirect_uri = format!("http://localhost:{}", port);
                    let tokens = oauth2_exchange_code(&code, &redirect_uri)?;
                    let (email, name) = oauth2_get_email(&tokens.access_token)?;
                    Ok((tokens, email, name))
                })();
                let _ = tx.send(result);
            });

            // Show the OAuth URL in status so user can open manually if needed
            {
                let weak2 = ui_weak.clone();
                let t_url = Timer::default();
                t_url.start(TimerMode::Repeated, Duration::from_millis(100), move || {
                    if let Ok((url, _port)) = url_rx.try_recv() {
                        tracing::info!("OAuth URL (open manually if browser didn't launch): {}", url);
                        if let Some(ui) = weak2.upgrade() {
                            ui.set_email_setup_status(
                                "Waiting for Google sign-in in browser...".into()
                            );
                        }
                    }
                });
                std::mem::forget(t_url);
            }

            let weak = ui_weak.clone();
            let config_path2 = config_path.clone();
            let acct_info2 = acct_info.clone();
            let model2 = model.clone();
            let cache2 = cache.clone();
            let folders2 = folders.clone();
            let t = Timer::default();
            t.start(TimerMode::Repeated, Duration::from_millis(200), move || {
                if let Ok(result) = rx.try_recv() {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_email_setup_testing(false);
                        match result {
                            Ok((tokens, email, name)) => {
                                let now_ts = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs_f64())
                                    .unwrap_or(0.0);
                                let expiry_ts = now_ts + tokens.expires_in as f64;

                                let display_name = if name.is_empty() { email.clone() } else { name.clone() };
                                let srv = provider_servers("gmail").unwrap();

                                // Update cached account info
                                {
                                    let mut info = acct_info2.borrow_mut();
                                    info.email = email.clone();
                                    info.password = String::new();
                                    info.display_name = display_name.clone();
                                    info.provider = "gmail".to_string();
                                    info.imap_server = srv.imap_server.clone();
                                    info.imap_port = srv.imap_port;
                                    info.smtp_server = srv.smtp_server.clone();
                                    info.smtp_port = srv.smtp_port;
                                    info.auth_method = "oauth2".to_string();
                                    info.oauth_access_token = tokens.access_token.clone();
                                    info.oauth_refresh_token = tokens.refresh_token.clone();
                                    info.oauth_token_expiry = expiry_ts;
                                }

                                // Save to config.yaml
                                let email_yaml = format!(
                                    r#"
email:
  enabled: true
  poll_interval_minutes: 5
  notify_important: true
  accounts:
    - name: "{name}"
      email: "{email}"
      provider: "gmail"
      auth_method: "oauth2"
      imap_server: "{imap_srv}"
      imap_port: {imap_port}
      smtp_server: "{smtp_srv}"
      smtp_port: {smtp_port}
      password: ""
      oauth_access_token: "{access}"
      oauth_refresh_token: "{refresh}"
      oauth_token_expiry: {expiry}
"#,
                                    name = display_name, email = email,
                                    imap_srv = srv.imap_server, imap_port = srv.imap_port,
                                    smtp_srv = srv.smtp_server, smtp_port = srv.smtp_port,
                                    access = tokens.access_token,
                                    refresh = tokens.refresh_token,
                                    expiry = expiry_ts,
                                );

                                if let Some(ref path) = config_path2 {
                                    if let Ok(content) = std::fs::read_to_string(path) {
                                        let new_content = if content.contains("\nemail:") {
                                            let mut lines: Vec<&str> = content.lines().collect();
                                            let mut start = None;
                                            let mut end = None;
                                            for (i, line) in lines.iter().enumerate() {
                                                if line.starts_with("email:") { start = Some(i); }
                                                else if start.is_some() && end.is_none() && !line.is_empty()
                                                    && !line.starts_with(' ') && !line.starts_with('#') {
                                                    end = Some(i);
                                                }
                                            }
                                            if let Some(s) = start {
                                                let e = end.unwrap_or(lines.len());
                                                lines.drain(s..e);
                                                format!("{}\n{}", lines.join("\n").trim_end(), email_yaml.trim())
                                            } else {
                                                format!("{}\n{}", content.trim_end(), email_yaml.trim())
                                            }
                                        } else {
                                            format!("{}\n{}", content.trim_end(), email_yaml.trim())
                                        };
                                        let _ = std::fs::write(path, &new_content);
                                    }
                                }

                                tracing::info!(email = %email, "Google OAuth2 sign-in successful");
                                ui.set_email_setup_status(format!("Signed in as {}!", email).into());
                                ui.set_email_has_account(true);
                                ui.set_email_account_name(display_name.into());

                                // Auto-fetch inbox
                                let info = acct_info2.borrow().clone();
                                fetch_and_populate(info, "INBOX", weak.clone(), model2.clone(), cache2.clone(), folders2.clone());
                            }
                            Err(msg) => {
                                tracing::error!(error = %msg, "Google OAuth2 failed");
                                ui.set_email_setup_status(msg.into());
                            }
                        }
                    }
                }
            });
            std::mem::forget(t);
        });
    }

    // ── Folder click ──
    {
        let ui_weak = ui.as_weak();
        let folder = current_folder.clone();
        let folders_model = default_folders.clone();
        let model = email_model.clone();
        let cache = email_cache.clone();
        let acct = account_info.clone();
        ui.on_email_folder_clicked(move |idx| {
            let idx = idx as usize;
            let folder_names = ["Inbox", "Starred", "Sent", "Drafts", "Archive", "Trash"];
            if idx < folder_names.len() {
                *folder.borrow_mut() = folder_names[idx].to_string();
                for i in 0..folders_model.row_count() {
                    if let Some(mut f) = folders_model.row_data(i) {
                        f.is_selected = i == idx;
                        folders_model.set_row_data(i, f);
                    }
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_email_selected_folder(idx as i32);
                }
                // Map folder names to IMAP folder names
                let imap_folder = match folder_names[idx] {
                    "Inbox" => "INBOX",
                    "Starred" => "[Gmail]/Starred",
                    "Sent" => "[Gmail]/Sent Mail",
                    "Drafts" => "[Gmail]/Drafts",
                    "Archive" => "[Gmail]/All Mail",
                    "Trash" => "[Gmail]/Trash",
                    _ => "INBOX",
                };
                let info = acct.borrow().clone();
                if !info.email.is_empty() {
                    fetch_and_populate(info, imap_folder, ui_weak.clone(), model.clone(), cache.clone(), folders_model.clone());
                }
            }
        });
    }

    // ── Email selected (show detail) ──
    {
        let cache = email_cache.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_selected(move |id| {
            if let Some(ui) = ui_weak.upgrade() {
                let cached = cache.borrow();
                if let Some(email) = cached.iter().find(|e| e.id == id) {
                    ui.set_email_detail(EmailDetailData {
                        id: email.id,
                        from_name: email.from_name.clone().into(),
                        from_addr: email.from_addr.clone().into(),
                        to_addr: email.to_addr.clone().into(),
                        cc_addr: "".into(),
                        subject: email.subject.clone().into(),
                        date_text: email.date_text.clone().into(),
                        body: email.body.clone().into(),
                        ai_summary: "".into(),
                        is_flagged: false,
                        is_read: email.is_read,
                        has_attachment: false,
                        attachment_names: "".into(),
                        thread_count: 0,
                    });
                    // Clear thread messages for non-threaded view
                    ui.set_email_thread_messages(ModelRc::from(Rc::new(VecModel::<EmailThreadMessage>::default())));
                    // Clear AI intent from previous email
                    ui.set_email_ai_intent("".into());
                }
            }
        });
    }

    // ── Compose new ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_compose_new(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_is_composing(true);
                ui.set_email_compose_to("".into());
                ui.set_email_compose_cc("".into());
                ui.set_email_compose_bcc("".into());
                ui.set_email_compose_subject("".into());
                ui.set_email_compose_body("".into());
            }
        });
    }

    // ── Send email via SMTP ──
    {
        let ui_weak = ui.as_weak();
        let acct = account_info.clone();
        ui.on_email_send(move |to, cc, bcc, subject, body| {
            let info = acct.borrow().clone();
            if info.email.is_empty() || info.smtp_server.is_empty() {
                tracing::warn!("No SMTP config for send");
                return;
            }
            let to_str = to.to_string();
            let cc_str = cc.to_string();
            let bcc_str = bcc.to_string();
            let subject_str = subject.to_string();
            let body_str = body.to_string();

            tracing::info!(to = %to_str, subject = %subject_str, "Sending email via SMTP");

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_is_loading(true);
            }

            let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
            std::thread::spawn(move || {
                let oauth_token = if info.is_oauth() {
                    match info.get_valid_token() {
                        Ok((tok, _)) => Some(tok),
                        Err(e) => { let _ = tx.send(Err(e)); return; }
                    }
                } else {
                    None
                };
                let result = smtp_send_email(
                    &info.smtp_server, info.smtp_port,
                    &info.email, &info.display_name, &info.password,
                    &to_str, &cc_str, &bcc_str, &subject_str, &body_str,
                    oauth_token.as_deref(),
                );
                let _ = tx.send(result);
            });

            let weak = ui_weak.clone();
            let t = Timer::default();
            t.start(TimerMode::Repeated, Duration::from_millis(100), move || {
                if let Ok(result) = rx.try_recv() {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_email_is_loading(false);
                        ui.set_email_is_composing(false);
                        match result {
                            Ok(msg) => tracing::info!("{}", msg),
                            Err(msg) => tracing::error!("Send failed: {}", msg),
                        }
                    }
                }
            });
            std::mem::forget(t);
        });
    }

    // ── Reply ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_reply(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let detail = ui.get_email_detail();
                ui.set_email_is_composing(true);
                ui.set_email_compose_to(detail.from_addr);
                ui.set_email_compose_cc("".into());
                ui.set_email_compose_bcc("".into());
                let subject = detail.subject.to_string();
                let re = if subject.starts_with("Re: ") { subject } else { format!("Re: {}", subject) };
                ui.set_email_compose_subject(re.into());
                ui.set_email_compose_body("".into());
            }
        });
    }

    // ── Forward ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_forward(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let detail = ui.get_email_detail();
                ui.set_email_is_composing(true);
                ui.set_email_compose_to("".into());
                ui.set_email_compose_cc("".into());
                ui.set_email_compose_bcc("".into());
                let subject = detail.subject.to_string();
                let fwd = if subject.starts_with("Fwd: ") { subject } else { format!("Fwd: {}", subject) };
                ui.set_email_compose_subject(fwd.into());
                let body = format!(
                    "\n\n---------- Forwarded message ----------\nFrom: {} <{}>\nDate: {}\nSubject: {}\n\n{}",
                    detail.from_name, detail.from_addr, detail.date_text, detail.subject, detail.body
                );
                ui.set_email_compose_body(body.into());
            }
        });
    }

    // ── Reply All ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_reply_all(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let detail = ui.get_email_detail();
                ui.set_email_is_composing(true);
                ui.set_email_compose_to(detail.from_addr.clone());
                // Put the To and CC from original as CC
                let mut cc_parts = Vec::new();
                let to_str = detail.to_addr.to_string();
                if !to_str.is_empty() { cc_parts.push(to_str); }
                let cc_str = detail.cc_addr.to_string();
                if !cc_str.is_empty() { cc_parts.push(cc_str); }
                ui.set_email_compose_cc(cc_parts.join(", ").into());
                ui.set_email_compose_bcc("".into());
                let subject = detail.subject.to_string();
                let re = if subject.starts_with("Re: ") { subject } else { format!("Re: {}", subject) };
                ui.set_email_compose_subject(re.into());
                ui.set_email_compose_body("".into());
            }
        });
    }

    // ── Archive ──
    { ui.on_email_archive(move |id| { tracing::info!(email_id = id, "Archive requested"); }); }

    // ── Delete / Mark read / Mark flagged ──
    { ui.on_email_delete(move |id| { tracing::info!(email_id = id, "Delete requested"); }); }
    { ui.on_email_mark_read(move |id| { tracing::debug!(email_id = id, "Mark read"); }); }
    { ui.on_email_mark_flagged(move |id| { tracing::debug!(email_id = id, "Mark flagged"); }); }

    // ── Toggle thread message collapse/expand ──
    { ui.on_email_toggle_thread_message(move |id| { tracing::debug!(msg_id = id, "Toggle thread message"); }); }

    // ── AI Summarize ──
    {
        let bridge = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_summarize(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let detail = ui.get_email_detail();
                if detail.id <= 0 { return; }
                ui.set_email_ai_working(true);

                let prompt = format!(
                    "Summarize this email in 2-3 concise sentences. Focus on key action items.\n\nFrom: {} <{}>\nSubject: {}\n\n{}\n",
                    detail.from_name, detail.from_addr, detail.subject, detail.body
                );
                let token_rx = bridge.send_message(prompt);
                let weak = ui_weak.clone();
                let collected = Rc::new(RefCell::new(String::new()));
                let ci = collected.clone();
                let eid = detail.id;
                let t = Timer::default();
                t.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                    let mut got = false;
                    while let Ok(tok) = token_rx.try_recv() {
                        if tok == "__DONE__" {
                            if let Some(ui) = weak.upgrade() {
                                let mut d = ui.get_email_detail();
                                if d.id == eid { d.ai_summary = ci.borrow().clone().into(); ui.set_email_detail(d); }
                                ui.set_email_ai_working(false);
                            }
                            return;
                        }
                        if tok == "__REPLACE__" { ci.borrow_mut().clear(); continue; }
                        if tok.starts_with("__") && tok.ends_with("__") { continue; }
                        ci.borrow_mut().push_str(&tok);
                        got = true;
                    }
                    if got {
                        if let Some(ui) = weak.upgrade() {
                            let mut d = ui.get_email_detail();
                            if d.id == eid { d.ai_summary = ci.borrow().clone().into(); ui.set_email_detail(d); }
                        }
                    }
                });
                std::mem::forget(t);
            }
        });
    }

    // ── AI Enhance (improve compose text) ──
    {
        let bridge = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_enhance(move |text| {
            if text.is_empty() { return; }
            if let Some(ui) = ui_weak.upgrade() { ui.set_email_ai_working(true); }
            let prompt = format!(
                "Improve the following email text. Make it professional and clear. Return only the improved text, no explanation.\n\n{}\n",
                text
            );
            let token_rx = bridge.send_message(prompt);
            let weak = ui_weak.clone();
            let ci = Rc::new(RefCell::new(String::new()));
            let ci2 = ci.clone();
            let t = Timer::default();
            t.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                while let Ok(tok) = token_rx.try_recv() {
                    if tok == "__DONE__" {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_email_compose_body(ci2.borrow().clone().into());
                            ui.set_email_ai_working(false);
                        }
                        return;
                    }
                    if tok == "__REPLACE__" { ci2.borrow_mut().clear(); continue; }
                    if tok.starts_with("__") && tok.ends_with("__") { continue; }
                    ci2.borrow_mut().push_str(&tok);
                }
            });
            std::mem::forget(t);
        });
    }

    // ── AI Draft (generate email from prompt) ──
    {
        let bridge = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_ai_draft(move |prompt| {
            if prompt.is_empty() { return; }
            if let Some(ui) = ui_weak.upgrade() { ui.set_email_ai_working(true); }
            let full_prompt = format!(
                "Write an email based on this description. Return only the email body text, no subject line or headers. Be professional.\n\nDescription: {}\n",
                prompt
            );
            let token_rx = bridge.send_message(full_prompt);
            let weak = ui_weak.clone();
            let ci = Rc::new(RefCell::new(String::new()));
            let ci2 = ci.clone();
            let t = Timer::default();
            t.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                while let Ok(tok) = token_rx.try_recv() {
                    if tok == "__DONE__" {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_email_compose_body(ci2.borrow().clone().into());
                            ui.set_email_ai_working(false);
                        }
                        return;
                    }
                    if tok == "__REPLACE__" { ci2.borrow_mut().clear(); continue; }
                    if tok.starts_with("__") && tok.ends_with("__") { continue; }
                    ci2.borrow_mut().push_str(&tok);
                }
            });
            std::mem::forget(t);
        });
    }

    // ── AI Reply Suggest (auto-draft reply) ──
    {
        let bridge = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_ai_reply_suggest(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let detail = ui.get_email_detail();
                if detail.id <= 0 { return; }
                ui.set_email_ai_working(true);

                let prompt = format!(
                    "Write a brief, professional reply to this email. Return only the reply text.\n\nFrom: {} <{}>\nSubject: {}\n\n{}\n",
                    detail.from_name, detail.from_addr, detail.subject, detail.body
                );
                let token_rx = bridge.send_message(prompt);
                let weak = ui_weak.clone();
                let ci = Rc::new(RefCell::new(String::new()));
                let ci2 = ci.clone();
                let from_addr = detail.from_addr.to_string();
                let subject = detail.subject.to_string();
                let t = Timer::default();
                t.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                    while let Ok(tok) = token_rx.try_recv() {
                        if tok == "__DONE__" {
                            if let Some(ui) = weak.upgrade() {
                                ui.set_email_is_composing(true);
                                ui.set_email_compose_to(from_addr.clone().into());
                                ui.set_email_compose_cc("".into());
                                ui.set_email_compose_bcc("".into());
                                let re = if subject.starts_with("Re: ") { subject.clone() } else { format!("Re: {}", subject) };
                                ui.set_email_compose_subject(re.into());
                                ui.set_email_compose_body(ci2.borrow().clone().into());
                                ui.set_email_ai_working(false);
                            }
                            return;
                        }
                        if tok == "__REPLACE__" { ci2.borrow_mut().clear(); continue; }
                        if tok.starts_with("__") && tok.ends_with("__") { continue; }
                        ci2.borrow_mut().push_str(&tok);
                    }
                });
                std::mem::forget(t);
            }
        });
    }

    // ── Add Account (reset setup view) ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_add_account(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_has_account(false);
                ui.set_email_setup_email("".into());
                ui.set_email_setup_password("".into());
                ui.set_email_setup_display_name("".into());
                ui.set_email_setup_provider("".into());
                ui.set_email_setup_status("".into());
            }
        });
    }

    // ── Sync (manual refresh) ──
    {
        let ui_weak = ui.as_weak();
        let acct = account_info.clone();
        let model = email_model.clone();
        let cache = email_cache.clone();
        let folder = current_folder.clone();
        let folders = default_folders.clone();
        ui.on_email_sync(move || {
            let info = acct.borrow().clone();
            if info.email.is_empty() { return; }
            let f = folder.borrow().clone();
            let imap_folder = match f.as_str() {
                "Inbox" | "INBOX" => "INBOX",
                "Starred" => "[Gmail]/Starred",
                "Sent" => "[Gmail]/Sent Mail",
                "Drafts" => "[Gmail]/Drafts",
                "Archive" => "[Gmail]/All Mail",
                "Trash" => "[Gmail]/Trash",
                _ => "INBOX",
            };
            fetch_and_populate(info, imap_folder, ui_weak.clone(), model.clone(), cache.clone(), folders.clone());
        });
    }

    // ── Search (client-side filter) ──
    {
        let cache = email_cache.clone();
        let model = email_model.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_search(move |query| {
            let q = query.to_string().to_lowercase();
            let cached = cache.borrow();
            if q.is_empty() {
                // Restore full list
                while model.row_count() > 0 { model.remove(0); }
                for e in cached.iter() {
                    let initial = if !e.from_name.is_empty() {
                        e.from_name.chars().next().unwrap_or('?').to_uppercase().to_string()
                    } else {
                        e.from_addr.chars().next().unwrap_or('?').to_uppercase().to_string()
                    };
                    let color_seed: u32 = e.from_addr.bytes().map(|b| b as u32).sum();
                    let avatar_color = match color_seed % 6 {
                        0 => slint::Color::from_rgb_u8(90, 200, 212),
                        1 => slint::Color::from_rgb_u8(212, 165, 116),
                        2 => slint::Color::from_rgb_u8(180, 132, 224),
                        3 => slint::Color::from_rgb_u8(108, 212, 128),
                        4 => slint::Color::from_rgb_u8(232, 120, 152),
                        _ => slint::Color::from_rgb_u8(74, 184, 240),
                    };
                    model.push(EmailListItem {
                        id: e.id, from_name: e.from_name.clone().into(), from_addr: e.from_addr.clone().into(),
                        subject: e.subject.clone().into(), preview: e.preview.clone().into(),
                        date_text: e.date_text.clone().into(), is_read: e.is_read, is_flagged: false,
                        is_selected: false, has_attachment: false, thread_count: 0, thread_id: "".into(),
                        avatar_initial: initial.into(), avatar_color,
                    });
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_email_search_active(false);
                    ui.set_email_search_count(0);
                    ui.set_email_folder_total(cached.len() as i32);
                    ui.set_email_folder_unread(cached.iter().filter(|e| !e.is_read).count() as i32);
                }
            } else {
                // Filter by sender, subject, body
                let matches: Vec<&FetchedEmail> = cached.iter().filter(|e| {
                    e.from_name.to_lowercase().contains(&q)
                    || e.from_addr.to_lowercase().contains(&q)
                    || e.subject.to_lowercase().contains(&q)
                    || e.body.to_lowercase().contains(&q)
                    || e.preview.to_lowercase().contains(&q)
                }).collect();
                let count = matches.len() as i32;
                while model.row_count() > 0 { model.remove(0); }
                for e in &matches {
                    let initial = if !e.from_name.is_empty() {
                        e.from_name.chars().next().unwrap_or('?').to_uppercase().to_string()
                    } else {
                        e.from_addr.chars().next().unwrap_or('?').to_uppercase().to_string()
                    };
                    let color_seed: u32 = e.from_addr.bytes().map(|b| b as u32).sum();
                    let avatar_color = match color_seed % 6 {
                        0 => slint::Color::from_rgb_u8(90, 200, 212),
                        1 => slint::Color::from_rgb_u8(212, 165, 116),
                        2 => slint::Color::from_rgb_u8(180, 132, 224),
                        3 => slint::Color::from_rgb_u8(108, 212, 128),
                        4 => slint::Color::from_rgb_u8(232, 120, 152),
                        _ => slint::Color::from_rgb_u8(74, 184, 240),
                    };
                    model.push(EmailListItem {
                        id: e.id, from_name: e.from_name.clone().into(), from_addr: e.from_addr.clone().into(),
                        subject: e.subject.clone().into(), preview: e.preview.clone().into(),
                        date_text: e.date_text.clone().into(), is_read: e.is_read, is_flagged: false,
                        is_selected: false, has_attachment: false, thread_count: 0, thread_id: "".into(),
                        avatar_initial: initial.into(), avatar_color,
                    });
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_email_search_active(true);
                    ui.set_email_search_count(count);
                    ui.set_email_folder_total(count);
                }
            }
            tracing::debug!(query = %q, "Email search");
        });
    }

    // ── Cancel compose ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_cancel_compose(move || {
            if let Some(ui) = ui_weak.upgrade() { ui.set_email_is_composing(false); }
        });
    }

    // ── V41: Triage filter ──
    {
        let cache = email_cache.clone();
        let model = email_model.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_triage_filter(move |view| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_triage_view(view);
            }
            let cached = cache.borrow();
            // Filter based on triage view
            let filtered: Vec<&FetchedEmail> = cached.iter().filter(|e| {
                match view {
                    0 => true,  // All
                    1 => !e.is_read, // Priority proxy: unread with content (heuristic)
                    2 => !e.is_read, // Unread
                    3 => false,      // Flagged — we don't persist flag state in cache yet
                    _ => true,
                }
            }).collect();

            while model.row_count() > 0 { model.remove(0); }
            for e in &filtered {
                let initial = if !e.from_name.is_empty() {
                    e.from_name.chars().next().unwrap_or('?').to_uppercase().to_string()
                } else {
                    e.from_addr.chars().next().unwrap_or('?').to_uppercase().to_string()
                };
                let color_seed: u32 = e.from_addr.bytes().map(|b| b as u32).sum();
                let avatar_color = match color_seed % 6 {
                    0 => slint::Color::from_rgb_u8(90, 200, 212),
                    1 => slint::Color::from_rgb_u8(212, 165, 116),
                    2 => slint::Color::from_rgb_u8(180, 132, 224),
                    3 => slint::Color::from_rgb_u8(108, 212, 128),
                    4 => slint::Color::from_rgb_u8(232, 120, 152),
                    _ => slint::Color::from_rgb_u8(74, 184, 240),
                };
                model.push(EmailListItem {
                    id: e.id, from_name: e.from_name.clone().into(), from_addr: e.from_addr.clone().into(),
                    subject: e.subject.clone().into(), preview: e.preview.clone().into(),
                    date_text: e.date_text.clone().into(), is_read: e.is_read, is_flagged: false,
                    is_selected: false, has_attachment: false, thread_count: 0, thread_id: "".into(),
                    avatar_initial: initial.into(), avatar_color,
                });
            }

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_folder_total(filtered.len() as i32);
                ui.set_email_folder_unread(filtered.iter().filter(|e| !e.is_read).count() as i32);
            }
            tracing::debug!(view = view, count = filtered.len(), "Triage filter applied");
        });
    }

    // ── V41: Save draft ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_save_draft(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_draft_status("Saving...".into());
                let to = ui.get_email_compose_to().to_string();
                let cc = ui.get_email_compose_cc().to_string();
                let bcc = ui.get_email_compose_bcc().to_string();
                let subject = ui.get_email_compose_subject().to_string();
                let body = ui.get_email_compose_body().to_string();
                tracing::info!(to = %to, subject = %subject, "Draft saved (local)");
                // Store as simple log for now — full KV store integration later
                let _ = (to, cc, bcc, subject, body);
                ui.set_email_draft_status("Draft saved".into());

                // Clear status after 3 seconds
                let weak = ui_weak.clone();
                let t = Timer::default();
                t.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_email_draft_status("".into());
                    }
                });
                std::mem::forget(t);
            }
        });
    }

    // ── V41: AI intent classify ──
    {
        let bridge = bridge.clone();
        let ui_weak = ui.as_weak();
        ui.on_email_ai_classify(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let detail = ui.get_email_detail();
                if detail.id <= 0 { return; }
                ui.set_email_ai_working(true);

                let body_str = detail.body.to_string();
                let body_preview = truncate_utf8(&body_str, 500);
                let prompt = format!(
                    "Classify this email intent as exactly one of: action-needed, fyi, meeting, newsletter, personal. Reply with just the label, nothing else.\n\nSubject: {}\n\n{}\n",
                    detail.subject,
                    body_preview,
                );
                let token_rx = bridge.send_message(prompt);
                let weak = ui_weak.clone();
                let collected = Rc::new(RefCell::new(String::new()));
                let ci = collected.clone();
                let t = Timer::default();
                t.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                    while let Ok(tok) = token_rx.try_recv() {
                        if tok == "__DONE__" {
                            if let Some(ui) = weak.upgrade() {
                                let label = ci.borrow().trim().to_lowercase();
                                // Normalize to known labels
                                let intent = if label.contains("action") { "action-needed" }
                                    else if label.contains("fyi") { "fyi" }
                                    else if label.contains("meeting") { "meeting" }
                                    else if label.contains("newsletter") { "newsletter" }
                                    else if label.contains("personal") { "personal" }
                                    else { &label };
                                ui.set_email_ai_intent(intent.into());
                                ui.set_email_ai_working(false);
                                tracing::info!(intent = %intent, "Email AI classified");
                            }
                            return;
                        }
                        if tok == "__REPLACE__" { ci.borrow_mut().clear(); continue; }
                        if tok.starts_with("__") && tok.ends_with("__") { continue; }
                        ci.borrow_mut().push_str(&tok);
                    }
                });
                std::mem::forget(t);
            }
        });
    }

    // ── V41: Download attachment (stub) ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_download_attachment(move |idx| {
            tracing::info!(attachment_idx = idx, "Attachment download requested");
            if let Some(ui) = ui_weak.upgrade() {
                let attachments_model = ui.get_email_attachments();
                if let Some(row) = attachments_model.row_data(idx as usize) {
                    let mut updated = row;
                    updated.is_downloaded = true;
                    // Re-create the model with updated item
                    let count = attachments_model.row_count();
                    let mut items = Vec::with_capacity(count);
                    for i in 0..count {
                        if i == idx as usize {
                            items.push(updated.clone());
                        } else if let Some(item) = attachments_model.row_data(i) {
                            items.push(item);
                        }
                    }
                    ui.set_email_attachments(ModelRc::from(Rc::new(VecModel::from(items))));
                    tracing::info!(name = %updated.name, "Attachment marked as downloaded");
                }
            }
        });
    }

    // ── V41: Preview attachment (stub) ──
    {
        ui.on_email_preview_attachment(move |idx| {
            tracing::info!(attachment_idx = idx, "Attachment preview requested");
        });
    }

    // ── V41: Set signature ──
    {
        let ui_weak = ui.as_weak();
        ui.on_email_set_signature(move |idx| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_email_active_signature(idx);
                let sigs = ui.get_email_signatures();
                if let Some(sig) = sigs.row_data(idx as usize) {
                    let content = sig.content.to_string();
                    if !content.is_empty() {
                        let current_body = ui.get_email_compose_body().to_string();
                        // Strip any existing signature (delimited by \n-- \n)
                        let base = if let Some(pos) = current_body.find("\n-- \n") {
                            current_body[..pos].to_string()
                        } else {
                            current_body
                        };
                        let new_body = format!("{}\n-- \n{}", base.trim_end(), content);
                        ui.set_email_compose_body(new_body.into());
                    }
                    tracing::debug!(signature = %sig.name, "Signature applied");
                }
            }
        });
    }

    // ── V41: Initialize default signatures ──
    {
        let default_sigs = vec![
            EmailSignatureData {
                name: "Default".into(),
                content: "".into(),
                is_default: true,
            },
            EmailSignatureData {
                name: "Professional".into(),
                content: "Best regards".into(),
                is_default: false,
            },
            EmailSignatureData {
                name: "Casual".into(),
                content: "Cheers!".into(),
                is_default: false,
            },
        ];
        ui.set_email_signatures(ModelRc::from(Rc::new(VecModel::from(default_sigs))));
    }
}

/// Fetch emails from IMAP on background thread, then populate UI model.
fn fetch_and_populate(
    info: AccountInfo,
    folder: &str,
    ui_weak: slint::Weak<App>,
    model: Rc<VecModel<EmailListItem>>,
    cache: Rc<RefCell<Vec<FetchedEmail>>>,
    folders_model: Rc<VecModel<EmailFolderData>>,
) {
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_email_is_loading(true);
        ui.set_email_sync_status("Syncing...".into());
    }

    let folder_str = folder.to_string();
    let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<FetchedEmail>, String>>();

    std::thread::spawn(move || {
        // If OAuth, get a valid token (refresh if needed)
        let oauth_token = if info.is_oauth() {
            match info.get_valid_token() {
                Ok((tok, _)) => Some(tok),
                Err(e) => { let _ = tx.send(Err(e)); return; }
            }
        } else {
            None
        };
        let result = imap_fetch_emails(
            &info.imap_server, info.imap_port,
            &info.email, &info.password,
            &folder_str, 50,
            oauth_token.as_deref(),
        );
        let _ = tx.send(result);
    });

    let weak = ui_weak.clone();
    let t = Timer::default();
    t.start(TimerMode::Repeated, Duration::from_millis(200), move || {
        if let Ok(result) = rx.try_recv() {
            if let Some(ui) = weak.upgrade() {
                ui.set_email_is_loading(false);
                match result {
                    Ok(emails) => {
                        // Update unread count for selected folder + status bar
                        let total = emails.len() as i32;
                        let unread = emails.iter().filter(|e| !e.is_read).count() as i32;
                        ui.set_email_folder_total(total);
                        ui.set_email_folder_unread(unread);
                        ui.set_email_sync_status("Synced".into());
                        for i in 0..folders_model.row_count() {
                            if let Some(mut f) = folders_model.row_data(i) {
                                if f.is_selected {
                                    f.unread_count = unread;
                                    f.total_count = total;
                                    folders_model.set_row_data(i, f);
                                }
                            }
                        }

                        // Populate list model
                        let items: Vec<EmailListItem> = emails.iter().map(|e| {
                            let initial = if !e.from_name.is_empty() {
                                e.from_name.chars().next().unwrap_or('?').to_uppercase().to_string()
                            } else if !e.from_addr.is_empty() {
                                e.from_addr.chars().next().unwrap_or('?').to_uppercase().to_string()
                            } else {
                                "?".to_string()
                            };
                            // Generate a stable color from the sender name/addr
                            let color_seed: u32 = e.from_addr.bytes().map(|b| b as u32).sum();
                            let avatar_color = match color_seed % 6 {
                                0 => slint::Color::from_rgb_u8(90, 200, 212),   // cyan
                                1 => slint::Color::from_rgb_u8(212, 165, 116),  // amber
                                2 => slint::Color::from_rgb_u8(180, 132, 224),  // purple
                                3 => slint::Color::from_rgb_u8(108, 212, 128),  // green
                                4 => slint::Color::from_rgb_u8(232, 120, 152),  // pink
                                _ => slint::Color::from_rgb_u8(74, 184, 240),   // blue
                            };
                            EmailListItem {
                                id: e.id,
                                from_name: e.from_name.clone().into(),
                                from_addr: e.from_addr.clone().into(),
                                subject: e.subject.clone().into(),
                                preview: e.preview.clone().into(),
                                date_text: e.date_text.clone().into(),
                                is_read: e.is_read,
                                is_flagged: false,
                                is_selected: false,
                                has_attachment: false,
                                thread_count: 0,
                                thread_id: "".into(),
                                avatar_initial: initial.into(),
                                avatar_color,
                            }
                        }).collect();

                        // Clear and repopulate
                        while model.row_count() > 0 { model.remove(0); }
                        for item in items { model.push(item); }

                        // Cache for detail view
                        *cache.borrow_mut() = emails;

                        tracing::info!(count = model.row_count(), "Emails loaded");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "IMAP fetch failed");
                        ui.set_email_sync_status("Sync failed".into());
                    }
                }
            }
        }
    });
    std::mem::forget(t);
}
