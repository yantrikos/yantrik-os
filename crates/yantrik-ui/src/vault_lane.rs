//! The credential vault's own thread: every vault operation is answered here, never by the
//! companion's worker.
//!
//! VM 520, 30 Oct 2026 (the gate for #1026): a companion turn called a mail tool, the tool asked
//! email-service, email-service asked the shell's credential door (#1021), and the door read the
//! vault through the companion worker's command loop, which was busy running the very turn that
//! had started the call. The read waited its full 10 s and failed. Outside turns 72 of 72 calls
//! were fine; during turns 10 of 10 failed. The gateway's saved-provider keys (#1015) and the free
//! tiers' keys travelled the same road and would stall the same way.
//!
//! So the vault has a lane of its own:
//!
//! - **One thread, its own connection.** The thread owns a second SQLite connection to the memory
//!   store (the memory screen's reader is the precedent, `memory_reader`). The store is in WAL
//!   mode with a busy timeout, so this connection reads beside the worker's and its writes take
//!   turns with the worker's; neither waits for a turn, only for another statement.
//! - **The unlock state stays one thing.** The unwrapped key is the engine's process-wide
//!   `vault::UNLOCKED`, not a field of any connection or thread. Locking (`vault::lock()`, on the
//!   screen lock) is a call on that static, from any thread, and takes effect for the very next
//!   operation on every lane, the companion's own vault tools included.
//! - **Nothing else asks the worker.** The worker has no vault command at all
//!   (`the_worker_has_no_vault_command` pins it), so there is no path on which a vault call waits
//!   behind a turn.
//!
//! The typed operations are the ones the worker used to run ([`crate::vault_unlock::Op`] with a
//! passphrase in it, [`Op`] with a key), still reachable only from the shell's own bridge and not
//! from the handle the companion socket serves. Not on the job board and not logged, for the
//! reason the worker's arms were not.

use std::path::Path;
use std::time::Duration;

use crossbeam_channel::{bounded, unbounded, Sender};
use rusqlite::{Connection, OpenFlags};

use crate::vault_unlock;
use crate::wire::free_ai::store::{self, Op, Reply};

/// How long this lane waits for the store's write lock before it answers `Failed`. The same as
/// the engine's own connection.
const BUSY_MS: u64 = 5000;

/// One request. No `Debug`: a passphrase and a key travel in it.
enum Request {
    Keys { op: Op, reply_tx: Sender<Reply> },
    Vault { op: vault_unlock::Op, reply_tx: Sender<vault_unlock::Reply> },
}

/// A handle on the vault's thread. Cheap to hold; the thread ends when the last handle goes.
pub struct VaultLane {
    tx: Sender<Request>,
}

impl VaultLane {
    /// Start the lane for the memory store at `db_path`. The store need not exist yet: the
    /// connection is opened on the first request that finds it, and a request before then
    /// is answered as the vault being unavailable.
    pub fn start(db_path: &str) -> Self {
        let (tx, rx) = unbounded::<Request>();
        let path = db_path.to_string();
        let spawned = std::thread::Builder::new().name("vault-lane".into()).spawn(move || {
            let mut conn: Option<Connection> = None;
            for request in rx {
                if conn.is_none() {
                    conn = open(Path::new(&path));
                }
                match (request, conn.as_ref()) {
                    (Request::Keys { op, reply_tx }, Some(c)) => {
                        let _ = reply_tx.send(store::run(c, op));
                    }
                    (Request::Keys { reply_tx, .. }, None) => {
                        let _ = reply_tx.send(Reply::Failed);
                    }
                    (Request::Vault { op, reply_tx }, Some(c)) => {
                        let _ = reply_tx.send(vault_unlock::run(c, op));
                    }
                    // Dropping the sender is the answer: the caller's wait ends at once.
                    (Request::Vault { .. }, None) => {}
                }
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "the vault lane did not start");
        }
        Self { tx }
    }

    /// Keep, read, remove or list the OS's provider keys. Blocking; every caller is off the UI
    /// thread. `Err` means the lane did not answer within `timeout`.
    pub fn keys(&self, op: Op, timeout: Duration) -> Result<Reply, String> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx.send(Request::Keys { op, reply_tx }).map_err(|_| "the vault lane is not running".to_string())?;
        reply_rx.recv_timeout(timeout).map_err(|_| "the vault did not answer in time".to_string())
    }

    /// Protect, open, re-wrap or read the vault's state.
    pub fn vault(&self, op: vault_unlock::Op, timeout: Duration) -> Result<vault_unlock::Reply, String> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx.send(Request::Vault { op, reply_tx }).map_err(|_| "the vault lane is not running".to_string())?;
        reply_rx.recv_timeout(timeout).map_err(|_| "the vault did not answer in time".to_string())
    }
}

/// The lane's connection to the store, or `None` while there is no store to open. Never creates
/// the file: the companion's worker makes it, with its schema.
fn open(path: &Path) -> Option<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).ok()?;
    conn.busy_timeout(Duration::from_millis(BUSY_MS)).ok()?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;").ok()?;
    // Idempotent, and the worker makes the same tables at its start.
    yantrikdb_core::vault::init_tables(&conn);
    Some(conn)
}

#[cfg(test)]
mod tests;
