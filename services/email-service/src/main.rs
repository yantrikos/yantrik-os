//! Email service — IMAP fetch, SMTP send, folder management via JSON-RPC.
//!
//! Reads account configuration from environment or config file.
//! Supports Gmail, Outlook, Yahoo, iCloud, and custom IMAP/SMTP servers.
//!
//! Methods:
//!   email.list_folders    { account_id }                           → Vec<EmailFolder>
//!   email.list_messages   { account_id, folder, page?, per_page? } → Vec<EmailSummary>
//!   email.get_message     { account_id, message_id }               → EmailDetail
//!   email.send_message    { account_id, to, subject, body, ... }   → ()
//!   email.mark_read       { account_id, message_id, read }         → ()
//!   email.mark_starred    { account_id, message_id, starred }      → ()
//!   email.move_message    { account_id, message_id, target_folder } → ()
//!   email.delete_message  { account_id, message_id }               → ()
//!   email.search          { account_id, query }                    → Vec<EmailSummary>

use std::sync::Arc;

use yantrik_ipc_contracts::email::*;
use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("email_service=info".parse().unwrap()),
        )
        .init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        let handler = Arc::new(EmailHandler::new());
        let addr = RpcServer::default_address("email");
        let server = RpcServer::new(&addr);
        tracing::info!("Starting email service");
        if let Err(e) = server.serve(handler).await {
            tracing::error!(error = %e, "Email service failed");
        }
    });
}

// ── Account configuration ────────────────────────────────────────────

#[derive(Clone)]
struct AccountConfig {
    id: String,
    email: String,
    password: String,
    imap_server: String,
    imap_port: u16,
    smtp_server: String,
    smtp_port: u16,
    use_oauth: bool,
    oauth_token: Option<String>,
}

struct EmailHandler {
    accounts: std::sync::Mutex<Vec<AccountConfig>>,
}

impl EmailHandler {
    fn new() -> Self {
        // Load accounts from config file or environment
        let accounts = load_accounts();
        Self {
            accounts: std::sync::Mutex::new(accounts),
        }
    }

    fn get_account(&self, account_id: &str) -> Result<AccountConfig, ServiceError> {
        let accounts = self.accounts.lock().unwrap();
        accounts
            .iter()
            .find(|a| a.id == account_id)
            .cloned()
            .ok_or_else(|| ServiceError {
                code: -32000,
                message: format!("Unknown account: {account_id}"),
            })
    }
}

impl ServiceHandler for EmailHandler {
    fn service_id(&self) -> &str {
        "email"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        let account_id = params["account_id"]
            .as_str()
            .unwrap_or("default");

        match method {
            "email.list_folders" => {
                let account = self.get_account(account_id)?;
                let folders = imap_list_folders(&account)?;
                Ok(serde_json::to_value(folders).unwrap())
            }
            "email.list_messages" => {
                let account = self.get_account(account_id)?;
                let folder = params["folder"].as_str().unwrap_or("INBOX");
                let page = params["page"].as_u64().unwrap_or(1) as u32;
                let per_page = params["per_page"].as_u64().unwrap_or(20) as u32;
                let messages = imap_list_messages(&account, folder, page, per_page)?;
                Ok(serde_json::to_value(messages).unwrap())
            }
            "email.get_message" => {
                let account = self.get_account(account_id)?;
                let message_id = require_str(&params, "message_id")?;
                let detail = imap_get_message(&account, message_id)?;
                Ok(serde_json::to_value(detail).unwrap())
            }
            "email.send_message" => {
                let account = self.get_account(account_id)?;
                let compose: ComposeRequest =
                    serde_json::from_value(params.clone()).map_err(|e| ServiceError {
                        code: -32602,
                        message: format!("Invalid compose params: {e}"),
                    })?;
                smtp_send(&account, &compose)?;
                Ok(serde_json::json!(null))
            }
            "email.mark_read" => {
                let account = self.get_account(account_id)?;
                let message_id = require_str(&params, "message_id")?;
                let read = params["read"].as_bool().unwrap_or(true);
                imap_mark_read(&account, message_id, read)?;
                Ok(serde_json::json!(null))
            }
            "email.mark_starred" => {
                let account = self.get_account(account_id)?;
                let message_id = require_str(&params, "message_id")?;
                let starred = params["starred"].as_bool().unwrap_or(true);
                imap_mark_starred(&account, message_id, starred)?;
                Ok(serde_json::json!(null))
            }
            "email.move_message" => {
                let account = self.get_account(account_id)?;
                let message_id = require_str(&params, "message_id")?;
                let target = require_str(&params, "target_folder")?;
                imap_move_message(&account, message_id, target)?;
                Ok(serde_json::json!(null))
            }
            "email.delete_message" => {
                let account = self.get_account(account_id)?;
                let message_id = require_str(&params, "message_id")?;
                imap_delete_message(&account, message_id)?;
                Ok(serde_json::json!(null))
            }
            "email.search" => {
                let account = self.get_account(account_id)?;
                let query = require_str(&params, "query")?;
                let results = imap_search(&account, query)?;
                Ok(serde_json::to_value(results).unwrap())
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
}

fn require_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ServiceError> {
    params[key].as_str().ok_or_else(|| ServiceError {
        code: -32602,
        message: format!("Missing '{key}' parameter"),
    })
}

// ── Account loading ──────────────────────────────────────────────────

fn load_accounts() -> Vec<AccountConfig> {
    // Try config file first
    let config_path = std::env::var("YANTRIK_EMAIL_CONFIG")
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            format!("{home}/.config/yantrik/email.json")
        });

    if let Ok(content) = std::fs::read_to_string(&config_path) {
        if let Ok(accounts) = serde_json::from_str::<Vec<AccountConfigJson>>(&content) {
            return accounts
                .into_iter()
                .map(|a| AccountConfig {
                    id: a.id,
                    email: a.email,
                    password: a.password.unwrap_or_default(),
                    imap_server: a.imap_server,
                    imap_port: a.imap_port.unwrap_or(993),
                    smtp_server: a.smtp_server,
                    smtp_port: a.smtp_port.unwrap_or(587),
                    use_oauth: a.use_oauth.unwrap_or(false),
                    oauth_token: a.oauth_token,
                })
                .collect();
        }
    }

    // Fallback: environment variables for a single account
    if let (Ok(email), Ok(password), Ok(imap)) = (
        std::env::var("YANTRIK_EMAIL"),
        std::env::var("YANTRIK_EMAIL_PASSWORD"),
        std::env::var("YANTRIK_EMAIL_IMAP"),
    ) {
        let smtp = std::env::var("YANTRIK_EMAIL_SMTP")
            .unwrap_or_else(|_| imap.replace("imap.", "smtp."));
        return vec![AccountConfig {
            id: "default".to_string(),
            email,
            password,
            imap_server: imap,
            imap_port: 993,
            smtp_server: smtp,
            smtp_port: 587,
            use_oauth: false,
            oauth_token: None,
        }];
    }

    Vec::new()
}

#[derive(serde::Deserialize)]
struct AccountConfigJson {
    id: String,
    email: String,
    password: Option<String>,
    imap_server: String,
    imap_port: Option<u16>,
    smtp_server: String,
    smtp_port: Option<u16>,
    use_oauth: Option<bool>,
    oauth_token: Option<String>,
}

// ── IMAP operations ──────────────────────────────────────────────────

fn imap_connect(account: &AccountConfig) -> Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>, ServiceError> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("TLS error: {e}"),
        })?;

    let client = imap::connect(
        (account.imap_server.as_str(), account.imap_port),
        &account.imap_server,
        &tls,
    )
    .map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP connect failed: {e}"),
    })?;

    let session = if account.use_oauth {
        let token = account.oauth_token.as_deref().unwrap_or("");
        let auth_string = format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            account.email, token
        );
        client
            .authenticate("XOAUTH2", &XOAuth2Authenticator(auth_string))
            .map_err(|e| ServiceError {
                code: -32000,
                message: format!("IMAP OAuth2 auth failed: {}", e.0),
            })?
    } else {
        client
            .login(&account.email, &account.password)
            .map_err(|e| ServiceError {
                code: -32000,
                message: format!("IMAP login failed: {}", e.0),
            })?
    };

    Ok(session)
}

struct XOAuth2Authenticator(String);

impl imap::Authenticator for XOAuth2Authenticator {
    type Response = String;
    fn process(&self, _data: &[u8]) -> Self::Response {
        self.0.clone()
    }
}

fn imap_list_folders(account: &AccountConfig) -> Result<Vec<EmailFolder>, ServiceError> {
    let mut session = imap_connect(account)?;

    let folders = session
        .list(None, Some("*"))
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP LIST failed: {e}"),
        })?;

    let mut result = Vec::new();
    for folder in folders.iter() {
        let name = folder.name().to_string();

        // Get unread/total counts
        let (unread, total) = match session.examine(&name) {
            Ok(mailbox) => {
                let total = mailbox.exists as i32;
                // UNSEEN requires STATUS command
                let unread = session
                    .status(&name, "(UNSEEN)")
                    .ok()
                    .and_then(|s| s.unseen)
                    .unwrap_or(0) as i32;
                (unread, total)
            }
            Err(_) => (0, 0),
        };

        result.push(EmailFolder {
            name,
            unread_count: unread,
            total_count: total,
        });
    }

    let _ = session.logout();
    Ok(result)
}

fn imap_list_messages(
    account: &AccountConfig,
    folder: &str,
    page: u32,
    per_page: u32,
) -> Result<Vec<EmailSummary>, ServiceError> {
    let mut session = imap_connect(account)?;

    session.select(folder).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {folder} failed: {e}"),
    })?;

    // Fetch recent messages (by sequence number, newest first)
    let total = session.select(folder).map(|m| m.exists).unwrap_or(0);
    if total == 0 {
        let _ = session.logout();
        return Ok(Vec::new());
    }

    let start = total.saturating_sub((page * per_page) as u32);
    let end = total.saturating_sub(((page - 1) * per_page) as u32);
    if start >= end {
        let _ = session.logout();
        return Ok(Vec::new());
    }

    let range = format!("{}:{}", start.max(1), end);
    let messages = session
        .fetch(&range, "(UID FLAGS ENVELOPE BODYSTRUCTURE)")
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP FETCH failed: {e}"),
        })?;

    let mut result = Vec::new();
    for msg in messages.iter() {
        let uid = msg.uid.unwrap_or(0);
        let envelope = match msg.envelope() {
            Some(e) => e,
            None => continue,
        };

        let from = envelope
            .from
            .as_ref()
            .and_then(|addrs| addrs.first())
            .map(|a| {
                let name = a.name.as_ref().map(|n| {
                    String::from_utf8_lossy(n).to_string()
                });
                let mailbox = a.mailbox.as_ref().map(|m| String::from_utf8_lossy(m).to_string()).unwrap_or_default();
                let host = a.host.as_ref().map(|h| String::from_utf8_lossy(h).to_string()).unwrap_or_default();
                match name {
                    Some(n) if !n.is_empty() => n,
                    _ => format!("{mailbox}@{host}"),
                }
            })
            .unwrap_or_else(|| "Unknown".to_string());

        let subject = envelope
            .subject
            .as_ref()
            .map(|s| String::from_utf8_lossy(s).to_string())
            .unwrap_or_else(|| "(no subject)".to_string());

        let date = envelope
            .date
            .as_ref()
            .map(|d| String::from_utf8_lossy(d).to_string())
            .unwrap_or_default();

        let flags = msg.flags();
        let is_read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));
        let is_starred = flags.iter().any(|f| matches!(f, imap::types::Flag::Flagged));

        result.push(EmailSummary {
            id: uid.to_string(),
            from,
            to: Vec::new(),
            subject,
            snippet: String::new(),
            date,
            is_read,
            is_starred,
            has_attachments: false,
            folder: folder.to_string(),
            thread_id: None,
        });
    }

    result.reverse(); // newest first
    let _ = session.logout();
    Ok(result)
}

fn imap_get_message(
    account: &AccountConfig,
    message_id: &str,
) -> Result<EmailDetail, ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    session.select("INBOX").ok();

    let messages = session
        .uid_fetch(uid.to_string(), "(RFC822 FLAGS)")
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP UID FETCH failed: {e}"),
        })?;

    let msg = messages.first().ok_or_else(|| ServiceError {
        code: -32000,
        message: format!("Message not found: {message_id}"),
    })?;

    let body = msg.body().unwrap_or(&[]);
    let parsed = mailparse::parse_mail(body).map_err(|e| ServiceError {
        code: -32000,
        message: format!("Mail parse error: {e}"),
    })?;

    let from = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "From")
        .map(|h| h.get_value())
        .unwrap_or_default();

    let to: Vec<String> = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "To")
        .map(|h| h.get_value().split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let cc: Vec<String> = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "Cc")
        .map(|h| h.get_value().split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let subject = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "Subject")
        .map(|h| h.get_value())
        .unwrap_or_default();

    let date = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "Date")
        .map(|h| h.get_value())
        .unwrap_or_default();

    // Extract body text/html
    let mut body_text = String::new();
    let mut body_html = String::new();
    let mut attachments = Vec::new();

    extract_parts(&parsed, &mut body_text, &mut body_html, &mut attachments);

    // If only HTML, convert to text
    if body_text.is_empty() && !body_html.is_empty() {
        body_text = html2text::from_read(body_html.as_bytes(), 80);
    }

    let _ = session.logout();

    Ok(EmailDetail {
        id: message_id.to_string(),
        from,
        to,
        cc,
        bcc: Vec::new(),
        subject,
        body_html,
        body_text,
        date,
        attachments,
        thread_messages: Vec::new(),
    })
}

fn extract_parts(
    mail: &mailparse::ParsedMail,
    body_text: &mut String,
    body_html: &mut String,
    attachments: &mut Vec<EmailAttachment>,
) {
    let content_type = mail.ctype.mimetype.as_str();

    if mail.subparts.is_empty() {
        match content_type {
            "text/plain" => {
                if body_text.is_empty() {
                    *body_text = mail.get_body().unwrap_or_default();
                }
            }
            "text/html" => {
                if body_html.is_empty() {
                    *body_html = mail.get_body().unwrap_or_default();
                }
            }
            _ => {
                // Attachment
                let filename = mail
                    .ctype
                    .params
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| "attachment".to_string());
                let size = mail.get_body_raw().map(|b| b.len() as u64).unwrap_or(0);
                attachments.push(EmailAttachment {
                    filename,
                    mime_type: content_type.to_string(),
                    size_bytes: size,
                });
            }
        }
    } else {
        for part in &mail.subparts {
            extract_parts(part, body_text, body_html, attachments);
        }
    }
}

fn smtp_send(account: &AccountConfig, compose: &ComposeRequest) -> Result<(), ServiceError> {
    use lettre::{Message, SmtpTransport, Transport};
    use lettre::transport::smtp::authentication::Credentials;

    let mut email_builder = Message::builder()
        .from(account.email.parse().map_err(|e| ServiceError {
            code: -32000,
            message: format!("Invalid from address: {e}"),
        })?)
        .subject(&compose.subject);

    for to in &compose.to {
        email_builder = email_builder.to(to.parse().map_err(|e| ServiceError {
            code: -32000,
            message: format!("Invalid to address '{to}': {e}"),
        })?);
    }
    for cc in &compose.cc {
        email_builder = email_builder.cc(cc.parse().map_err(|e| ServiceError {
            code: -32000,
            message: format!("Invalid cc address '{cc}': {e}"),
        })?);
    }

    let body = if let Some(ref sig) = compose.signature {
        format!("{}\n\n--\n{}", compose.body, sig)
    } else {
        compose.body.clone()
    };

    let email = email_builder
        .body(body)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("Failed to build email: {e}"),
        })?;

    let creds = Credentials::new(account.email.clone(), account.password.clone());

    let mailer = SmtpTransport::relay(&account.smtp_server)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("SMTP relay error: {e}"),
        })?
        .credentials(creds)
        .build();

    mailer.send(&email).map_err(|e| ServiceError {
        code: -32000,
        message: format!("SMTP send failed: {e}"),
    })?;

    tracing::info!(to = ?compose.to, subject = %compose.subject, "Email sent");
    Ok(())
}

fn imap_mark_read(
    account: &AccountConfig,
    message_id: &str,
    read: bool,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    session.select("INBOX").ok();

    let flag = "+FLAGS (\\Seen)";
    let unflag = "-FLAGS (\\Seen)";
    session
        .uid_store(uid.to_string(), if read { flag } else { unflag })
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP STORE failed: {e}"),
        })?;

    let _ = session.logout();
    Ok(())
}

fn imap_mark_starred(
    account: &AccountConfig,
    message_id: &str,
    starred: bool,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    session.select("INBOX").ok();

    let flag = "+FLAGS (\\Flagged)";
    let unflag = "-FLAGS (\\Flagged)";
    session
        .uid_store(uid.to_string(), if starred { flag } else { unflag })
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP STORE failed: {e}"),
        })?;

    let _ = session.logout();
    Ok(())
}

fn imap_move_message(
    account: &AccountConfig,
    message_id: &str,
    target_folder: &str,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    session.select("INBOX").ok();

    session
        .uid_mv(uid.to_string(), target_folder)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP MOVE failed: {e}"),
        })?;

    let _ = session.logout();
    Ok(())
}

fn imap_delete_message(
    account: &AccountConfig,
    message_id: &str,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    session.select("INBOX").ok();

    session
        .uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP delete flag failed: {e}"),
        })?;
    session.expunge().map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP EXPUNGE failed: {e}"),
    })?;

    let _ = session.logout();
    Ok(())
}

fn imap_search(
    account: &AccountConfig,
    query: &str,
) -> Result<Vec<EmailSummary>, ServiceError> {
    let mut session = imap_connect(account)?;
    session.select("INBOX").ok();

    // IMAP search by subject or from
    let search_query = format!("OR SUBJECT \"{}\" FROM \"{}\"", query, query);
    let uids = session
        .search(&search_query)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP SEARCH failed: {e}"),
        })?;

    if uids.is_empty() {
        let _ = session.logout();
        return Ok(Vec::new());
    }

    // Fetch the found messages (limit to 50)
    let mut uid_vec: Vec<u32> = uids.into_iter().collect();
    uid_vec.sort_unstable();
    uid_vec.reverse();
    uid_vec.truncate(50);
    let uid_list: Vec<String> = uid_vec.iter().map(|u| u.to_string()).collect();
    let uid_range = uid_list.join(",");

    let messages = session
        .fetch(&uid_range, "(UID FLAGS ENVELOPE)")
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP FETCH failed: {e}"),
        })?;

    let mut result = Vec::new();
    for msg in messages.iter() {
        let uid = msg.uid.unwrap_or(0);
        let envelope = match msg.envelope() {
            Some(e) => e,
            None => continue,
        };

        let from = envelope
            .from
            .as_ref()
            .and_then(|addrs| addrs.first())
            .map(|a| {
                let name = a.name.as_ref().map(|n| String::from_utf8_lossy(n).to_string());
                let mailbox = a.mailbox.as_ref().map(|m| String::from_utf8_lossy(m).to_string()).unwrap_or_default();
                let host = a.host.as_ref().map(|h| String::from_utf8_lossy(h).to_string()).unwrap_or_default();
                match name {
                    Some(n) if !n.is_empty() => n,
                    _ => format!("{mailbox}@{host}"),
                }
            })
            .unwrap_or_else(|| "Unknown".to_string());

        let subject = envelope
            .subject
            .as_ref()
            .map(|s| String::from_utf8_lossy(s).to_string())
            .unwrap_or_default();

        let date = envelope
            .date
            .as_ref()
            .map(|d| String::from_utf8_lossy(d).to_string())
            .unwrap_or_default();

        let flags = msg.flags();
        let is_read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));
        let is_starred = flags.iter().any(|f| matches!(f, imap::types::Flag::Flagged));

        result.push(EmailSummary {
            id: uid.to_string(),
            from,
            to: Vec::new(),
            subject,
            snippet: String::new(),
            date,
            is_read,
            is_starred,
            has_attachments: false,
            folder: "INBOX".to_string(),
            thread_id: None,
        });
    }

    let _ = session.logout();
    Ok(result)
}
