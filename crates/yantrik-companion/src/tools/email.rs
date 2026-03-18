//! Email tools — full email management: check, list, read, send, reply, reply-all,
//! forward, search, mark read/unread, flag/unflag, delete, move, archive.
//!
//! IMAP/SMTP operations are blocking and run on the calling thread.
//! The agent loop already runs tool executions on a worker, so this is fine.

use std::sync::Arc;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::config::EmailAccountConfig;
use crate::email::{self, db, imap_client, smtp_client};

/// Register all email tools (only if accounts are configured).
pub fn register(reg: &mut ToolRegistry, accounts: Vec<EmailAccountConfig>) {
    if accounts.is_empty() {
        return;
    }
    let accounts = Arc::new(accounts);
    reg.register(Box::new(EmailCheckTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailListTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailReadTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailSendTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailReplyTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailReplyAllTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailForwardTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailSearchTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailMarkReadTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailMarkUnreadTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailFlagTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailUnflagTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailDeleteTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailMoveTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailArchiveTool { accounts: accounts.clone() }));
    reg.register(Box::new(EmailListFoldersTool { accounts: accounts.clone() }));
}

// ── Helpers ──

/// Find an account and ensure OAuth token is fresh. Returns a cloned, refreshed account.
fn get_fresh_account(accounts: &[EmailAccountConfig], name_or_idx: Option<&str>) -> Result<(usize, EmailAccountConfig), String> {
    let (idx, account) = find_account(accounts, name_or_idx)?;
    let mut account = account.clone();
    // Try to find config path for persisting refreshed tokens
    let config_path = std::env::var("YANTRIK_CONFIG").ok()
        .or_else(|| {
            let path = "/opt/yantrik/config.yaml";
            if std::path::Path::new(path).exists() { Some(path.to_string()) } else { None }
        });
    if let Err(e) = email::ensure_fresh_token(&mut account, config_path.as_deref()) {
        return Err(format!("Token refresh failed: {}", e));
    }
    Ok((idx, account))
}

/// Find an account by name or index. Returns (index, account).
fn find_account<'a>(accounts: &'a [EmailAccountConfig], name_or_idx: Option<&str>) -> Result<(usize, &'a EmailAccountConfig), String> {
    if accounts.is_empty() {
        return Err("No email accounts configured".to_string());
    }
    match name_or_idx {
        None | Some("") => Ok((0, &accounts[0])),
        Some(s) => {
            // Try by index first
            if let Ok(idx) = s.parse::<usize>() {
                if idx < accounts.len() {
                    return Ok((idx, &accounts[idx]));
                }
            }
            // Try by name (case-insensitive)
            let lower = s.to_lowercase();
            for (i, acc) in accounts.iter().enumerate() {
                if acc.name.to_lowercase().contains(&lower) || acc.email.to_lowercase().contains(&lower) {
                    return Ok((i, acc));
                }
            }
            Err(format!("Account '{}' not found. Available: {}", s,
                accounts.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")))
        }
    }
}

/// Format a cached email as a compact summary line.
fn format_email_summary(email: &db::CachedEmail) -> String {
    let read_marker = if email.is_read { " " } else { "*" };
    let flag_marker = if email.is_flagged { "!" } else { " " };
    let from = if email.from_name.is_empty() { &email.from_addr } else { &email.from_name };
    let subject = if email.subject.len() > 60 {
        format!("{}...", &email.subject[..email.subject.floor_char_boundary(57)])
    } else {
        email.subject.clone()
    };
    let date = format_timestamp(email.date_ts);
    format!("[{}]{}{} {} | {} | {}", email.id, read_marker, flag_marker, from, subject, date)
}

/// Format a unix timestamp into a human-readable date.
fn format_timestamp(ts: f64) -> String {
    if ts <= 0.0 {
        return "unknown".to_string();
    }
    let dt = chrono::DateTime::from_timestamp(ts as i64, 0);
    match dt {
        Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
        None => "unknown".to_string(),
    }
}

// ── email_check ──

struct EmailCheckTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailCheckTool {
    fn name(&self) -> &'static str { "email_check" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_check",
                "description": "Fetch new emails into cache; read-only, no sending",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "account": {
                            "type": "string",
                            "description": "Account name or email address. Omit to use the default account."
                        },
                        "folder": {
                            "type": "string",
                            "description": "Folder to check. Default: INBOX."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let account_name = args.get("account").and_then(|v| v.as_str());
        let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");

        let (_, account) = match get_fresh_account(&self.accounts, account_name) {
            Ok(a) => a,
            Err(e) => return e,
        };

        // Ensure DB tables exist
        db::init_tables(ctx.db.conn());

        // Ensure account record exists
        let account_id = db::ensure_account(ctx.db.conn(), &account.name, &account.email, &account.provider);
        let since_uid = db::get_last_sync_uid(ctx.db.conn(), account_id);

        // Connect and fetch headers
        let mut session = match imap_client::connect(&account) {
            Ok(s) => s,
            Err(e) => return format!("IMAP connection failed: {}", e),
        };

        let headers = match imap_client::fetch_headers(&mut session, folder, since_uid) {
            Ok(h) => h,
            Err(e) => return format!("Fetch failed: {}", e),
        };

        let _ = session.logout();

        // Cache the headers
        let mut max_uid = since_uid;
        let mut new_count = 0u32;
        for hdr in &headers {
            let preview = if hdr.subject.len() > 100 {
                format!("{}...", &hdr.subject[..hdr.subject.floor_char_boundary(97)])
            } else {
                hdr.subject.clone()
            };
            db::upsert_email(
                ctx.db.conn(), account_id, hdr.uid, folder,
                &hdr.from_addr, &hdr.from_name, &hdr.to_addr, &hdr.subject,
                hdr.date_ts, &preview, hdr.is_read, &hdr.message_id, &hdr.in_reply_to,
            );
            if hdr.uid > max_uid {
                max_uid = hdr.uid;
            }
            new_count += 1;
        }

        if max_uid > since_uid {
            db::update_last_sync_uid(ctx.db.conn(), account_id, max_uid);
        }

        // Build response
        let unread = db::count_unread(ctx.db.conn(), account_id, folder);
        let recent = db::list_emails(ctx.db.conn(), account_id, folder, 10);

        let mut result = format!(
            "Synced {} new emails for {} ({}). {} unread in {}.\n\nRecent:\n",
            new_count, account.name, account.email, unread, folder
        );
        for email in &recent {
            result.push_str(&format_email_summary(email));
            result.push('\n');
        }

        result
    }
}

// ── email_list ──

struct EmailListTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailListTool {
    fn name(&self) -> &'static str { "email_list" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_list",
                "description": "List cached emails in a folder; not full bodies",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "account": {
                            "type": "string",
                            "description": "Account name or email. Omit for default."
                        },
                        "folder": {
                            "type": "string",
                            "description": "Folder to list. Default: INBOX."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max emails to return. Default: 20."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let account_name = args.get("account").and_then(|v| v.as_str());
        let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let (_, account) = match get_fresh_account(&self.accounts, account_name) {
            Ok(a) => a,
            Err(e) => return e,
        };

        db::init_tables(ctx.db.conn());
        let account_id = db::ensure_account(ctx.db.conn(), &account.name, &account.email, &account.provider);
        let unread = db::count_unread(ctx.db.conn(), account_id, folder);
        let emails = db::list_emails(ctx.db.conn(), account_id, folder, limit);

        if emails.is_empty() {
            return format!("No emails in {} for {}. Try email_check to sync first.", folder, account.name);
        }

        let mut result = format!("{} ({}) — {} | {} unread\n", account.name, account.email, folder, unread);
        result.push_str("Legend: * = unread, ! = flagged\n\n");
        for email in &emails {
            result.push_str(&format_email_summary(email));
            result.push('\n');
        }
        result
    }
}

// ── email_read ──

struct EmailReadTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailReadTool {
    fn name(&self) -> &'static str { "email_read" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_read",
                "description": "Read full cached email body by ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID from the [ID] prefix in email_list output."
                        }
                    },
                    "required": ["id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required (integer)".to_string(),
        };

        db::init_tables(ctx.db.conn());

        let cached = match db::get_email(ctx.db.conn(), email_id) {
            Some(e) => e,
            None => return format!("Email ID {} not found in cache. Use email_check to sync first.", email_id),
        };

        // If we already have the full body cached, return it
        if !cached.body_full.is_empty() {
            return format_full_email(&cached);
        }

        // Need to fetch from IMAP — find and refresh the account's OAuth token
        let mut account = match self.accounts.iter().find(|a| {
            db::ensure_account(ctx.db.conn(), &a.name, &a.email, &a.provider) == cached.account_id
        }) {
            Some(a) => a.clone(),
            None => return format!("Account for email ID {} not found in config.", email_id),
        };

        // Refresh OAuth token if needed
        let config_path = std::env::var("YANTRIK_CONFIG").ok()
            .or_else(|| {
                let path = "/opt/yantrik/config.yaml";
                if std::path::Path::new(path).exists() { Some(path.to_string()) } else { None }
            });
        if let Err(e) = email::ensure_fresh_token(&mut account, config_path.as_deref()) {
            return format!("Token refresh failed: {}", e);
        }

        let mut session = match imap_client::connect(&account) {
            Ok(s) => s,
            Err(e) => return format!("IMAP connection failed: {}", e),
        };

        let body = match imap_client::fetch_body(&mut session, &cached.folder, cached.uid) {
            Ok(b) => b,
            Err(e) => {
                let _ = session.logout();
                return format!("Fetch body failed: {}", e);
            }
        };

        // Mark as read on the server
        let _ = imap_client::mark_read(&mut session, &cached.folder, cached.uid);
        let _ = session.logout();

        // Cache the body
        db::update_email_body(ctx.db.conn(), email_id, &body);

        let mut updated = cached;
        updated.body_full = body;
        format_full_email(&updated)
    }
}

fn format_full_email(email: &db::CachedEmail) -> String {
    let date = format_timestamp(email.date_ts);
    let from = if email.from_name.is_empty() {
        email.from_addr.clone()
    } else {
        format!("{} <{}>", email.from_name, email.from_addr)
    };
    format!(
        "From: {}\nTo: {}\nSubject: {}\nDate: {}\nID: {}\n\n{}",
        from, email.to_addr, email.subject, date, email.id, email.body_full
    )
}

// ── email_send ──

struct EmailSendTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailSendTool {
    fn name(&self) -> &'static str { "email_send" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_send",
                "description": "Send a new email; not reply or forward",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to": {
                            "type": "string",
                            "description": "Recipient email address."
                        },
                        "subject": {
                            "type": "string",
                            "description": "Email subject line."
                        },
                        "body": {
                            "type": "string",
                            "description": "Email body text (plain text)."
                        },
                        "account": {
                            "type": "string",
                            "description": "Account to send from. Omit for default."
                        }
                    },
                    "required": ["to", "subject", "body"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return "Error: 'to' (recipient email) is required".to_string(),
        };
        let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let account_name = args.get("account").and_then(|v| v.as_str());

        let (_, account) = match get_fresh_account(&self.accounts, account_name) {
            Ok(a) => a,
            Err(e) => return e,
        };

        match smtp_client::send_email(&account, to, subject, body, None) {
            Ok(()) => format!("Email sent to {} from {} — subject: \"{}\"", to, account.email, subject),
            Err(e) => format!("Send failed: {}", e),
        }
    }
}

// ── email_reply ──

struct EmailReplyTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailReplyTool {
    fn name(&self) -> &'static str { "email_reply" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_reply",
                "description": "Reply to sender of an email by ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to reply to (from email_list output)."
                        },
                        "body": {
                            "type": "string",
                            "description": "Reply body text (plain text)."
                        }
                    },
                    "required": ["id", "body"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required (integer)".to_string(),
        };
        let body = match args.get("body").and_then(|v| v.as_str()) {
            Some(b) if !b.is_empty() => b,
            _ => return "Error: 'body' is required".to_string(),
        };

        db::init_tables(ctx.db.conn());

        let cached = match db::get_email(ctx.db.conn(), email_id) {
            Some(e) => e,
            None => return format!("Email ID {} not found. Use email_check to sync.", email_id),
        };

        // Find the account that owns this email and refresh token
        let mut account = match self.accounts.iter().find(|a| {
            db::ensure_account(ctx.db.conn(), &a.name, &a.email, &a.provider) == cached.account_id
        }) {
            Some(a) => a.clone(),
            None => return "Account not found for this email".to_string(),
        };

        let config_path = std::env::var("YANTRIK_CONFIG").ok()
            .or_else(|| {
                let path = "/opt/yantrik/config.yaml";
                if std::path::Path::new(path).exists() { Some(path.to_string()) } else { None }
            });
        if let Err(e) = email::ensure_fresh_token(&mut account, config_path.as_deref()) {
            return format!("Token refresh failed: {}", e);
        }

        let subject = if cached.subject.to_lowercase().starts_with("re:") {
            cached.subject.clone()
        } else {
            format!("Re: {}", cached.subject)
        };

        let reply_to = if cached.message_id.is_empty() { None } else { Some(cached.message_id.as_str()) };

        match smtp_client::send_email(&account, &cached.from_addr, &subject, body, reply_to) {
            Ok(()) => format!("Reply sent to {} — subject: \"{}\"", cached.from_addr, subject),
            Err(e) => format!("Reply failed: {}", e),
        }
    }
}

// ── email_search ──

struct EmailSearchTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailSearchTool {
    fn name(&self) -> &'static str { "email_search" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_search",
                "description": "Search cached emails by keyword; not web search",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search keyword (matches subject, sender name, sender address)."
                        },
                        "account": {
                            "type": "string",
                            "description": "Account to search. Omit for default."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results. Default: 20."
                        }
                    },
                    "required": ["query"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q,
            _ => return "Error: 'query' is required".to_string(),
        };
        let account_name = args.get("account").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let (_, account) = match get_fresh_account(&self.accounts, account_name) {
            Ok(a) => a,
            Err(e) => return e,
        };

        db::init_tables(ctx.db.conn());
        let account_id = db::ensure_account(ctx.db.conn(), &account.name, &account.email, &account.provider);
        let results = db::search_emails(ctx.db.conn(), account_id, query, limit);

        if results.is_empty() {
            return format!("No emails matching '{}' in {} cache.", query, account.name);
        }

        let mut output = format!("Search '{}' in {} — {} results:\n\n", query, account.name, results.len());
        for email in &results {
            output.push_str(&format_email_summary(email));
            output.push('\n');
        }
        output
    }
}

// ── Helper: connect IMAP for a cached email ──

/// Look up a cached email by ID, find its account, refresh OAuth, and connect IMAP.
/// Returns (cached_email, refreshed_account, imap_session).
fn connect_for_email(
    ctx: &ToolContext,
    accounts: &[EmailAccountConfig],
    email_id: i64,
) -> Result<(db::CachedEmail, EmailAccountConfig, imap::Session<native_tls::TlsStream<std::net::TcpStream>>), String> {
    db::init_tables(ctx.db.conn());
    let cached = db::get_email(ctx.db.conn(), email_id)
        .ok_or_else(|| format!("Email ID {} not found. Use email_check to sync.", email_id))?;

    let mut account = accounts.iter()
        .find(|a| db::ensure_account(ctx.db.conn(), &a.name, &a.email, &a.provider) == cached.account_id)
        .cloned()
        .ok_or_else(|| format!("Account for email ID {} not found in config.", email_id))?;

    let config_path = std::env::var("YANTRIK_CONFIG").ok()
        .or_else(|| {
            let path = "/opt/yantrik/config.yaml";
            if std::path::Path::new(path).exists() { Some(path.to_string()) } else { None }
        });
    email::ensure_fresh_token(&mut account, config_path.as_deref())
        .map_err(|e| format!("Token refresh failed: {}", e))?;

    let session = imap_client::connect(&account)
        .map_err(|e| format!("IMAP connection failed: {}", e))?;

    Ok((cached, account, session))
}

// ── email_mark_read ──

struct EmailMarkReadTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailMarkReadTool {
    fn name(&self) -> &'static str { "email_mark_read" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_mark_read",
                "description": "Mark email as read by ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to mark as read. Omit to mark ALL unread in the folder."
                        },
                        "all": {
                            "type": "boolean",
                            "description": "Set to true to mark ALL unread emails in the folder as read."
                        },
                        "account": {
                            "type": "string",
                            "description": "Account name or email. Required when using 'all'."
                        },
                        "folder": {
                            "type": "string",
                            "description": "Folder to mark all as read in. Default: INBOX."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let mark_all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        if mark_all {
            // Mark ALL unread in folder
            let account_name = args.get("account").and_then(|v| v.as_str());
            let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");

            let (_, account) = match get_fresh_account(&self.accounts, account_name) {
                Ok(a) => a,
                Err(e) => return e,
            };

            db::init_tables(ctx.db.conn());
            let account_id = db::ensure_account(ctx.db.conn(), &account.name, &account.email, &account.provider);
            let uids = db::get_unread_uids(ctx.db.conn(), account_id, folder);

            if uids.is_empty() {
                return format!("No unread emails in {} for {}.", folder, account.name);
            }

            // Mark on IMAP server
            let mut session = match imap_client::connect(&account) {
                Ok(s) => s,
                Err(e) => return format!("IMAP connection failed: {}", e),
            };
            match imap_client::mark_read_bulk(&mut session, folder, &uids) {
                Ok(count) => {
                    let _ = session.logout();
                    // Update local cache
                    db::mark_all_read(ctx.db.conn(), account_id, folder);
                    format!("Marked {} emails as read in {} for {}.", count, folder, account.name)
                }
                Err(e) => {
                    let _ = session.logout();
                    format!("Failed to mark as read: {}", e)
                }
            }
        } else {
            // Mark single email
            let email_id = match args.get("id").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => return "Error: provide 'id' for a single email, or 'all: true' for bulk.".to_string(),
            };

            let (cached, _account, mut session) = match connect_for_email(ctx, &self.accounts, email_id) {
                Ok(r) => r,
                Err(e) => return e,
            };

            match imap_client::mark_read(&mut session, &cached.folder, cached.uid) {
                Ok(()) => {
                    let _ = session.logout();
                    db::mark_read(ctx.db.conn(), email_id);
                    format!("Marked email [{}] \"{}\" as read.", email_id, cached.subject)
                }
                Err(e) => {
                    let _ = session.logout();
                    format!("Failed: {}", e)
                }
            }
        }
    }
}

// ── email_mark_unread ──

struct EmailMarkUnreadTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailMarkUnreadTool {
    fn name(&self) -> &'static str { "email_mark_unread" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_mark_unread",
                "description": "Mark email as unread by ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to mark as unread."
                        }
                    },
                    "required": ["id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };

        let (cached, _account, mut session) = match connect_for_email(ctx, &self.accounts, email_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        match imap_client::mark_unread(&mut session, &cached.folder, cached.uid) {
            Ok(()) => {
                let _ = session.logout();
                db::mark_unread(ctx.db.conn(), email_id);
                format!("Marked email [{}] \"{}\" as unread.", email_id, cached.subject)
            }
            Err(e) => {
                let _ = session.logout();
                format!("Failed: {}", e)
            }
        }
    }
}

// ── email_flag ──

struct EmailFlagTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailFlagTool {
    fn name(&self) -> &'static str { "email_flag" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_flag",
                "description": "Star or flag email by ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to flag/star."
                        }
                    },
                    "required": ["id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };

        let (cached, _account, mut session) = match connect_for_email(ctx, &self.accounts, email_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        match imap_client::mark_flagged(&mut session, &cached.folder, cached.uid) {
            Ok(()) => {
                let _ = session.logout();
                db::set_flagged(ctx.db.conn(), email_id, true);
                format!("Flagged email [{}] \"{}\".", email_id, cached.subject)
            }
            Err(e) => {
                let _ = session.logout();
                format!("Failed: {}", e)
            }
        }
    }
}

// ── email_unflag ──

struct EmailUnflagTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailUnflagTool {
    fn name(&self) -> &'static str { "email_unflag" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_unflag",
                "description": "Remove star or flag from email by ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to unflag."
                        }
                    },
                    "required": ["id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };

        let (cached, _account, mut session) = match connect_for_email(ctx, &self.accounts, email_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        match imap_client::unflag(&mut session, &cached.folder, cached.uid) {
            Ok(()) => {
                let _ = session.logout();
                db::set_flagged(ctx.db.conn(), email_id, false);
                format!("Unflagged email [{}] \"{}\".", email_id, cached.subject)
            }
            Err(e) => {
                let _ = session.logout();
                format!("Failed: {}", e)
            }
        }
    }
}

// ── email_delete ──

struct EmailDeleteTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailDeleteTool {
    fn name(&self) -> &'static str { "email_delete" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_delete",
                "description": "Delete email by ID; not archive or move",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to delete."
                        }
                    },
                    "required": ["id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };

        let (cached, _account, mut session) = match connect_for_email(ctx, &self.accounts, email_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        match imap_client::delete_message(&mut session, &cached.folder, cached.uid) {
            Ok(()) => {
                let _ = session.logout();
                db::delete_email(ctx.db.conn(), email_id);
                format!("Deleted email [{}] \"{}\" from {}.", email_id, cached.subject, cached.folder)
            }
            Err(e) => {
                let _ = session.logout();
                format!("Failed: {}", e)
            }
        }
    }
}

// ── email_reply_all ──

struct EmailReplyAllTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailReplyAllTool {
    fn name(&self) -> &'static str { "email_reply_all" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_reply_all",
                "description": "Reply to all recipients on an email",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to reply to (from email_list output)."
                        },
                        "body": {
                            "type": "string",
                            "description": "Reply body text (plain text)."
                        }
                    },
                    "required": ["id", "body"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let body = match args.get("body").and_then(|v| v.as_str()) {
            Some(b) if !b.is_empty() => b,
            _ => return "Error: 'body' is required.".to_string(),
        };

        db::init_tables(ctx.db.conn());
        let cached = match db::get_email(ctx.db.conn(), email_id) {
            Some(e) => e,
            None => return format!("Email ID {} not found.", email_id),
        };

        let mut account = match self.accounts.iter().find(|a| {
            db::ensure_account(ctx.db.conn(), &a.name, &a.email, &a.provider) == cached.account_id
        }) {
            Some(a) => a.clone(),
            None => return "Account not found for this email.".to_string(),
        };

        let config_path = std::env::var("YANTRIK_CONFIG").ok()
            .or_else(|| {
                let path = "/opt/yantrik/config.yaml";
                if std::path::Path::new(path).exists() { Some(path.to_string()) } else { None }
            });
        if let Err(e) = email::ensure_fresh_token(&mut account, config_path.as_deref()) {
            return format!("Token refresh failed: {}", e);
        }

        let subject = if cached.subject.to_lowercase().starts_with("re:") {
            cached.subject.clone()
        } else {
            format!("Re: {}", cached.subject)
        };
        let reply_to = if cached.message_id.is_empty() { None } else { Some(cached.message_id.as_str()) };

        // Collect all recipients: original sender + all To addresses (excluding ourselves)
        let mut recipients: Vec<String> = vec![cached.from_addr.clone()];
        if !cached.to_addr.is_empty() {
            for addr in cached.to_addr.split(',').map(|s| s.trim().to_string()) {
                if !addr.is_empty() && addr.to_lowercase() != account.email.to_lowercase() && !recipients.contains(&addr) {
                    recipients.push(addr);
                }
            }
        }

        let mut sent_to = Vec::new();
        let mut errors = Vec::new();
        for recipient in &recipients {
            match smtp_client::send_email(&account, recipient, &subject, body, reply_to) {
                Ok(()) => sent_to.push(recipient.as_str()),
                Err(e) => errors.push(format!("{}: {}", recipient, e)),
            }
        }

        if errors.is_empty() {
            format!("Reply-all sent to {} — subject: \"{}\"", sent_to.join(", "), subject)
        } else {
            format!("Sent to: {}. Errors: {}", sent_to.join(", "), errors.join("; "))
        }
    }
}

// ── email_forward ──

struct EmailForwardTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailForwardTool {
    fn name(&self) -> &'static str { "email_forward" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_forward",
                "description": "Forward email to new recipient",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to forward (from email_list output)."
                        },
                        "to": {
                            "type": "string",
                            "description": "Recipient email address to forward to."
                        },
                        "comment": {
                            "type": "string",
                            "description": "Optional message to prepend before the forwarded content."
                        }
                    },
                    "required": ["id", "to"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return "Error: 'to' (recipient) is required.".to_string(),
        };
        let comment = args.get("comment").and_then(|v| v.as_str()).unwrap_or("");

        db::init_tables(ctx.db.conn());
        let cached = match db::get_email(ctx.db.conn(), email_id) {
            Some(e) => e,
            None => return format!("Email ID {} not found.", email_id),
        };

        let mut account = match self.accounts.iter().find(|a| {
            db::ensure_account(ctx.db.conn(), &a.name, &a.email, &a.provider) == cached.account_id
        }) {
            Some(a) => a.clone(),
            None => return "Account not found for this email.".to_string(),
        };

        let config_path = std::env::var("YANTRIK_CONFIG").ok()
            .or_else(|| {
                let path = "/opt/yantrik/config.yaml";
                if std::path::Path::new(path).exists() { Some(path.to_string()) } else { None }
            });
        if let Err(e) = email::ensure_fresh_token(&mut account, config_path.as_deref()) {
            return format!("Token refresh failed: {}", e);
        }

        let subject = if cached.subject.to_lowercase().starts_with("fwd:") {
            cached.subject.clone()
        } else {
            format!("Fwd: {}", cached.subject)
        };

        // Build forwarded body
        let original_body = if cached.body_full.is_empty() {
            cached.body_preview.clone()
        } else {
            cached.body_full.clone()
        };

        let from_display = if cached.from_name.is_empty() {
            cached.from_addr.clone()
        } else {
            format!("{} <{}>", cached.from_name, cached.from_addr)
        };

        let body = if comment.is_empty() {
            format!(
                "\n---------- Forwarded message ----------\nFrom: {}\nDate: {}\nSubject: {}\n\n{}",
                from_display, format_timestamp(cached.date_ts), cached.subject, original_body
            )
        } else {
            format!(
                "{}\n\n---------- Forwarded message ----------\nFrom: {}\nDate: {}\nSubject: {}\n\n{}",
                comment, from_display, format_timestamp(cached.date_ts), cached.subject, original_body
            )
        };

        match smtp_client::send_email(&account, to, &subject, &body, None) {
            Ok(()) => format!("Forwarded email [{}] \"{}\" to {}.", email_id, cached.subject, to),
            Err(e) => format!("Forward failed: {}", e),
        }
    }
}

// ── email_move ──

struct EmailMoveTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailMoveTool {
    fn name(&self) -> &'static str { "email_move" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_move",
                "description": "Move email to another folder",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to move."
                        },
                        "folder": {
                            "type": "string",
                            "description": "Destination folder name (e.g., 'Trash', '[Gmail]/Spam', 'Archive')."
                        }
                    },
                    "required": ["id", "folder"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let dest_folder = match args.get("folder").and_then(|v| v.as_str()) {
            Some(f) if !f.is_empty() => f,
            _ => return "Error: 'folder' (destination) is required.".to_string(),
        };

        let (cached, _account, mut session) = match connect_for_email(ctx, &self.accounts, email_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        match imap_client::move_message(&mut session, &cached.folder, cached.uid, dest_folder) {
            Ok(()) => {
                let _ = session.logout();
                db::delete_email(ctx.db.conn(), email_id);
                format!("Moved email [{}] \"{}\" from {} to {}.", email_id, cached.subject, cached.folder, dest_folder)
            }
            Err(e) => {
                let _ = session.logout();
                format!("Move failed: {}", e)
            }
        }
    }
}

// ── email_archive ──

struct EmailArchiveTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailArchiveTool {
    fn name(&self) -> &'static str { "email_archive" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_archive",
                "description": "Archive email by ID; remove from inbox, not delete",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Email ID to archive."
                        }
                    },
                    "required": ["id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };

        let (cached, account, mut session) = match connect_for_email(ctx, &self.accounts, email_id) {
            Ok(r) => r,
            Err(e) => return e,
        };

        // Determine archive folder based on provider
        let archive_folder = match account.provider.to_lowercase().as_str() {
            "gmail" => "[Gmail]/All Mail",
            "outlook" => "Archive",
            _ => "Archive",
        };

        match imap_client::move_message(&mut session, &cached.folder, cached.uid, archive_folder) {
            Ok(()) => {
                let _ = session.logout();
                db::delete_email(ctx.db.conn(), email_id);
                format!("Archived email [{}] \"{}\" → {}.", email_id, cached.subject, archive_folder)
            }
            Err(e) => {
                let _ = session.logout();
                format!("Archive failed: {}", e)
            }
        }
    }
}

// ── email_list_folders ──

struct EmailListFoldersTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for EmailListFoldersTool {
    fn name(&self) -> &'static str { "email_list_folders" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "email" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "email_list_folders",
                "description": "List available email folders or labels",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "account": {
                            "type": "string",
                            "description": "Account name or email. Omit for default."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let account_name = args.get("account").and_then(|v| v.as_str());

        let (_, account) = match get_fresh_account(&self.accounts, account_name) {
            Ok(a) => a,
            Err(e) => return e,
        };

        let mut session = match imap_client::connect(&account) {
            Ok(s) => s,
            Err(e) => return format!("IMAP connection failed: {}", e),
        };

        let folders = imap_client::list_folders(&mut session);
        let _ = session.logout();

        let mut result = format!("Folders for {} ({}):\n\n", account.name, account.email);
        for folder in &folders {
            result.push_str(&format!("  - {}\n", folder));
        }
        result
    }
}
