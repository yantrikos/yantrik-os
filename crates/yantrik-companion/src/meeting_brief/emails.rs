//! Section 1: the last messages exchanged with the people on the invitation.

use super::keywords::short_name;
use super::model::{age, clamp, Open, Row, LOOKBACK_DAYS, MAX_EMAILS, MAX_ROW_CHARS};
use super::window::{Own, Person};

/// One cached message, as a row needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Mail {
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub date_ts: f64,
    /// Which message this is, exactly: the account's address, the folder and the id there. A
    /// subject can be on many messages, and a message the person sent is not in the inbox.
    pub account: String,
    pub folder: String,
    pub id: String,
}

/// Where messages come from: the companion's email cache, or a stand-in in a test.
pub trait EmailSource {
    /// Messages sent by or to any of `addresses` at or after `since` (unix seconds), newest
    /// first, at most `limit`.
    fn exchanged(&self, addresses: &[String], since: i64, limit: usize) -> Result<Vec<Mail>, String>;
}

/// Up to three rows from the last 30 days: "Maya · Slides for Friday · 2 d".
pub fn section(source: &dyn EmailSource, people: &[Person], own: &Own, now: i64) -> Result<Vec<Row>, String> {
    let addresses: Vec<String> = people.iter().map(|p| p.email.clone()).filter(|e| !e.is_empty()).collect();
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    let since = now - LOOKBACK_DAYS * 86_400;
    let mut mails = source.exchanged(&addresses, since, MAX_EMAILS)?;
    mails.retain(|m| m.date_ts >= since as f64 && !m.subject.trim().is_empty());
    mails.sort_by(|a, b| b.date_ts.total_cmp(&a.date_ts));
    mails.truncate(MAX_EMAILS);
    Ok(mails
        .into_iter()
        .map(|m| {
            let who = if own.has_address(&m.from_addr) {
                "You".to_string()
            } else {
                sender(&m, people)
            };
            Row {
                text: clamp(&format!("{who} \u{b7} {} \u{b7} {}", m.subject, age(now, m.date_ts as i64)), MAX_ROW_CHARS),
                open: Some(Open::Message { account: m.account, folder: m.folder, id: m.id }),
            }
        })
        .collect())
}

/// Who a message is from, as the person knows them: the invitation's name for the address, else
/// the name on the message, else the part of the address before the `@`.
fn sender(m: &Mail, people: &[Person]) -> String {
    let from = m.from_addr.trim().to_lowercase();
    if let Some(p) = people.iter().find(|p| p.email == from) {
        return short_name(&p.name);
    }
    if !m.from_name.trim().is_empty() {
        return short_name(m.from_name.trim());
    }
    from.split('@').next().unwrap_or("").to_string()
}
