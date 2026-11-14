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
use super::prefetch;
use super::notes::{NoteHit, NoteSource};
use super::promises::{Promise, PromiseSource};
use super::remote::{CachedFiles, CachedNotes, Fetcher, Missing, SnapshotCalendar};
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
            .map(|m| Mail {
                account: db::account_address(self.conn, m.account_id).unwrap_or_default(),
                folder: m.folder,
                id: m.uid.to_string(),
                from_name: m.from_name,
                from_addr: m.from_addr,
                subject: m.subject,
                date_ts: m.date_ts,
            })
            .collect())
    }
}

/// The world model's open commitments.
pub struct WorldModelPromises<'a> {
    pub conn: &'a Connection,
    /// The names the person goes by in a commitment, lower case: their own, and `user`, which is
    /// what the extractor writes for them.
    pub me: Vec<String>,
}

impl WorldModelPromises<'_> {
    fn is_me(&self, who: &str) -> bool {
        let who = who.trim().to_lowercase();
        self.me.iter().any(|m| *m == who)
    }

    fn promise(&self, c: Commitment) -> Promise {
        let open = match c.source {
            // The message the promise was found in, by where the cache holds it: a subject names
            // no message exactly.
            CommitmentSource::Email { message_id, .. } => {
                db::locate_message(self.conn, &message_id).map(|(account, folder, uid)| Open::Message { account, folder, id: uid.to_string() })
            }
            CommitmentSource::Calendar { event_id, .. } if !event_id.trim().is_empty() => Some(Open::Event { id: event_id }),
            _ => None,
        };
        Promise { text: c.action, open }
    }
}

impl PromiseSource for WorldModelPromises<'_> {
    /// What `name` promised the person: made by them, to the person.
    fn made_by(&self, name: &str) -> Result<Vec<Promise>, String> {
        let all = WorldModel::commitments_by(self.conn, name);
        Ok(all.into_iter().filter(|c| self.is_me(&c.promisee)).map(|c| self.promise(c)).collect())
    }

    /// What the person promised `name`: made to them, by the person. A promise between two other
    /// people on the invitation is neither.
    fn made_to(&self, name: &str) -> Result<Vec<Promise>, String> {
        let all = WorldModel::commitments_to(self.conn, name);
        Ok(all.into_iter().filter(|c| self.is_me(&c.promisor)).map(|c| self.promise(c)).collect())
    }
}

/// The notes service, `notes.search`. The service is started first if it is down, as every
/// caller of it does.
pub struct ServiceNotes;

impl NoteSource for ServiceNotes {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        let client = yantrik_ipc_transport::service::client("notes")?;
        let found: Vec<NoteSummary> = client.call_typed("notes.search", &serde_json::json!({ "query": query })).map_err(|e| e.message)?;
        Ok(found.into_iter().map(|n| NoteHit { id: n.id, title: n.title, modified_at: n.modified_at }).collect())
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
    let snapshot = fetcher.snapshot();
    let missing = Missing::default();
    let (notes, files) = (CachedNotes { snapshot: &snapshot, missing: &missing }, CachedFiles { snapshot: &snapshot, missing: &missing });
    let me = own(conn, user_name);
    let (mail, promises) = (EmailCache { conn }, WorldModelPromises { conn, me: [me.name.clone(), "user".to_string()].into_iter().filter(|n| !n.is_empty()).collect() });
    let sources = Sources { emails: &mail, notes: &notes, promises: &promises, files: &files };
    let outcome = engine::tick(conn, &SnapshotCalendar(&snapshot), &sources, &me, now, true);
    // The searches of the meetings about to be due, so their briefs are complete when made.
    if let Some(at) = super::window::to_unix(now) {
        prefetch::want(&snapshot, at, &me, &missing);
    }
    // The calendar once per tick, and the searches a brief is waiting for or is about to.
    fetcher.request(missing.notes.take(), missing.files.take());
    outcome
}

/// The fetcher for this process: started on first use, with the real calendar and notes service.
fn fetcher() -> &'static Fetcher {
    static FETCHER: OnceLock<Fetcher> = OnceLock::new();
    FETCHER.get_or_init(|| {
        let roots = std::env::var_os("HOME").map(|h| FileRoots::in_home(std::path::Path::new(&h))).unwrap_or_default();
        Fetcher::spawn(Arc::new(ServiceCalendar), Arc::new(ServiceNotes), Arc::new(roots))
    })
}

/// The worker's tick against the real stores, at the local time now.
pub fn tick_local(conn: &Connection, user_name: &str, enabled: bool) -> Outcome {
    if !enabled {
        return Outcome::default();
    }
    tick_with(conn, user_name, fetcher(), chrono::Local::now().naive_local(), true)
}
