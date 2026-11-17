//! The cards for mail the companion is about to delete or move (#1311): `email_delete` asks
//! `delete_for_good`; `email_move` and `email_archive` ask `change_item`. Each is asked right
//! before the IMAP store and expunge, with the sender, subject and date of the message and where
//! it goes. Sender and subject come from the mail itself, so each is quoted: text inside one
//! cannot pose as another field.

use yantrik_companion_tools::asks::{act, Card, Question};

use crate::email::db::CachedEmail;

fn when(timestamp: f64) -> String {
    chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "no date".to_string())
}

/// The message as the card shows it.
fn message(tool: &str, mail: &CachedEmail, account: &str) -> Card {
    let from = if mail.from_name.is_empty() { mail.from_addr.clone() } else { format!("{} <{}>", mail.from_name, mail.from_addr) };
    Card::new(tool)
        .plain("account", account)
        .text("from", &from)
        .text("subject", &mail.subject)
        .plain("date", when(mail.date_ts))
        .text("folder", &mail.folder)
}

/// `email_delete`: ask, then run `delete` (the IMAP store and expunge).
pub fn delete(mail: &CachedEmail, account: &str, delete: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let card = message("email_delete", mail, account).plain("change", "deletes the message for good");
    act(Question::DeleteForGood, &card, delete)
}

/// `email_move` and `email_archive` (`tool` names which): ask, then run `moved`.
pub fn move_to(tool: &str, mail: &CachedEmail, account: &str, to: &str, moved: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let card = message(tool, mail, account).text("to_folder", to);
    act(Question::ChangeItem, &card, moved)
}
