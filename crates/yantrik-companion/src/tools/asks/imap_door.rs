//! The real mailbox behind the mail asks (#1311, round 2): a fresh IMAP session each time it is
//! needed, so no session is held open while a card waits for the person.
//!
//! `mail::Door` is what the asks need of a mailbox: open a session, read the one message that was
//! shown, change it, close. This is that over IMAP. The tools look the message up, let go of that
//! session, ask, and on an Allow come back through here to check the message is the one that was
//! shown before they change it.

use std::net::TcpStream;

use imap::Session;

use super::mail::{Door, Seen};
use crate::config::EmailAccountConfig;
use crate::email::imap_client;

pub type Live = Session<native_tls::TlsStream<TcpStream>>;

/// A session with a fresh token: what `connect_for_email` and the reconnect both need.
pub fn fresh_session(account: &mut EmailAccountConfig) -> Result<Live, String> {
    let config_path = std::env::var("YANTRIK_CONFIG").ok().or_else(|| {
        let path = "/opt/yantrik/config.yaml";
        std::path::Path::new(path).exists().then(|| path.to_string())
    });
    crate::email::ensure_fresh_token(account, config_path.as_deref()).map_err(|e| format!("Token refresh failed: {}", e))?;
    imap_client::connect(account).map_err(|e| format!("IMAP connection failed: {}", e))
}

/// One account's mailbox.
pub struct Imap<'a> {
    pub account: &'a EmailAccountConfig,
}

impl Door for Imap<'_> {
    type S = Live;

    fn open(&self) -> Result<Live, String> {
        fresh_session(&mut self.account.clone())
    }

    fn seen(&self, session: &mut Live, folder: &str, uid: u32) -> Result<Option<Seen>, String> {
        session.select(folder).map_err(|e| format!("SELECT failed: {}", e))?;
        let found = session.uid_fetch(uid.to_string(), "(UID ENVELOPE)").map_err(|e| format!("FETCH failed: {}", e))?;
        let text = |bytes: Option<&[u8]>| bytes.map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
        // `uid:*` answers with the last message when there is no `uid`, so the uid is read back.
        Ok(found.iter().find(|m| m.uid == Some(uid)).and_then(|m| m.envelope()).map(|e| Seen {
            subject: text(e.subject),
            message_id: text(e.message_id),
        }))
    }

    fn delete(&self, session: &mut Live, folder: &str, uid: u32) -> Result<(), String> {
        imap_client::delete_message(session, folder, uid)
    }

    fn move_to(&self, session: &mut Live, folder: &str, uid: u32, to: &str) -> Result<(), String> {
        imap_client::move_message(session, folder, uid, to)
    }

    fn close(&self, mut session: Live) {
        let _ = session.logout();
    }
}
