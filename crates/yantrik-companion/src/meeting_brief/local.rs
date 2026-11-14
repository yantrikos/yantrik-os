//! The real sources: the companion's email cache, the world model's commitments and the notes
//! service, which is a program on this machine reached over its local socket.

use rusqlite::Connection;
use yantrik_ipc_contracts::notes::NoteSummary;

use super::assemble::Sources;
use super::emails::{EmailSource, Mail};
use super::engine::{self, Outcome};
use super::files::FileRoots;
use super::model::Open;
use super::notes::{NoteHit, NoteSource};
use super::promises::{Promise, PromiseSource};
use super::window::Own;
use crate::calendar::backend::ServiceCalendar;
use crate::email::db;
use crate::world_model::{Commitment, CommitmentSource, WorldModel};

/// The email cache.
pub struct EmailCache<'a> {
    pub conn: &'a Connection,
}

impl EmailSource for EmailCache<'_> {
    fn exchanged(&self, addresses: &[String], since: i64, limit: usize) -> Result<Vec<Mail>, String> {
        Ok(db::emails_with_participants(self.conn, addresses, since as f64, limit)
            .into_iter()
            .map(|m| Mail { from_name: m.from_name, from_addr: m.from_addr, subject: m.subject, date_ts: m.date_ts })
            .collect())
    }
}

/// The world model's open commitments.
pub struct WorldModelPromises<'a> {
    pub conn: &'a Connection,
}

fn promise(c: Commitment) -> Promise {
    let open = match c.source {
        CommitmentSource::Email { subject, .. } if !subject.trim().is_empty() => Some(Open::Message { subject }),
        CommitmentSource::Calendar { event_id, .. } if !event_id.trim().is_empty() => Some(Open::Event { id: event_id }),
        _ => None,
    };
    Promise { text: c.action, open }
}

impl PromiseSource for WorldModelPromises<'_> {
    fn made_by(&self, name: &str) -> Result<Vec<Promise>, String> {
        Ok(WorldModel::commitments_by(self.conn, name).into_iter().map(promise).collect())
    }

    fn made_to(&self, name: &str) -> Result<Vec<Promise>, String> {
        Ok(WorldModel::commitments_to(self.conn, name).into_iter().map(promise).collect())
    }
}

/// The notes service, `notes.search`. The service is started first if it is down, as every
/// caller of it does.
pub struct ServiceNotes;

impl NoteSource for ServiceNotes {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        let client = yantrik_ipc_transport::service::client("notes")?;
        let found: Vec<NoteSummary> = client.call_typed("notes.search", &serde_json::json!({ "query": query })).map_err(|e| e.message)?;
        Ok(found.into_iter().map(|n| NoteHit { title: n.title, modified_at: n.modified_at }).collect())
    }
}

/// The person's own identity: every account address the email cache knows, and their name.
pub fn own(conn: &Connection, user_name: &str) -> Own {
    let addresses: Vec<String> = conn
        .prepare("SELECT email FROM email_accounts")
        .and_then(|mut stmt| stmt.query_map([], |row| row.get::<_, String>(0)).map(|rows| rows.flatten().collect()))
        .unwrap_or_default();
    Own::new(addresses, user_name)
}

/// One tick against the real stores, at the local time now. `enabled` is the person's switch;
/// off reads nothing. The home folder is `$HOME`, and a machine without one has no files to
/// offer, not a search of the wrong place.
pub fn tick_local(conn: &Connection, user_name: &str, enabled: bool) -> Outcome {
    if !enabled {
        return Outcome::default();
    }
    let roots = std::env::var_os("HOME").map(|h| FileRoots::in_home(std::path::Path::new(&h))).unwrap_or_default();
    let (mail, promises) = (EmailCache { conn }, WorldModelPromises { conn });
    let sources = Sources { emails: &mail, notes: &ServiceNotes, promises: &promises, files: &roots };
    engine::tick(conn, &ServiceCalendar, &sources, &own(conn, user_name), chrono::Local::now().naive_local(), true)
}
