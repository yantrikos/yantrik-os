//! The mind's tools that change mail on the server: mark read or unread, star, delete, move,
//! archive.
//!
//! Each one asks `email-service` to do it, in the folder the message was listed from (a UID means
//! nothing without its mailbox), then brings the local cache into line. Grades are as they were:
//! all `Standard` except `email_delete`, which is `Sensitive`.

use std::sync::Arc;

use rusqlite::Connection;

use super::support;
use super::{PermissionLevel, Tool, ToolContext};
use crate::email::db;
use crate::email::port::MailPort;

// ── email_mark_read ──

pub(super) struct EmailMarkReadTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailMarkReadTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        if args.get("all").and_then(|v| v.as_bool()).unwrap_or(false) {
            let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
            let account = match support::find(&*self.mail, args) {
                Ok(a) => a,
                Err(e) => return e,
            };
            let account_id = support::cache_id(conn, &account);
            let uids: Vec<String> =
                db::get_unread_uids(conn, account_id, folder).iter().map(u32::to_string).collect();
            if uids.is_empty() {
                return format!("No unread emails in {} for {}.", folder, support::label(&account));
            }
            return match self.mail.mark_read(&account.id, folder, &uids, true) {
                Ok(()) => {
                    db::mark_all_read(conn, account_id, folder);
                    format!("Marked {} emails as read in {} for {}.", uids.len(), folder, support::label(&account))
                }
                Err(e) => format!("Failed to mark as read: {}", support::said(&e)),
            };
        }
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: provide 'id' for a single email, or 'all: true' for bulk.".to_string(),
        };
        let c = match support::cached(conn, &*self.mail, email_id) {
            Ok(c) => c,
            Err(e) => return e,
        };

        match self.mail.mark_read(&c.account.id, &c.row.folder, &[c.row.uid.to_string()], true) {
            Ok(()) => {
                db::mark_read(conn, email_id);
                format!("Marked email [{}] \"{}\" as read.", email_id, c.row.subject)
            }
            Err(e) => format!("Failed: {}", support::said(&e)),
        }
    }
}

// ── email_mark_unread ──

pub(super) struct EmailMarkUnreadTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailMarkUnreadTool {
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

        match self.mail.mark_read(&c.account.id, &c.row.folder, &[c.row.uid.to_string()], false) {
            Ok(()) => {
                db::mark_unread(conn, email_id);
                format!("Marked email [{}] \"{}\" as unread.", email_id, c.row.subject)
            }
            Err(e) => format!("Failed: {}", support::said(&e)),
        }
    }
}

// ── email_flag ──

pub(super) struct EmailFlagTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailFlagTool {
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

        match self.mail.mark_starred(&c.account.id, &c.row.folder, &c.row.uid.to_string(), true) {
            Ok(()) => {
                db::set_flagged(conn, email_id, true);
                format!("Flagged email [{}] \"{}\".", email_id, c.row.subject)
            }
            Err(e) => format!("Failed: {}", support::said(&e)),
        }
    }
}

// ── email_unflag ──

pub(super) struct EmailUnflagTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailUnflagTool {
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

        match self.mail.mark_starred(&c.account.id, &c.row.folder, &c.row.uid.to_string(), false) {
            Ok(()) => {
                db::set_flagged(conn, email_id, false);
                format!("Unflagged email [{}] \"{}\".", email_id, c.row.subject)
            }
            Err(e) => format!("Failed: {}", support::said(&e)),
        }
    }
}

// ── email_delete ──

pub(super) struct EmailDeleteTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailDeleteTool {
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

        match self.mail.delete_message(&c.account.id, &c.row.folder, &c.row.uid.to_string()) {
            Ok(()) => {
                db::delete_email(conn, email_id);
                format!("Deleted email [{}] \"{}\" from {}.", email_id, c.row.subject, c.row.folder)
            }
            Err(e) => format!("Failed: {}", support::said(&e)),
        }
    }
}

// ── email_move ──

pub(super) struct EmailMoveTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailMoveTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let dest_folder = match args.get("folder").and_then(|v| v.as_str()) {
            Some(f) if !f.is_empty() => f,
            _ => return "Error: 'folder' (destination) is required.".to_string(),
        };
        let c = match support::cached(conn, &*self.mail, email_id) {
            Ok(c) => c,
            Err(e) => return e,
        };

        match self.mail.move_message(&c.account.id, &c.row.folder, &c.row.uid.to_string(), dest_folder) {
            Ok(()) => {
                db::delete_email(conn, email_id);
                format!("Moved email [{}] \"{}\" from {} to {}.", email_id, c.row.subject, c.row.folder, dest_folder)
            }
            Err(e) => format!("Move failed: {}", support::said(&e)),
        }
    }
}

// ── email_archive ──

pub(super) struct EmailArchiveTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailArchiveTool {
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

        let archive_folder = archive_folder(&c.account.provider);
        match self.mail.move_message(&c.account.id, &c.row.folder, &c.row.uid.to_string(), archive_folder) {
            Ok(()) => {
                db::delete_email(conn, email_id);
                format!("Archived email [{}] \"{}\" → {}.", email_id, c.row.subject, archive_folder)
            }
            Err(e) => format!("Archive failed: {}", support::said(&e)),
        }
    }
}

/// Where "archive" puts a message, by provider.
pub(super) fn archive_folder(provider: &str) -> &'static str {
    match provider.to_lowercase().as_str() {
        "gmail" => "[Gmail]/All Mail",
        _ => "Archive",
    }
}
