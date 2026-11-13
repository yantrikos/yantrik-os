//! The mind's tools that read mail: check, list, read, search, list folders.
//!
//! Every one asks `email-service`; none opens a mailbox itself. All are `Safe`: they change
//! nothing on the server, except that reading a message marks it read there, as it always did.

use std::sync::Arc;

use rusqlite::Connection;

use super::support;
use super::{PermissionLevel, Tool, ToolContext};
use crate::email::db;
use crate::email::port::MailPort;

/// How many of a folder's newest messages one check lists.
pub(super) const CHECK_PAGE: u32 = 50;

// ── email_check ──

pub(super) struct EmailCheckTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailCheckTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
        let account = match support::find(&*self.mail, args) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let account_id = support::cache_id(conn, &account);
        let since_uid = db::get_last_sync_uid(conn, account_id);

        let listed = match self.mail.list_messages(&account.id, folder, 1, CHECK_PAGE) {
            Ok(l) => l,
            Err(e) => return format!("Fetch failed: {}", support::said(&e)),
        };
        let (max_uid, new_count) = support::remember(conn, account_id, folder, &listed, since_uid);
        if max_uid > since_uid {
            db::update_last_sync_uid(conn, account_id, max_uid);
        }

        let unread = support::unread_in(&*self.mail, &account.id, folder, db::count_unread(conn, account_id, folder), CHECK_PAGE);
        let recent = db::list_emails(conn, account_id, folder, 10);
        let mut result = format!(
            "Synced {} new emails for {} ({}). {}.\n\nRecent:\n",
            new_count, support::label(&account), account.email, unread
        );
        for email in &recent {
            result.push_str(&support::summary_line(email));
            result.push('\n');
        }
        result
    }
}

// ── email_list ──

pub(super) struct EmailListTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailListTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let account = match support::find(&*self.mail, args) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let account_id = support::cache_id(conn, &account);
        let unread = support::unread_in(&*self.mail, &account.id, folder, db::count_unread(conn, account_id, folder), CHECK_PAGE);
        let emails = db::list_emails(conn, account_id, folder, limit);

        if emails.is_empty() {
            return format!("No emails in {} for {}. Try email_check to sync first.", folder, support::label(&account));
        }
        let mut result = format!("{} ({}) — {} | {}\n", support::label(&account), account.email, folder, unread);
        result.push_str("Legend: * = unread, ! = flagged\n\n");
        for email in &emails {
            result.push_str(&support::summary_line(email));
            result.push('\n');
        }
        result
    }
}

// ── email_read ──

pub(super) struct EmailReadTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailReadTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let c = match support::cached(conn, &*self.mail, email_id) {
            Ok(c) => c,
            Err(e) => return e,
        };

        // Already fetched: return it.
        if !c.row.body_full.is_empty() {
            return format_full_email(&c.row);
        }
        let uid = c.row.uid.to_string();
        let detail = match self.mail.get_message(&c.account.id, &c.row.folder, &uid) {
            Ok(d) => d,
            Err(e) => return format!("Fetch body failed: {}", support::said(&e)),
        };
        // Reading it marks it read on the server, best effort, as it always did.
        let _ = self.mail.mark_read(&c.account.id, &c.row.folder, &[uid], true);

        db::update_email_body(conn, email_id, &detail.body_text);
        let mut updated = c.row;
        updated.body_full = detail.body_text;
        format_full_email(&updated)
    }
}

fn format_full_email(email: &db::CachedEmail) -> String {
    let date = support::stamp(email.date_ts);
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

// ── email_search ──

pub(super) struct EmailSearchTool {
    pub mail: Arc<dyn MailPort>,
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
                "description": "Search emails by keyword in a folder; not web search",
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
                        "folder": {
                            "type": "string",
                            "description": "Folder to search. Default: INBOX."
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailSearchTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q,
            _ => return "Error: 'query' is required".to_string(),
        };
        let folder = args.get("folder").and_then(|v| v.as_str()).filter(|f| !f.is_empty()).unwrap_or("INBOX");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let account = match support::find(&*self.mail, args) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let found = match self.mail.search(&account.id, folder, query) {
            Ok(f) => f,
            Err(e) => return format!("Search failed: {}", support::said(&e)),
        };
        let account_id = support::cache_id(conn, &account);
        support::remember(conn, account_id, folder, &found, u32::MAX);

        let results: Vec<db::CachedEmail> = found
            .iter()
            .filter_map(|m| m.id.parse::<u32>().ok())
            .filter_map(|uid| db::find(conn, account_id, uid, folder))
            .take(limit)
            .collect();
        if results.is_empty() {
            return format!("No emails matching '{}' in {} for {}.", query, folder, support::label(&account));
        }
        let mut output = format!("Search '{}' in {} — {} results:\n\n", query, support::label(&account), results.len());
        for email in &results {
            output.push_str(&support::summary_line(email));
            output.push('\n');
        }
        output
    }
}

// ── email_list_folders ──

pub(super) struct EmailListFoldersTool {
    pub mail: Arc<dyn MailPort>,
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

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailListFoldersTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let account = match support::find(&*self.mail, args) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let _ = conn;
        let folders = match self.mail.list_folders(&account.id) {
            Ok(f) => f,
            Err(e) => return format!("Could not list folders: {}", support::said(&e)),
        };
        let mut result = format!("Folders for {} ({}):\n\n", support::label(&account), account.email);
        for folder in &folders {
            match folder.counts {
                Some(c) => result.push_str(&format!("  - {} ({} unread)\n", folder.name, c.unread)),
                None => result.push_str(&format!("  - {}\n", folder.name)),
            }
        }
        result
    }
}
