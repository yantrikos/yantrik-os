//! The mind's tools that send mail: send, reply, reply all, forward.
//!
//! Every one hands a message to `email-service`, which signs in with the account's secret from the
//! credential vault. All four are `Standard`, as they were, so a send still goes through the
//! companion's approval card (`email_send` and `email_reply` by name, in `companion.rs` and the
//! policy engine); nothing here sends without being asked to by that path.

use std::sync::Arc;

use rusqlite::Connection;
use yantrik_ipc_contracts::email::ComposeRequest;

use super::support;
use super::{PermissionLevel, Tool, ToolContext};
use crate::email::port::MailPort;

fn compose(to: Vec<String>, cc: Vec<String>, subject: &str, body: &str, reply_to_id: Option<String>) -> ComposeRequest {
    ComposeRequest { to, cc, bcc: Vec::new(), subject: subject.to_string(), body: body.to_string(), reply_to_id, signature: None }
}

// ── email_send ──

pub(super) struct EmailSendTool {
    pub mail: Arc<dyn MailPort>,
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

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailSendTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return "Error: 'to' (recipient email) is required".to_string(),
        };
        let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let account = match support::find(&*self.mail, args) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let _ = conn;
        match self.mail.send(&account.id, &compose(vec![to.to_string()], Vec::new(), subject, body, None)) {
            Ok(()) => format!("Email sent to {} from {} — subject: \"{}\"", to, account.email, subject),
            Err(e) => format!("Send failed: {}", support::said(&e)),
        }
    }
}

// ── email_reply ──

pub(super) struct EmailReplyTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailReplyTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let body = match args.get("body").and_then(|v| v.as_str()) {
            Some(b) if !b.is_empty() => b,
            _ => return "Error: 'body' is required".to_string(),
        };
        let c = match support::cached(conn, &*self.mail, email_id) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let (sender, parent) = match sender_and_parent(&*self.mail, &c) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let subject = support::prefixed("Re:", &c.row.subject);
        match self.mail.send(&c.account.id, &compose(vec![sender.clone()], Vec::new(), &subject, body, parent)) {
            Ok(()) => format!("Reply sent to {} — subject: \"{}\"", sender, subject),
            Err(e) => format!("Reply failed: {}", support::said(&e)),
        }
    }
}

/// Who a reply goes to, and the `Message-ID` it answers: from the cache when the listing gave
/// them, else from the message itself.
fn sender_and_parent(mail: &dyn MailPort, c: &support::Cached) -> Result<(String, Option<String>), String> {
    let parent = Some(c.row.message_id.clone()).filter(|m| !m.is_empty());
    if !c.row.from_addr.is_empty() && parent.is_some() {
        return Ok((c.row.from_addr.clone(), parent));
    }
    let detail = mail
        .get_message(&c.account.id, &c.row.folder, &c.row.uid.to_string())
        .map_err(|e| format!("Could not read the message to answer it: {}", support::said(&e)))?;
    let sender = if c.row.from_addr.is_empty() {
        support::addresses(&detail.from)
            .into_iter()
            .next()
            .ok_or_else(|| "The message has no sender address to answer.".to_string())?
    } else {
        c.row.from_addr.clone()
    };
    Ok((sender, parent.or(Some(detail.message_id).filter(|m| !m.is_empty()))))
}

// ── email_reply_all ──

pub(super) struct EmailReplyAllTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailReplyAllTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let body = match args.get("body").and_then(|v| v.as_str()) {
            Some(b) if !b.is_empty() => b,
            _ => return "Error: 'body' is required.".to_string(),
        };
        let c = match support::cached(conn, &*self.mail, email_id) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let detail = match self.mail.get_message(&c.account.id, &c.row.folder, &c.row.uid.to_string()) {
            Ok(d) => d,
            Err(e) => return format!("Reply-all failed: {}", support::said(&e)),
        };
        let sender = match Some(c.row.from_addr.clone())
            .filter(|a| !a.is_empty())
            .or_else(|| support::addresses(&detail.from).into_iter().next())
        {
            Some(s) => s,
            None => return "Reply-all failed: the message has no sender address.".to_string(),
        };
        let subject = support::prefixed("Re:", &c.row.subject);
        let parent = Some(c.row.message_id.clone())
            .filter(|m| !m.is_empty())
            .or(Some(detail.message_id.clone()).filter(|m| !m.is_empty()));

        // One message to the sender, everyone else on it copied; never ourselves, never twice.
        let mut cc: Vec<String> = Vec::new();
        for addr in detail.to.iter().chain(detail.cc.iter()).flat_map(|h| support::addresses(h)) {
            let same = |x: &String| x.eq_ignore_ascii_case(&addr);
            if !addr.eq_ignore_ascii_case(&c.account.email) && !same(&sender) && !cc.iter().any(same) {
                cc.push(addr);
            }
        }
        let everyone: Vec<String> = std::iter::once(sender.clone()).chain(cc.iter().cloned()).collect();
        match self.mail.send(&c.account.id, &compose(vec![sender], cc, &subject, body, parent)) {
            Ok(()) => format!("Reply-all sent to {} — subject: \"{}\"", everyone.join(", "), subject),
            Err(e) => format!("Reply-all failed: {}", support::said(&e)),
        }
    }
}

// ── email_forward ──

pub(super) struct EmailForwardTool {
    pub mail: Arc<dyn MailPort>,
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
        self.run(&ctx.db.conn(), args)
    }
}

impl EmailForwardTool {
    /// The tool, given the cache to use. `execute` is this with the companion's database.
    pub(super) fn run(&self, conn: &Connection, args: &serde_json::Value) -> String {
        let email_id = match args.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return "Error: 'id' is required.".to_string(),
        };
        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return "Error: 'to' (recipient) is required.".to_string(),
        };
        let comment = args.get("comment").and_then(|v| v.as_str()).unwrap_or("");
        let c = match support::cached(conn, &*self.mail, email_id) {
            Ok(c) => c,
            Err(e) => return e,
        };

        let subject = support::prefixed("Fwd:", &c.row.subject);
        let original_body = if c.row.body_full.is_empty() {
            match self.mail.get_message(&c.account.id, &c.row.folder, &c.row.uid.to_string()) {
                Ok(d) => d.body_text,
                Err(e) => return format!("Forward failed: {}", support::said(&e)),
            }
        } else {
            c.row.body_full.clone()
        };
        let from_display = if c.row.from_name.is_empty() {
            c.row.from_addr.clone()
        } else {
            format!("{} <{}>", c.row.from_name, c.row.from_addr)
        };
        let forwarded = format!(
            "---------- Forwarded message ----------\nFrom: {}\nDate: {}\nSubject: {}\n\n{}",
            from_display, support::stamp(c.row.date_ts), c.row.subject, original_body
        );
        let body = if comment.is_empty() { format!("\n{forwarded}") } else { format!("{comment}\n\n{forwarded}") };
        match self.mail.send(&c.account.id, &compose(vec![to.to_string()], Vec::new(), &subject, &body, None)) {
            Ok(()) => format!("Forwarded email [{}] \"{}\" to {}.", email_id, c.row.subject, to),
            Err(e) => format!("Forward failed: {}", support::said(&e)),
        }
    }
}
