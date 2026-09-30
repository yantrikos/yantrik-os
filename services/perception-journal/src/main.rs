//! The optic nerve's first segment: drain the eye, durably, before anything interprets.
//!
//! # Why this is a separate process
//!
//! perception-service worked for a full day and drained nowhere. `grep -rn "perception.since"`
//! over the whole workspace returned zero. The service was designed, measured, debugged,
//! deployed and independently probed, and nobody checked whether anything was listening —
//! because **a source with no consumer is indistinguishable from a source with one, from the
//! source's side.** Every log line was healthy. Every observation was correct. The ring filled
//! perfectly and overwrote itself.
//!
//! Measured while an agent was actually working on the machine: `next_seq 5885`, ring capacity
//! 2048, `missed 3837`. Eight minutes of that agent's real work was unrecoverable by the time
//! anyone looked.
//!
//! The obvious fix was to read the feed from the shell. That is wrong for a reason worth writing
//! down: the shell hosts the LLM, the companion worker and sixteen wire modules, and is by a wide
//! margin the most crash-prone thing in the system. Putting the drain there gates the most
//! reliable component's output behind the least reliable component's uptime. So the drain is its
//! own supervised process, it does nothing expensive, and it stays running while the shell
//! restarts.
//!
//! # What it does and does not do
//!
//! It appends and it counts. It does not decide what matters, attribute causality, build
//! episodes or write memories — all of that belongs to an interpretation worker reading *this*
//! journal, with its own separately-checkpointed cursor, so that a slow or wedged interpreter can
//! never stall the drain. Interpretation is where judgement lives and judgement is where bugs
//! live; this process is meant to be boring enough to trust.
//!
//! # The health surface is load-bearing, not decoration
//!
//! `journal.status` publishes lag, last consumed sequence, records, gaps and bytes. That exists
//! because of the specific bug above: the failure mode was *silence that looks like health*, and
//! no amount of testing the eye would have caught it. Something must be able to assert that a
//! consumer exists and is advancing — otherwise this service can also be perfect and
//! disconnected, one layer further along.

mod journal;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yantrik_service_sdk::prelude::*;

use journal::{Cursor, Journal, Record};

/// How long each read parks waiting for the eye to have something.
///
/// The service caps waits at 30s. Asking for the cap means one wakeup per half minute of complete
/// silence and none at all while events flow — the whole argument for kernel-level watching is
/// that it costs nothing when nothing is happening, and a polling drain would hand that back.
const WAIT: Duration = Duration::from_millis(30_000);

/// How long to wait before retrying when the eye is not there.
///
/// perception-service is optional: it needs CAP_SYS_ADMIN at startup and a kernel with fanotify.
/// Absence is a normal state and must not become a log flood.
const RETRY: Duration = Duration::from_secs(15);

/// Ceiling on one drain's batch, so a large backlog is written in bounded chunks rather than one
/// unbounded allocation after an outage.
const BATCH: usize = 512;

fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("YANTRIK_JOURNAL_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/state/yantrik/perception-journal")
}

/// What the drain has managed, for anything that needs to assert it is alive.
#[derive(Default)]
struct Health {
    connected: bool,
    /// The eye's `next_seq` at our last read — the far end of the stream.
    source_next_seq: u64,
    /// Our own cursor. `source_next_seq - consumed_next_seq` is the lag, and it is the number
    /// that matters: a lag that only grows means the drain is losing the race.
    consumed_next_seq: u64,
    records: u64,
    gaps: u64,
    observations_lost: u64,
    bytes: u64,
    last_progress_secs_ago: u64,
}

struct Service {
    health: Arc<Mutex<Health>>,
    journal: Arc<Mutex<Journal>>,
}

impl ServiceHandler for Service {
    fn service_id(&self) -> &str {
        "perception-journal"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            // The assertion that a consumer exists. Deliberately cheap and deliberately public:
            // whatever watches this OS should be able to notice that the eye is draining, and
            // notice equally when it stops.
            "journal.status" => {
                let h = self.health.lock().map_err(poisoned)?;
                Ok(serde_json::json!({
                    "connected": h.connected,
                    "source_next_seq": h.source_next_seq,
                    "consumed_next_seq": h.consumed_next_seq,
                    "lag": h.source_next_seq.saturating_sub(h.consumed_next_seq),
                    "records": h.records,
                    "gaps": h.gaps,
                    "observations_lost": h.observations_lost,
                    "bytes_on_disk": h.bytes,
                    "seconds_since_progress": h.last_progress_secs_ago,
                }))
            }

            // The interpretation worker's read path. It keeps its own cursor; this service does
            // not track who has read what, precisely so that a wedged reader cannot stall the
            // drain by failing to acknowledge.
            "journal.since" => {
                let from = params["seq"].as_u64().unwrap_or(0);
                let limit = params["limit"].as_u64().unwrap_or(256).min(2048) as usize;
                let journal = self.journal.lock().map_err(poisoned)?;
                let records = journal.read_from(from, limit).map_err(|e| ServiceError {
                    code: -32000,
                    message: format!("cannot read the journal: {e}"),
                })?;
                Ok(serde_json::json!({ "records": records }))
            }

            other => Err(ServiceError {
                code: -32601,
                message: format!(
                    "unknown method `{other}`; this service serves journal.status, journal.since"
                ),
            }),
        }
    }
}

fn poisoned<T>(_: T) -> ServiceError {
    ServiceError { code: -32000, message: "journal lock poisoned by a panicking thread".into() }
}

fn main() {
    yantrik_service_sdk::init_tracing("perception-journal");

    let dir = state_dir();
    let journal = match Journal::open(&dir) {
        Ok(j) => Arc::new(Mutex::new(j)),
        Err(e) => {
            // Nothing useful can be done without somewhere durable to write. Refusing to start is
            // the honest outcome: a journal that silently kept only what fit in memory would be
            // the ring again, wearing a different name.
            eprintln!("perception-journal: cannot open {}: {e}", dir.display());
            std::process::exit(1);
        }
    };
    let health = Arc::new(Mutex::new(Health::default()));
    tracing::info!(dir = %dir.display(), "Journal open");

    {
        let journal = journal.clone();
        let health = health.clone();
        let cursor_path = dir.join("cursor.json");
        std::thread::Builder::new()
            .name("journal-drain".into())
            .spawn(move || drain(journal, health, cursor_path))
            .ok();
    }

    ServiceBuilder::new("perception-journal").handler(Service { health, journal }).run();
}

fn drain(journal: Arc<Mutex<Journal>>, health: Arc<Mutex<Health>>, cursor_path: PathBuf) {
    let address = yantrik_ipc_transport::server::RpcServer::default_address("perception");
    let mut cursor = Cursor::load(&cursor_path);
    let mut last_progress = Instant::now();
    let mut announced = false;

    tracing::info!(next_seq = cursor.next_seq, source = %cursor.source, "Resuming");

    loop {
        let client = yantrik_ipc_transport::SyncRpcClient::new(&address)
            .with_timeout(WAIT + Duration::from_secs(10));

        let page = client.call(
            "perception.since",
            serde_json::json!({ "seq": cursor.next_seq, "wait_ms": WAIT.as_millis() as u64 }),
        );

        let Ok(page) = page else {
            if announced {
                tracing::warn!("The eye went away; retrying");
                announced = false;
                if let Ok(mut h) = health.lock() {
                    h.connected = false;
                }
            }
            std::thread::sleep(RETRY);
            continue;
        };

        if !announced {
            tracing::info!("Draining perception");
            announced = true;
        }

        let observations =
            page["observations"].as_array().cloned().unwrap_or_default();
        let next_seq = page["next_seq"].as_u64().unwrap_or(cursor.next_seq);
        let missed = page["missed"].as_u64().unwrap_or(0);

        {
            let mut j = match journal.lock() {
                Ok(j) => j,
                Err(_) => {
                    tracing::error!("Journal lock poisoned; stopping the drain rather than \
                                     writing into an unknown state");
                    return;
                }
            };

            // The hole goes in first, in order, so a reader meets it before the records that
            // follow it rather than after. It is written even though we cannot know what was in
            // it — that is the point. An unrecorded gap is a confident wrong answer about a
            // period when the machine was busy.
            if missed > 0 {
                tracing::warn!(
                    missed,
                    from = cursor.next_seq,
                    "The eye's ring overran before this drain read it; those observations are \
                     unrecoverable and the loss is being recorded in-band"
                );
                let _ = j.append(&Record::Gap {
                    from: cursor.next_seq,
                    to: cursor.next_seq + missed,
                    at: now(),
                    cause: "ring_overrun".into(),
                });
                if let Ok(mut h) = health.lock() {
                    h.observations_lost += missed;
                }
            }

            // Private mode: the person's activity is not recorded while it is on. The eye is still
            // drained, so nothing piles up to be written later, and what passed is one hole in
            // the journal that says why — never the observations themselves.
            let private = yantrik_ipc_transport::privacy::is_private();
            if private && next_seq > cursor.next_seq + missed {
                let _ = j.append(&Record::Gap {
                    from: cursor.next_seq + missed,
                    to: next_seq,
                    at: now(),
                    cause: "private".into(),
                });
            }

            for chunk in observations.chunks(BATCH).filter(|_| !private) {
                for o in chunk {
                    let seq = o["seq"].as_u64().unwrap_or(0);
                    let at = o["at"].as_f64().unwrap_or_else(now);
                    if let Err(e) = j.append(&Record::Observation {
                        seq,
                        source: cursor.source.clone(),
                        at,
                        observation: o.clone(),
                    }) {
                        // Do not advance the cursor past something that is not on disk. Losing
                        // the write and keeping the cursor would be the ring's failure mode with
                        // extra steps.
                        tracing::error!(error = %e, seq, "Append failed; not advancing the cursor");
                        std::thread::sleep(RETRY);
                        return;
                    }
                }
            }

            if let Ok(mut h) = health.lock() {
                h.connected = true;
                h.source_next_seq = next_seq;
                h.records = j.records;
                h.gaps = j.gaps;
                h.bytes = j.bytes_on_disk();
            }
        }

        // Durable first, cursor second. A crash in this window replays the tail, which is safe
        // because every record carries the eye's own sequence number.
        if next_seq != cursor.next_seq {
            cursor.next_seq = next_seq;
            if let Err(e) = cursor.save(&cursor_path) {
                tracing::error!(error = %e, "Cannot persist the cursor; will replay on restart");
            }
            last_progress = Instant::now();
        }

        if let Ok(mut h) = health.lock() {
            h.consumed_next_seq = cursor.next_seq;
            h.last_progress_secs_ago = last_progress.elapsed().as_secs();
        }
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
