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
//! - **One key, however many connections.** The data key is made by one atomic statement
//!   (`yantrik_companion::tools::vault_key`) that the worker runs at start, that this lane runs
//!   when it opens, and that every key-using call runs first. `Adopt` and `Rewrap` run inside
//!   `BEGIN IMMEDIATE`, so a passphrase change is all-or-nothing and nothing can mint a key in
//!   the middle of it (review of #1032).
//! - **It comes back.** A panic is caught, the request that caused it is dropped (its caller is
//!   told at once), and the lane starts again after a wait that doubles to a cap.
//! - **Nothing else asks the worker.** The worker has no vault command at all
//!   (`the_worker_has_no_vault_command` pins it), so there is no path on which a vault call waits
//!   behind a turn.
//!
//! The typed operations are the ones the worker used to run ([`crate::vault_unlock::Op`] with a
//! passphrase in it, [`Op`] with a key), still reachable only from the shell's own bridge and not
//! from the handle the companion socket serves. Not on the job board and not logged, for the
//! reason the worker's arms were not.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use rusqlite::{Connection, OpenFlags};

use crate::vault_unlock::{self, Outcome};
use crate::wire::free_ai::store::{self, Op, Reply};

/// How long this lane waits for the store's write lock before it answers `Failed`. The same as
/// the engine's own connection.
const BUSY_MS: u64 = 5000;

/// How often a request that finds no store looks again, until its caller's deadline.
const STORE_POLL: Duration = Duration::from_millis(200);

/// The wait before the lane starts again after a panic, doubling to the cap.
const RESTART_FIRST: Duration = Duration::from_millis(100);
const RESTART_CAP: Duration = Duration::from_secs(5);

/// What a caller is told when the lane is not there.
const DOWN: &str = "the vault is restarting, try again in a moment";

/// One request. No `Debug`: a passphrase and a key travel in it.
enum Request {
    Keys { op: Op, reply_tx: Sender<Reply>, deadline: Instant },
    Vault { op: vault_unlock::Op, reply_tx: Sender<vault_unlock::Reply>, deadline: Instant },
    /// Panics the lane, to test that it comes back.
    #[cfg(test)]
    Poison,
}

/// A handle on the vault's thread. Cheap to hold; the thread ends when the last handle goes.
pub struct VaultLane {
    tx: Sender<Request>,
}

impl VaultLane {
    /// Start the lane for the memory store at `db_path`. The store need not exist yet: a request
    /// that finds none waits for it until its own deadline.
    pub fn start(db_path: &str) -> Self {
        let (tx, rx) = unbounded::<Request>();
        let path = db_path.to_string();
        let spawned = std::thread::Builder::new().name("vault-lane".into()).spawn(move || supervise(&path, &rx));
        if let Err(e) = spawned {
            tracing::error!(error = %e, "the vault lane did not start");
        }
        Self { tx }
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let _ = self.tx.send(Request::Poison);
    }

    /// Keep, read, remove or list the OS's provider keys. Blocking; every caller is off the UI
    /// thread. `Err` means the lane did not answer within `timeout`.
    pub fn keys(&self, op: Op, timeout: Duration) -> Result<Reply, String> {
        let (reply_tx, reply_rx) = bounded(1);
        let deadline = Instant::now() + timeout;
        self.tx.send(Request::Keys { op, reply_tx, deadline }).map_err(|_| DOWN.to_string())?;
        reply_rx.recv_timeout(timeout).map_err(|_| "the vault did not answer in time".to_string())
    }

    /// Protect, open, re-wrap or read the vault's state.
    pub fn vault(&self, op: vault_unlock::Op, timeout: Duration) -> Result<vault_unlock::Reply, String> {
        let (reply_tx, reply_rx) = bounded(1);
        let deadline = Instant::now() + timeout;
        self.tx.send(Request::Vault { op, reply_tx, deadline }).map_err(|_| DOWN.to_string())?;
        reply_rx.recv_timeout(timeout).map_err(|_| "the vault did not answer in time".to_string())
    }
}

/// Run the lane, and run it again after a panic, with a wait that doubles. The request that
/// panicked is dropped, which tells its caller at once; the next one meets a fresh connection.
fn supervise(path: &str, rx: &Receiver<Request>) {
    let mut wait = RESTART_FIRST;
    loop {
        let started = Instant::now();
        if catch_unwind(AssertUnwindSafe(|| serve(path, rx))).is_ok() {
            return; // every handle is gone
        }
        // Logged without the panic's text: nothing about a request is printed.
        tracing::error!("the vault lane stopped and is starting again");
        if started.elapsed() > RESTART_CAP {
            wait = RESTART_FIRST;
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(RESTART_CAP);
    }
}

fn serve(path: &str, rx: &Receiver<Request>) {
    let mut conn: Option<Connection> = None;
    for request in rx {
        let deadline = match &request {
            Request::Keys { deadline, .. } | Request::Vault { deadline, .. } => *deadline,
            #[cfg(test)]
            Request::Poison => panic!("a poisoned request"),
        };
        // No store yet (the worker makes it after the model loads): wait for it, up to the
        // caller's own patience.
        while conn.is_none() {
            conn = open(Path::new(path));
            if conn.is_some() || Instant::now() + STORE_POLL > deadline {
                break;
            }
            std::thread::sleep(STORE_POLL);
        }
        match (request, conn.as_ref()) {
            (Request::Keys { op, reply_tx, .. }, Some(c)) => {
                let _ = reply_tx.send(store::run(c, op));
            }
            (Request::Keys { reply_tx, .. }, None) => {
                let _ = reply_tx.send(Reply::Failed);
            }
            (Request::Vault { op, reply_tx, .. }, Some(c)) => {
                let _ = reply_tx.send(run_vault(c, op));
            }
            // Dropping the sender is the answer: the caller's wait ends at once.
            (Request::Vault { .. }, None) => {}
            #[cfg(test)]
            (Request::Poison, _) => {}
        }
    }
}

/// A vault operation. Those that write (protecting, re-wrapping) run in one `BEGIN IMMEDIATE`
/// transaction: the engine makes five separate writes, and a busy time-out between them left a
/// clear key beside a vault that said it was protected. All of them land, or none do and the vault
/// is shut again. Opening an already protected vault writes nothing and needs no transaction.
fn run_vault(conn: &Connection, op: vault_unlock::Op) -> vault_unlock::Reply {
    let writes = match &op {
        vault_unlock::Op::Adopt(_) => !yantrikdb_core::vault::is_protected(conn),
        vault_unlock::Op::Rewrap { .. } => true,
        vault_unlock::Op::Open(_) | vault_unlock::Op::Read => false,
    };
    if !writes {
        return vault_unlock::run(conn, op);
    }
    if conn.execute_batch("BEGIN IMMEDIATE").is_err() {
        return failed(conn, "the vault is busy, try again");
    }
    let reply = vault_unlock::run(conn, op);
    if reply.outcome.as_ref().is_some_and(Outcome::opened) {
        if conn.execute_batch("COMMIT").is_ok() {
            return reply;
        }
        let _ = conn.execute_batch("ROLLBACK");
        yantrikdb_core::vault::lock();
        return failed(conn, "the vault could not be saved, try again");
    }
    let _ = conn.execute_batch("ROLLBACK");
    // The engine sets the key in memory only after its writes, but a failure partway leaves
    // nothing to trust: shut.
    if matches!(reply.outcome, Some(Outcome::Unusable(_))) {
        yantrikdb_core::vault::lock();
    }
    vault_unlock::Reply { outcome: reply.outcome, status: vault_unlock::status(conn) }
}

fn failed(conn: &Connection, why: &str) -> vault_unlock::Reply {
    vault_unlock::Reply { outcome: Some(Outcome::Unusable(why.to_string())), status: vault_unlock::status(conn) }
}

/// The lane's connection to the store, or `None` while there is none to open. Never creates the
/// file: the companion's worker makes it, with its schema.
fn open(path: &Path) -> Option<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).ok()?;
    conn.busy_timeout(Duration::from_millis(BUSY_MS)).ok()?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;").ok()?;
    // Idempotent, and the worker makes the same tables at its start.
    yantrikdb_core::vault::init_tables(&conn);
    yantrik_companion::tools::vault_key::ensure(&conn);
    Some(conn)
}

#[cfg(test)]
mod tests;
