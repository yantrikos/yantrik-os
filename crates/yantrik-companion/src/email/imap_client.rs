//! IMAP client — fetch emails via IMAP over TLS.

use std::net::TcpStream;
use imap::Session;
use crate::config::EmailAccountConfig;
use super::resolve_imap_server;

/// Minimal email header info from IMAP FETCH.
#[derive(Debug, Clone)]
pub struct EmailHeader {
    pub uid: u32,
    pub from_addr: String,
    pub from_name: String,
    pub to_addr: String,
    pub subject: String,
    pub date_ts: f64,
    pub is_read: bool,
    pub message_id: String,
    pub in_reply_to: String,
}

/// Connect to IMAP server with TLS.
/// Supports both password and XOAUTH2 authentication.
pub fn connect(account: &EmailAccountConfig) -> Result<Session<native_tls::TlsStream<TcpStream>>, String> {
    let (server, port) = resolve_imap_server(account);
    if server.is_empty() {
        return Err("No IMAP server configured".to_string());
    }

    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS error: {}", e))?;

    let client = imap::connect((&*server, port), &server, &tls)
        .map_err(|e| format!("IMAP connect failed: {}", e))?;

    let is_oauth = account.auth_method.as_deref() == Some("oauth2");

    let session = if is_oauth {
        let token = account.oauth_access_token.as_deref().unwrap_or("");
        if token.is_empty() {
            return Err("OAuth2 account has no access token".to_string());
        }
        // XOAUTH2 SASL: "user=<email>\x01auth=Bearer <token>\x01\x01"
        let auth_string = format!("user={}\x01auth=Bearer {}\x01\x01", account.email, token);
        client
            .authenticate("XOAUTH2", &XOAuth2Authenticator(auth_string))
            .map_err(|e| format!("IMAP XOAUTH2 login failed: {}", e.0))?
    } else {
        client
            .login(&account.email, &account.password)
            .map_err(|e| format!("IMAP login failed: {}", e.0))?
    };

    Ok(session)
}

/// XOAUTH2 SASL authenticator for IMAP.
struct XOAuth2Authenticator(String);

impl imap::Authenticator for XOAuth2Authenticator {
    type Response = String;
    fn process(&self, _challenge: &[u8]) -> Self::Response {
        self.0.clone()
    }
}

/// List available folders/mailboxes.
pub fn list_folders(session: &mut Session<native_tls::TlsStream<TcpStream>>) -> Vec<String> {
    match session.list(Some(""), Some("*")) {
        Ok(listing) => listing.iter().map(|m| m.name().to_string()).collect(),
        Err(_) => vec!["INBOX".to_string()],
    }
}

/// Fetch email headers from a folder since a given UID.
pub fn fetch_headers(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    since_uid: u32,
) -> Result<Vec<EmailHeader>, String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;

    let query = if since_uid > 0 {
        format!("{}:*", since_uid + 1)
    } else {
        // First sync — get all emails
        "1:*".to_string()
    };

    let messages = session
        .uid_fetch(&query, "(UID FLAGS ENVELOPE)")
        .map_err(|e| format!("FETCH failed: {}", e))?;

    let mut headers = Vec::new();
    for msg in messages.iter() {
        let uid = match msg.uid {
            Some(u) => u,
            None => continue,
        };

        // Skip if we already have this UID
        if uid <= since_uid && since_uid > 0 {
            continue;
        }

        let flags = msg.flags();
        let is_read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));

        let envelope = match msg.envelope() {
            Some(e) => e,
            None => continue,
        };

        let subject = envelope.subject
            .map(|s| decode_bytes(s))
            .unwrap_or_default();

        // Extract from address
        let (from_name, from_addr) = envelope.from.as_ref()
            .and_then(|addrs| addrs.first())
            .map(|addr| {
                let name = addr.name.map(|n| decode_bytes(n)).unwrap_or_default();
                let mailbox = addr.mailbox.map(|m| decode_bytes(m)).unwrap_or_default();
                let host = addr.host.map(|h| decode_bytes(h)).unwrap_or_default();
                let email = if !mailbox.is_empty() && !host.is_empty() {
                    format!("{}@{}", mailbox, host)
                } else {
                    mailbox
                };
                (name, email)
            })
            .unwrap_or_default();

        // Extract to address
        let to_addr = envelope.to.as_ref()
            .and_then(|addrs| addrs.first())
            .map(|addr| {
                let mailbox = addr.mailbox.map(|m| decode_bytes(m)).unwrap_or_default();
                let host = addr.host.map(|h| decode_bytes(h)).unwrap_or_default();
                if !mailbox.is_empty() && !host.is_empty() {
                    format!("{}@{}", mailbox, host)
                } else {
                    mailbox
                }
            })
            .unwrap_or_default();

        let date_ts = envelope.date
            .map(|d| parse_imap_date(&decode_bytes(d)))
            .unwrap_or(0.0);

        let message_id = envelope.message_id
            .map(|m| decode_bytes(m))
            .unwrap_or_default();

        let in_reply_to = envelope.in_reply_to
            .map(|m| decode_bytes(m))
            .unwrap_or_default();

        headers.push(EmailHeader {
            uid,
            from_addr,
            from_name,
            to_addr,
            subject,
            date_ts,
            is_read,
            message_id,
            in_reply_to,
        });
    }

    // Limit to last 50 on first sync
    if since_uid == 0 && headers.len() > 50 {
        headers = headers.split_off(headers.len() - 50);
    }

    Ok(headers)
}

/// Fetch full email body by UID — returns plain text.
pub fn fetch_body(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uid: u32,
) -> Result<String, String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;

    let messages = session
        .uid_fetch(uid.to_string(), "BODY[]")
        .map_err(|e| format!("FETCH body failed: {}", e))?;

    let msg = messages.iter().next()
        .ok_or("Message not found")?;

    let body = msg.body()
        .ok_or("No body in message")?;

    // Parse MIME and extract text
    match mailparse::parse_mail(body) {
        Ok(parsed) => Ok(extract_text_from_mime(&parsed)),
        Err(e) => Err(format!("MIME parse error: {}", e)),
    }
}

/// Mark a message as read.
pub fn mark_read(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uid: u32,
) -> Result<(), String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
    session.uid_store(uid.to_string(), "+FLAGS (\\Seen)")
        .map_err(|e| format!("STORE failed: {}", e))?;
    Ok(())
}

/// Mark a message as flagged.
pub fn mark_flagged(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uid: u32,
) -> Result<(), String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
    session.uid_store(uid.to_string(), "+FLAGS (\\Flagged)")
        .map_err(|e| format!("STORE failed: {}", e))?;
    Ok(())
}

/// Mark a message as unread (remove \Seen flag).
pub fn mark_unread(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uid: u32,
) -> Result<(), String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
    session.uid_store(uid.to_string(), "-FLAGS (\\Seen)")
        .map_err(|e| format!("STORE failed: {}", e))?;
    Ok(())
}

/// Remove flagged status from a message.
pub fn unflag(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uid: u32,
) -> Result<(), String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
    session.uid_store(uid.to_string(), "-FLAGS (\\Flagged)")
        .map_err(|e| format!("STORE failed: {}", e))?;
    Ok(())
}

/// Move a message to another folder (COPY + DELETE from source).
pub fn move_message(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uid: u32,
    dest_folder: &str,
) -> Result<(), String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
    session.uid_copy(uid.to_string(), dest_folder)
        .map_err(|e| format!("COPY failed: {}", e))?;
    session.uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
        .map_err(|e| format!("STORE failed: {}", e))?;
    session.expunge().map_err(|e| format!("EXPUNGE failed: {}", e))?;
    Ok(())
}

/// Mark multiple messages as read by UIDs.
pub fn mark_read_bulk(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uids: &[u32],
) -> Result<usize, String> {
    if uids.is_empty() {
        return Ok(0);
    }
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
    let uid_set: String = uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
    session.uid_store(&uid_set, "+FLAGS (\\Seen)")
        .map_err(|e| format!("STORE failed: {}", e))?;
    Ok(uids.len())
}

/// Delete a message (move to Trash).
pub fn delete_message(
    session: &mut Session<native_tls::TlsStream<TcpStream>>,
    folder: &str,
    uid: u32,
) -> Result<(), String> {
    session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
    session.uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
        .map_err(|e| format!("STORE failed: {}", e))?;
    session.expunge().map_err(|e| format!("EXPUNGE failed: {}", e))?;
    Ok(())
}

// -- Helpers --

/// Decode a byte slice to a UTF-8 string (lossy).
fn decode_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn parse_imap_date(date_str: &str) -> f64 {
    // Try to parse RFC 2822 date
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(date_str) {
        return dt.timestamp() as f64;
    }
    // Fallback: try common IMAP date formats
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(date_str, "%d-%b-%Y %H:%M:%S %z") {
        return dt.and_utc().timestamp() as f64;
    }
    0.0
}

fn extract_text_from_mime(parsed: &mailparse::ParsedMail) -> String {
    // Try to find text/plain part first
    if parsed.subparts.is_empty() {
        let ctype = parsed.ctype.mimetype.to_lowercase();
        if let Ok(body) = parsed.get_body() {
            if ctype.contains("text/plain") {
                return body;
            } else if ctype.contains("text/html") {
                return html2text::from_read(body.as_bytes(), 80);
            }
        }
        return String::new();
    }

    // Multi-part: prefer text/plain, fall back to text/html
    let mut plain_text = String::new();
    let mut html_text = String::new();

    for part in &parsed.subparts {
        let ctype = part.ctype.mimetype.to_lowercase();
        if let Ok(body) = part.get_body() {
            if ctype.contains("text/plain") && plain_text.is_empty() {
                plain_text = body;
            } else if ctype.contains("text/html") && html_text.is_empty() {
                html_text = body;
            }
        }
        // Recurse into nested multipart
        if !part.subparts.is_empty() {
            let nested = extract_text_from_mime(part);
            if !nested.is_empty() && plain_text.is_empty() {
                plain_text = nested;
            }
        }
    }

    if !plain_text.is_empty() {
        plain_text
    } else if !html_text.is_empty() {
        html2text::from_read(html_text.as_bytes(), 80)
    } else {
        String::new()
    }
}
