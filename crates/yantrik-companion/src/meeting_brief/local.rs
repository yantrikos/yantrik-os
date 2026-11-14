//! The real sources: the companion's email cache, the world model's commitments and the notes
//! service, which is a program on this machine reached over its local socket.

use std::sync::{Arc, OnceLock};

use rusqlite::Connection;
use yantrik_ipc_contracts::notes::NoteSummary;

use super::assemble::Sources;
use super::emails::{EmailSource, Mail};
use super::engine::{self, Outcome};
use super::files::FileRoots;
use super::model::Open;
use super::notes::{NoteHit, NoteSource};
use super::promises::{Promise, PromiseSource};
use super::remote::{CachedNotes, Fetcher, SnapshotCalendar};
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

/// One worker tick at local time `now`, against the local stores and what `fetcher` last found.
///
/// Nothing here waits on a service: the calendar and notes come from the fetcher's snapshot, the
/// email cache, the commitments and the anticipation table are short local reads and writes, and
/// the refresh is asked for with a send that cannot block. Off (`enabled` false) does nothing at
/// all, and asks for nothing.
pub fn tick_with(conn: &Connection, user_name: &str, fetcher: &Fetcher, now: chrono::NaiveDateTime, enabled: bool) -> Outcome {
    if !enabled {
        return Outcome::default();
    }
    let roots = std::env::var_os("HOME").map(|h| FileRoots::in_home(std::path::Path::new(&h))).unwrap_or_default();
    let snapshot = fetcher.snapshot();
    let notes = CachedNotes { snapshot: &snapshot, missing: Default::default() };
    let (mail, promises) = (EmailCache { conn }, WorldModelPromises { conn });
    let sources = Sources { emails: &mail, notes: &notes, promises: &promises, files: &roots };
    let outcome = engine::tick(conn, &SnapshotCalendar(&snapshot), &sources, &own(conn, user_name), now, true);
    // The calendar once per tick, and only the searches a brief was waiting for.
    fetcher.request(notes.missing.take());
    outcome
}

/// The fetcher for this process: started on first use, with the real calendar and notes service.
fn fetcher() -> &'static Fetcher {
    static FETCHER: OnceLock<Fetcher> = OnceLock::new();
    FETCHER.get_or_init(|| Fetcher::spawn(Arc::new(ServiceCalendar), Arc::new(ServiceNotes)))
}

/// The worker's tick against the real stores, at the local time now.
pub fn tick_local(conn: &Connection, user_name: &str, enabled: bool) -> Outcome {
    if !enabled {
        return Outcome::default();
    }
    tick_with(conn, user_name, fetcher(), chrono::Local::now().naive_local(), true)
}
