//! The vault answers while the companion is mid-turn, and every lane sees one lock state.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use yantrikdb_core::vault;

use super::*;
use crate::vault_unlock::{Op as VaultOp, Outcome, TEST_SERIAL};

/// Not a real key: a fixed string the tests can look for.
const FAKE_KEY: &str = "gsk_FAKE0000000000000000000000000000000000000000000000";
const FAKE_PASSPHRASE: &str = "fake passphrase for a test";
const PROMPT: Duration = Duration::from_secs(5);

fn serial() -> std::sync::MutexGuard<'static, ()> {
    TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// A real memory-store file in WAL mode with the vault's tables, as the worker makes it.
struct Store {
    path: PathBuf,
}

impl Store {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("yantrik-lane-{}-{nanos}-{tag}.db", std::process::id()));
        let conn = Connection::open(&path).expect("open the store");
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").expect("pragmas");
        vault::init_tables(&conn);
        Store { path }
    }

    /// The companion worker's own connection to the store.
    fn connect(&self) -> Connection {
        let conn = Connection::open(&self.path).expect("open the store");
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        conn
    }

    fn path(&self) -> &str {
        self.path.to_str().unwrap()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        remove_db(&self.path);
    }
}

fn remove_db(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

fn put(lane: &VaultLane, id: &str, value: &str) -> Reply {
    lane.keys(Op::Store { id: id.into(), value: value.into() }, PROMPT).expect("the lane answers")
}

fn read(lane: &VaultLane, id: &str) -> Reply {
    lane.keys(Op::Value { id: id.into() }, PROMPT).expect("the lane answers")
}

/// The bug of VM 520, 30 Oct: a read issued while a companion turn held the worker waited out its
/// 10 s and failed. The worker here is a thread in the middle of a turn, with a read transaction
/// open on its connection as a turn has; the lane must not wait for it.
#[test]
fn a_vault_read_does_not_wait_for_a_companion_turn() {
    let _g = serial();
    vault::lock();
    let store = Store::new("turn");
    let lane = VaultLane::start(store.path());
    assert_eq!(put(&lane, "groq", FAKE_KEY), Reply::Done);

    let turn_running = Arc::new(AtomicBool::new(true));
    let worker = {
        let (conn, running) = (store.connect(), turn_running.clone());
        std::thread::spawn(move || {
            conn.execute_batch("BEGIN; SELECT count(*) FROM vault_entries;").unwrap();
            std::thread::sleep(Duration::from_secs(3));
            conn.execute_batch("COMMIT").unwrap();
            running.store(false, Ordering::SeqCst);
        })
    };
    std::thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    let one = Duration::from_secs(1);
    let value = lane.keys(Op::Value { id: "groq".into() }, one).expect("answered inside a second");
    let tails = lane.keys(Op::Tails, one).expect("answered inside a second");
    let state = lane.vault(VaultOp::Read, one).expect("answered inside a second");
    let stored = lane.keys(Op::Store { id: "cloudflare".into(), value: "b".into() }, one).expect("a write too");

    assert!(turn_running.load(Ordering::SeqCst), "the turn was still running when the vault answered");
    assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    assert!(value == Reply::Value(Some(FAKE_KEY.into())));
    assert!(matches!(&tails, Reply::Tails(t) if t.get("groq").map(String::as_str) == Some("0000")));
    assert!(!state.status.protected);
    assert_eq!(stored, Reply::Done);
    worker.join().unwrap();
}

/// Readers on every road at once (the door, the gateway, the free tiers) are all answered.
#[test]
fn readers_on_every_road_are_answered_together() {
    let _g = serial();
    vault::lock();
    let store = Store::new("many");
    let lane = Arc::new(VaultLane::start(store.path()));
    assert_eq!(put(&lane, "groq", FAKE_KEY), Reply::Done);
    let readers: Vec<_> = (0..8)
        .map(|_| {
            let lane = lane.clone();
            std::thread::spawn(move || {
                lane.keys(Op::Value { id: "groq".into() }, Duration::from_secs(2)).map(|r| r == Reply::Value(Some(FAKE_KEY.into())))
            })
        })
        .collect();
    for r in readers {
        assert_eq!(r.join().unwrap(), Ok(true));
    }
}

/// One unlock state: locking is the engine's process-wide call, and the very next operation on
/// the lane, and on any other connection, sees it. Opening is seen by both as well.
#[test]
fn lock_and_unlock_are_seen_the_same_by_every_lane() {
    use yantrik_companion::tools::provider_keys as companion;
    let _g = serial();
    vault::lock();
    let store = Store::new("lock");
    let lane = VaultLane::start(store.path());

    let protected = lane.vault(VaultOp::Adopt(FAKE_PASSPHRASE.into()), PROMPT).expect("answered");
    assert!(matches!(protected.outcome, Some(Outcome::Protected)));
    assert_eq!(put(&lane, "groq", FAKE_KEY), Reply::Done);
    // The worker's own connection (as its vault tools use) agrees that it is open.
    let worker = store.connect();
    assert!(vault::is_protected(&worker) && vault::is_unlocked());
    assert!(companion::load_all(&worker).is_ok());

    vault_unlock::on_screen_lock();
    assert_eq!(read(&lane, "groq"), Reply::Locked, "the lane sees the lock on its next call");
    assert_eq!(put(&lane, "x", "y"), Reply::Locked);
    assert!(companion::load_all(&worker).is_err(), "so does the worker's connection");
    assert!(!lane.vault(VaultOp::Read, PROMPT).unwrap().status.unlocked);

    let opened = lane.vault(VaultOp::Open(FAKE_PASSPHRASE.into()), PROMPT).expect("answered");
    assert!(matches!(opened.outcome, Some(Outcome::Unlocked)));
    assert_eq!(read(&lane, "groq"), Reply::Value(Some(FAKE_KEY.into())));
    assert!(companion::load_all(&worker).is_ok(), "the worker sees it open");

    let wrong = lane.vault(VaultOp::Open("not the passphrase".into()), PROMPT).expect("answered");
    assert!(!matches!(wrong.outcome, Some(Outcome::Unlocked | Outcome::Protected)));
    vault::lock();
}

/// A key written on the lane is on the file for the worker's connection, and the reverse.
#[test]
fn the_lane_and_the_worker_share_one_store() {
    use yantrik_companion::tools::provider_keys as companion;
    let _g = serial();
    vault::lock();
    let store = Store::new("shared");
    let lane = VaultLane::start(store.path());
    assert_eq!(put(&lane, "groq", FAKE_KEY), Reply::Done);
    let worker = store.connect();
    let seen = companion::load_all(&worker).expect("readable");
    assert_eq!(seen.get("groq").map(String::as_str), Some(FAKE_KEY));
    companion::store(&worker, "gemini", FAKE_KEY).expect("stored by the worker");
    assert_eq!(read(&lane, "gemini"), Reply::Value(Some(FAKE_KEY.into())));
    assert_eq!(lane.keys(Op::Remove { id: "groq".into() }, PROMPT).unwrap(), Reply::Done);
    assert!(!companion::load_all(&worker).unwrap().contains_key("groq"));
}

/// No store yet (the worker has not made it): the lane says the vault is unavailable at once,
/// makes no file of its own, and answers once the store exists.
#[test]
fn a_lane_before_the_store_exists_waits_for_it() {
    let _g = serial();
    vault::lock();
    let store = Store::new("late");
    let late = std::env::temp_dir().join(format!("yantrik-lane-late-{}.db", std::process::id()));
    remove_db(&late);
    let lane = VaultLane::start(late.to_str().unwrap());
    assert_eq!(lane.keys(Op::Tails, Duration::from_secs(1)).unwrap(), Reply::Failed);
    assert!(lane.vault(VaultOp::Read, Duration::from_secs(1)).is_err());
    assert!(!late.exists(), "the lane never creates the store");

    std::fs::copy(&store.path, &late).unwrap();
    assert_eq!(put(&lane, "groq", FAKE_KEY), Reply::Done);
    remove_db(&late);
}

#[test]
fn a_value_read_back_never_reaches_a_debug_line() {
    let reply = Reply::Value(Some(FAKE_KEY.into()));
    assert!(!format!("{reply:?}").contains(FAKE_KEY));
}

/// The worker has no vault command, so there is no road on which a vault call queues behind a
/// turn; the bridge's two vault methods go to the lane, and only the lane runs the operations.
#[test]
fn the_worker_has_no_vault_command() {
    let source = include_str!("../bridge.rs");
    let code = source.split("#[cfg(test)]").next().unwrap_or(source);
    for banned in ["CompanionCommand::Vault", "CompanionCommand::ProviderKeys", "vault_unlock::run(", "store::run("] {
        assert!(!code.contains(banned), "the worker handles `{banned}` again");
    }
    assert!(code.contains("self.vault_lane.keys(op, timeout)") && code.contains("self.vault_lane.vault(op, timeout)"));

    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits);
                continue;
            }
            let name = path.to_string_lossy().replace('\\', "/");
            if !name.ends_with(".rs") || name.ends_with("/vault_lane.rs") || name.contains("/tests") || name.contains("/bin/") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or(&text);
            if code.contains("vault_unlock::run(") || code.contains("store::run(") {
                hits.push(name);
            }
        }
    }
    let mut hits = Vec::new();
    walk(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
    assert!(hits.is_empty(), "vault operations run outside the lane in {hits:?}");
}
