//! What every mail tool shares: choosing an account, reading the local cache of listed mail, and
//! the one line a message is shown as.
//!
//! The accounts are `email-service`'s. The cache is the companion's own table of what the service
//! last listed (`crate::email::db`), kept because the mind refers to a message by a small number
//! and because the VIP-mail playbook reads it. It holds mail the person already has, never a
//! secret, and a row's `uid` and `folder` are what the service is asked about.

use rusqlite::Connection;
use yantrik_ipc_contracts::email::{EmailAccountSummary, EmailSummary, ServiceError};

use crate::email::db;
use crate::email::port::MailPort;

/// What the person sees for an account: its name, else its address.
pub(super) fn label(account: &EmailAccountSummary) -> &str {
    if account.display_name.trim().is_empty() {
        &account.email
    } else {
        &account.display_name
    }
}

/// The service's words for a failure. They are fixed text and never carry a secret.
pub(super) fn said(error: &ServiceError) -> String {
    error.message.clone()
}

/// The accounts the mail service has, or why it could not say.
pub(super) fn accounts(port: &dyn MailPort) -> Result<Vec<EmailAccountSummary>, String> {
    let all = port.accounts().map_err(|e| said(&e))?;
    if all.is_empty() {
        return Err("No email accounts configured".to_string());
    }
    Ok(all)
}

/// An account by position, name or address (case-insensitive, partial); the first when none is
/// named.
pub(super) fn pick<'a>(
    accounts: &'a [EmailAccountSummary],
    wanted: Option<&str>,
) -> Result<&'a EmailAccountSummary, String> {
    match wanted.map(str::trim) {
        None | Some("") => Ok(&accounts[0]),
        Some(s) => {
            if let Some(found) = s.parse::<usize>().ok().and_then(|i| accounts.get(i)) {
                return Ok(found);
            }
            let lower = s.to_lowercase();
            accounts
                .iter()
                .find(|a| label(a).to_lowercase().contains(&lower) || a.email.to_lowercase().contains(&lower))
                .ok_or_else(|| {
                    format!(
                        "Account '{}' not found. Available: {}",
                        s,
                        accounts.iter().map(label).collect::<Vec<_>>().join(", ")
                    )
                })
        }
    }
}

/// The arguments that reach a mail server's command line, when one holds a line break or other
/// control character. The service refuses these too (`imap_text`); this is the same rule one step
/// earlier, so a model that was talked into one is told no without a call, and the text is never
/// echoed back.
pub(super) fn control_in(args: &serde_json::Value) -> Option<String> {
    ["account", "folder", "query"]
        .into_iter()
        .find(|key| args.get(*key).and_then(|v| v.as_str()).is_some_and(|s| s.chars().any(char::is_control)))
        .map(|key| format!("Error: '{key}' may not contain a line break or other control character."))
}

/// The account the arguments name (`account`), from the service's list.
pub(super) fn find(port: &dyn MailPort, args: &serde_json::Value) -> Result<EmailAccountSummary, String> {
    if let Some(refusal) = control_in(args) {
        return Err(refusal);
    }
    let all = accounts(port)?;
    pick(&all, args.get("account").and_then(|v| v.as_str())).cloned()
}

/// The cache's id for an account.
pub(super) fn cache_id(conn: &Connection, account: &EmailAccountSummary) -> i64 {
    db::init_tables(conn);
    db::ensure_account(conn, label(account), &account.email, &account.provider)
}

/// A cached message and the account that owns it.
pub(super) struct Cached {
    pub row: db::CachedEmail,
    pub account: EmailAccountSummary,
}

/// A cached message by its number, with its account. The two ways to miss are told apart.
pub(super) fn cached(conn: &Connection, port: &dyn MailPort, email_id: i64) -> Result<Cached, String> {
    db::init_tables(conn);
    let row = db::get_email(conn, email_id)
        .ok_or_else(|| format!("Email ID {} not found. Use email_check to sync.", email_id))?;
    let all = accounts(port)?;
    let account = all
        .into_iter()
        .find(|a| cache_id(conn, a) == row.account_id)
        .ok_or_else(|| format!("Account for email ID {} is not set up in the mail service.", email_id))?;
    Ok(Cached { row, account })
}

/// Cache what the service listed. Returns the highest UID seen and how many were above `since`.
pub(super) fn remember(
    conn: &Connection,
    account_id: i64,
    folder: &str,
    listed: &[EmailSummary],
    since: u32,
) -> (u32, u32) {
    let (mut max_uid, mut fresh) = (since, 0u32);
    for m in listed {
        let Ok(uid) = m.id.parse::<u32>() else { continue };
        let from_addr = if m.from_address.is_empty() && m.from.contains('@') { m.from.clone() } else { m.from_address.clone() };
        let from_name = if m.from == from_addr { String::new() } else { m.from.clone() };
        let preview = if m.subject.len() > 100 {
            format!("{}...", &m.subject[..m.subject.floor_char_boundary(97)])
        } else {
            m.subject.clone()
        };
        db::upsert_email(
            conn,
            account_id,
            uid,
            folder,
            &from_addr,
            &from_name,
            &m.to.join(", "),
            &m.subject,
            date_ts(&m.date),
            &preview,
            m.is_read,
            m.is_starred,
            &m.message_id,
            "",
        );
        if uid > since {
            fresh += 1;
        }
        max_uid = max_uid.max(uid);
    }
    (max_uid, fresh)
}

/// The service prints a date as `2026-09-20 03:47` in the machine's zone. Anything else it could
/// not parse is passed through as sent, which has no seconds to give: `0.0` shows as "unknown".
pub(super) fn date_ts(date: &str) -> f64 {
    use chrono::TimeZone;
    chrono::NaiveDateTime::parse_from_str(date.trim(), "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|t| chrono::Local.from_local_datetime(&t).earliest())
        .map(|t| t.timestamp() as f64)
        .unwrap_or(0.0)
}

/// Format a cached email as a compact summary line. The shape is read by the pulse extractor.
pub(super) fn summary_line(email: &db::CachedEmail) -> String {
    let read_marker = if email.is_read { " " } else { "*" };
    let flag_marker = if email.is_flagged { "!" } else { " " };
    let from = if email.from_name.is_empty() { &email.from_addr } else { &email.from_name };
    let subject = if email.subject.len() > 60 {
        format!("{}...", &email.subject[..email.subject.floor_char_boundary(57)])
    } else {
        email.subject.clone()
    };
    format!("[{}]{}{} {} | {} | {}", email.id, read_marker, flag_marker, from, subject, stamp(email.date_ts))
}

/// A unix timestamp as a date a person reads.
pub(super) fn stamp(ts: f64) -> String {
    if ts <= 0.0 {
        return "unknown".to_string();
    }
    match chrono::DateTime::from_timestamp(ts as i64, 0) {
        Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
        None => "unknown".to_string(),
    }
}

/// The addresses in a header value (`Ada <ada@example.com>, bob@example.com`), bare.
pub(super) fn addresses(header: &str) -> Vec<String> {
    use mailparse::MailAddr;
    let Ok(list) = mailparse::addrparse(header) else { return Vec::new() };
    list.iter()
        .flat_map(|a| match a {
            MailAddr::Single(one) => vec![one.addr.clone()],
            MailAddr::Group(group) => group.addrs.iter().map(|one| one.addr.clone()).collect(),
        })
        .collect()
}

/// `Re: ` or `Fwd: ` in front of a subject, once.
pub(super) fn prefixed(prefix: &str, subject: &str) -> String {
    if subject.to_lowercase().starts_with(&prefix.to_lowercase()) {
        subject.to_string()
    } else {
        format!("{prefix} {subject}")
    }
}

/// How many unread messages a folder has, in words a mind can state as true.
///
/// The cache holds only what `email_check` last listed (the newest page), so counting it gives
/// "41 unread" for a mailbox with 60 (520 gate, 31 Oct). The folder's own number comes from the
/// service's `list_folders`, which asks the server (IMAP STATUS) and is the number the Email app
/// shows. When the server will not say, the sentence names the window it counted instead of
/// presenting it as the folder's.
pub(super) fn unread_in(port: &dyn MailPort, account_id: &str, folder: &str, cached_unread: i64, window: u32) -> String {
    let counted = port
        .list_folders(account_id)
        .ok()
        .and_then(|all| all.into_iter().find(|f| f.name.eq_ignore_ascii_case(folder)))
        .and_then(|f| f.counts);
    match counted {
        Some(c) => format!("{} unread of {} in {}", c.unread, c.total, folder),
        None => format!(
            "{cached_unread} unread among the newest {window} messages synced (the count of the whole {folder} could not be read)"
        ),
    }
}
