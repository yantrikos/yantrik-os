//! The cards for mail the companion is about to delete or move (#1311): `email_delete` asks
//! `delete_for_good`; `email_move` and `email_archive` ask `change_item`. Each is asked right
//! before the IMAP store and expunge, with the sender, subject and date of the message and where
//! it goes. Sender and subject come from the mail itself, so each is quoted: text inside one
//! cannot pose as another field.
//!
//! No session is held while the card waits (a server that grew tired of waiting would turn an
//! Allow into a failure). The tool looks the message up, lets go of its session, and asks; on an
//! Allow this opens a new session, reads the message again by its id and checks it is the one
//! that was shown (same subject and message id) before it changes anything. A message that has
//! changed or gone is said plainly and nothing is done.

use yantrik_companion_tools::asks::{act, Card, Question};

use crate::email::db::CachedEmail;

/// What the mailbox says a message is, read again after the Allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub subject: String,
    pub message_id: String,
}

/// What the asks need of a mailbox. `imap_door::Imap` is the real one.
pub trait Door {
    type S;
    fn open(&self) -> Result<Self::S, String>;
    fn seen(&self, session: &mut Self::S, folder: &str, uid: u32) -> Result<Option<Seen>, String>;
    fn delete(&self, session: &mut Self::S, folder: &str, uid: u32) -> Result<(), String>;
    fn move_to(&self, session: &mut Self::S, folder: &str, uid: u32, to: &str) -> Result<(), String>;
    fn close(&self, session: Self::S);
}

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

/// Whether the message now at the shown place is the one that was shown.
fn still_the_one(mail: &CachedEmail, now: Option<Seen>) -> Result<(), String> {
    let gone = || format!("The message \"{}\" is no longer in {}: it was moved or deleted while the person was asked. Nothing was changed.", mail.subject, mail.folder);
    let Some(now) = now else { return Err(gone()) };
    let same_id = mail.message_id.is_empty() || now.message_id.is_empty() || mail.message_id == now.message_id;
    if now.subject == mail.subject && same_id {
        return Ok(());
    }
    Err(format!(
        "The message at that place in {} is no longer the one that was shown (it was \"{}\"; it is now \"{}\"). Nothing was changed.",
        mail.folder, mail.subject, now.subject
    ))
}

/// On an Allow: a new session, the check, the change, and the session closed whatever happens.
fn change<D: Door>(
    q: Question,
    card: &Card,
    mail: &CachedEmail,
    door: &D,
    apply: impl FnOnce(&D, &mut D::S) -> Result<(), String>,
) -> Result<(), String> {
    act(q, card, || {
        let mut session = door
            .open()
            .map_err(|e| format!("The person allowed it, but the mailbox could not be reached again ({e}). Nothing was changed."))?;
        let result = door
            .seen(&mut session, &mail.folder, mail.uid)
            .and_then(|now| still_the_one(mail, now))
            .and_then(|()| apply(door, &mut session));
        door.close(session);
        result
    })
}

/// `email_delete`: ask, then delete the message.
pub fn delete<D: Door>(mail: &CachedEmail, account: &str, door: &D) -> Result<(), String> {
    let card = message("email_delete", mail, account).plain("change", "deletes the message for good");
    change(Question::DeleteForGood, &card, mail, door, |d, s| d.delete(s, &mail.folder, mail.uid))
}

/// `email_move` and `email_archive` (`tool` names which): ask, then move the message to `to`.
pub fn move_to<D: Door>(tool: &str, mail: &CachedEmail, account: &str, to: &str, door: &D) -> Result<(), String> {
    let card = message(tool, mail, account).text("to_folder", to);
    change(Question::ChangeItem, &card, mail, door, |d, s| d.move_to(s, &mail.folder, mail.uid, to))
}
