//! The ordered persistence lane (design/app-framework-2026-10-21.md, §6).
//!
//! Every durable write an app asks for (its settings, a draft) goes through one lane, off the
//! UI thread, serialised per destination and tagged with the revision it carries. An older
//! write never lands after a newer one: when a newer revision for the same key is already
//! waiting, the older is skipped. A write may carry a delay (a draft's debounce): it waits that
//! long, and a newer write of the same key arriving meanwhile takes its place, so a burst of
//! changes is one write. A drain writes everything owed at once, delays or not. What came of
//! each write returns to the core as a [`Written`], and the core applies it by revision, so an
//! old outcome never replaces newer state. Workers never write these files themselves.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use yantrik_foundation::json_store::write_atomically;

/// What is being written: a settings file, a draft by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PersistKey {
    Settings,
    Draft(String),
}

/// What to do at the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Replace the file with these bytes, whole or not at all.
    Put(Vec<u8>),
    /// Remove the file; one that is already gone is fine.
    Remove,
}

/// One write, as the lane is given it.
pub struct Write {
    pub key: PersistKey,
    pub revision: u64,
    pub path: PathBuf,
    pub op: Op,
    /// How long to hold it for a newer one to take its place. Zero for a settings save.
    pub delay: Duration,
}

/// What came of one write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub key: PersistKey,
    pub revision: u64,
    /// The write was a removal (a draft discarded), not a file put.
    pub removed: bool,
    pub result: Result<(), String>,
}

/// The sentence a skipped write is reported with.
pub const SUPERSEDED: &str = "superseded";

/// A delayed write is held at most this long past the first revision of its key that waited: a
/// burst of changes that never pauses for its debounce is still written, so what is kept never
/// depends on the burst ending (design §6, "recovery never depends on a shutdown hook").
pub const MAX_HOLD: Duration = Duration::from_secs(5);

/// Where outcomes go: the core's queue in the runner, a channel in a test.
pub trait Report: Send + Sync {
    fn written(&self, written: Written);
}

/// The lane as the core drives it. [`Threaded`] in the runner; [`ManualLane`] in a test.
pub trait Lane: Send {
    fn write(&mut self, write: Write);
    /// Writes not yet done.
    fn waiting(&self) -> usize;
    /// Does every write owed, delays or not, within `grace`. Says whether it did.
    fn drain(&self, grace: Duration) -> bool;
}

/// A queued write and when it may go.
struct Queued {
    write: Write,
    due: Instant,
}

/// The writes still queued, newest revision per key winning.
#[derive(Default)]
struct Queue {
    writes: Vec<Queued>,
    newest: HashMap<PersistKey, u64>,
    /// When a key's first still-waiting delayed write arrived: later ones of the key may not
    /// push its due past `held + MAX_HOLD`. Cleared when a write of the key is done.
    held: HashMap<PersistKey, Instant>,
}

/// What the queue has for the thread.
enum Next {
    Empty,
    /// The earliest due is not due yet.
    Wait(Duration),
    Ready(Write),
}

impl Queue {
    fn push(&mut self, write: Write) {
        let newest = self.newest.entry(write.key.clone()).or_insert(0);
        if write.revision > *newest {
            *newest = write.revision;
        }
        let now = Instant::now();
        let mut due = now + write.delay;
        if write.delay > Duration::ZERO {
            let held = *self.held.entry(write.key.clone()).or_insert(now);
            due = due.min(held + MAX_HOLD);
        }
        self.writes.push(Queued { write, due });
    }

    /// The earliest due write still worth doing, whatever its key: a settings save never waits
    /// behind a draft's debounce. Stale ones are skipped first, each reported as superseded. With
    /// `now_regardless`, a delay is not waited for.
    fn next(&mut self, now_regardless: bool, report: &dyn Report) -> Next {
        let mut i = 0;
        while i < self.writes.len() {
            let newest = self.newest.get(&self.writes[i].write.key).copied().unwrap_or(0);
            if self.writes[i].write.revision < newest {
                let stale = self.writes.remove(i).write;
                report.written(Written {
                    key: stale.key,
                    revision: stale.revision,
                    removed: matches!(stale.op, Op::Remove),
                    result: Err(SUPERSEDED.into()),
                });
            } else {
                i += 1;
            }
        }
        let Some(idx) = (0..self.writes.len()).min_by_key(|&i| self.writes[i].due) else {
            return Next::Empty;
        };
        let now = Instant::now();
        if !now_regardless && self.writes[idx].due > now {
            return Next::Wait(self.writes[idx].due - now);
        }
        let write = self.writes.remove(idx).write;
        self.held.remove(&write.key);
        Next::Ready(write)
    }
}

fn perform(write: Write, report: &dyn Report) {
    let removed = matches!(write.op, Op::Remove);
    let result = match write.op {
        // A settings save replaces only a file with the app's own mark, or nothing: a file that
        // arrived after the save was queued and is not this app's is left as it is.
        Op::Put(bytes) if write.key == PersistKey::Settings => crate::settings::file::replace(&write.path, &bytes),
        Op::Put(bytes) => write_atomically(&write.path, &bytes).map_err(|e| e.to_string()),
        Op::Remove => match std::fs::remove_file(&write.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        },
    };
    report.written(Written {
        key: write.key,
        revision: write.revision,
        removed,
        result,
    });
}

/// One thread per app, named `<id>-persist`.
pub struct Threaded {
    queue: Arc<Mutex<Queue>>,
    sender: mpsc::Sender<()>,
    /// Writes taken from the queue and not yet finished.
    in_flight: Arc<AtomicUsize>,
    /// Set by `drain`: delays are over, everything owed goes now. Never cleared: a drain happens
    /// once, at shutdown.
    flushing: Arc<AtomicBool>,
}

impl Threaded {
    pub fn new(name: &str, report: Arc<dyn Report>) -> Threaded {
        let queue = Arc::new(Mutex::new(Queue::default()));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let flushing = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel::<()>();
        let (q, f, busy) = (queue.clone(), flushing.clone(), in_flight.clone());
        std::thread::Builder::new()
            .name(format!("{name}-persist"))
            .spawn(move || loop {
                let next = {
                    let mut queue = q.lock().unwrap_or_else(|p| p.into_inner());
                    let next = queue.next(f.load(Ordering::Acquire), report.as_ref());
                    // Counted as in flight before the lock drops, so `waiting` never sees a
                    // write that is neither queued nor in flight and lets a drain return before
                    // it ran.
                    if matches!(next, Next::Ready(_)) {
                        busy.fetch_add(1, Ordering::AcqRel);
                    }
                    next
                };
                match next {
                    Next::Ready(write) => {
                        // A panic in a write is this thread's end; the lane reports it as a
                        // failure rather than dying silently.
                        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            perform(write, report.as_ref())
                        }));
                        if let Err(what) = outcome {
                            tracing::error!(?what, "a write panicked");
                        }
                        busy.fetch_sub(1, Ordering::AcqRel);
                    }
                    Next::Wait(for_) => {
                        // A newer write or a drain wakes it sooner.
                        if matches!(receiver.recv_timeout(for_), Err(mpsc::RecvTimeoutError::Disconnected)) {
                            return;
                        }
                    }
                    Next::Empty => {
                        if receiver.recv().is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("the persist thread");
        Threaded {
            queue,
            sender,
            in_flight,
            flushing,
        }
    }
}

impl Lane for Threaded {
    fn write(&mut self, write: Write) {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(write);
        let _ = self.sender.send(());
    }

    fn waiting(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .writes
            .len()
            + self.in_flight.load(Ordering::Acquire)
    }

    fn drain(&self, grace: Duration) -> bool {
        self.flushing.store(true, Ordering::Release);
        let _ = self.sender.send(());
        let deadline = Instant::now() + grace;
        while self.waiting() > 0 {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
}

/// Writes only when a test says so, in order, on the calling thread. Delays are the test's to
/// honour: `write_one` and `write_all` do not wait.
pub struct ManualLane {
    queue: Arc<Mutex<Queue>>,
    report: Arc<dyn Report>,
}

impl ManualLane {
    pub fn new(report: Arc<dyn Report>) -> ManualLane {
        ManualLane {
            queue: Arc::new(Mutex::new(Queue::default())),
            report,
        }
    }

    /// Queues a write through a shared handle (the queue is behind a lock).
    pub fn push(&self, write: Write) {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(write);
    }

    /// Does the oldest write still worth doing. Says whether there was one.
    pub fn write_one(&self) -> bool {
        let next = self
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .next(true, self.report.as_ref());
        match next {
            Next::Ready(write) => {
                perform(write, self.report.as_ref());
                true
            }
            Next::Wait(_) | Next::Empty => false,
        }
    }

    pub fn write_all(&self) {
        while self.write_one() {}
    }
}

impl Lane for ManualLane {
    fn write(&mut self, write: Write) {
        self.push(write);
    }

    fn waiting(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .writes
            .len()
    }

    /// A test's shutdown does the writes it owes, as the thread would.
    fn drain(&self, _grace: Duration) -> bool {
        self.write_all();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Log(Mutex<Vec<Written>>);
    impl Report for Log {
        fn written(&self, w: Written) {
            self.0.lock().unwrap().push(w);
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-persist-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn put(key: PersistKey, revision: u64, path: &std::path::Path, bytes: &[u8], delay: Duration) -> Write {
        Write {
            key,
            revision,
            path: path.to_path_buf(),
            op: Op::Put(bytes.to_vec()),
            delay,
        }
    }

    /// A settings file's bytes, as the store renders them: marked as app `t`'s.
    fn marked(text: &str) -> Vec<u8> {
        crate::settings::file::render_raw("t", 1, serde_json::json!(text)).unwrap()
    }

    /// A settings save (no delay) goes before a draft that is still debouncing, whichever was
    /// queued first; and a key whose debounce keeps restarting is still written once its hold is up.
    #[test]
    fn the_earliest_due_goes_first_and_a_restless_key_is_written_within_the_hold() {
        let log = Arc::new(Log(Mutex::new(Vec::new())));
        let d = dir("due");
        let mut queue = Queue::default();
        queue.push(put(PersistKey::Draft("t".into()), 1, &d.join("t"), b"a", Duration::from_millis(600)));
        queue.push(put(PersistKey::Settings, 1, &d.join("s.json"), b"s", Duration::ZERO));
        match queue.next(false, log.as_ref()) {
            Next::Ready(w) => assert_eq!(w.key, PersistKey::Settings),
            _ => panic!("the settings save was due now"),
        }
        assert!(matches!(queue.next(false, log.as_ref()), Next::Wait(_)), "the draft still waits");
        // The draft's debounce has been restarting for longer than the hold: the next revision
        // is due now, not 600 ms from now.
        *queue.held.get_mut(&PersistKey::Draft("t".into())).unwrap() = Instant::now() - MAX_HOLD;
        queue.push(put(PersistKey::Draft("t".into()), 2, &d.join("t"), b"ab", Duration::from_millis(600)));
        match queue.next(false, log.as_ref()) {
            Next::Ready(w) => assert_eq!((w.key, w.revision), (PersistKey::Draft("t".into()), 2)),
            _ => panic!("the hold was up"),
        }
        assert!(queue.held.is_empty(), "a done key holds nothing");
        let superseded: Vec<u64> = log.0.lock().unwrap().iter().map(|w| w.revision).collect();
        assert_eq!(superseded, vec![1]);
    }

    #[test]
    fn writes_land_in_order_and_a_stale_one_is_skipped() {
        let log = Arc::new(Log(Mutex::new(Vec::new())));
        let path = dir("order").join("s.json");
        let mut lane = ManualLane::new(log.clone());
        lane.write(put(PersistKey::Settings, 1, &path, &marked("one"), Duration::ZERO));
        lane.write(put(PersistKey::Settings, 2, &path, &marked("two"), Duration::ZERO));
        lane.write(put(PersistKey::Draft("a".into()), 1, &path.with_extension("draft"), b"d", Duration::ZERO));
        lane.write_all();
        assert_eq!(std::fs::read(&path).unwrap(), marked("two"));
        let outcomes: Vec<(u64, bool)> = log.0.lock().unwrap().iter().map(|w| (w.revision, w.result.is_ok())).collect();
        assert_eq!(outcomes, vec![(1, false), (2, true), (1, true)], "revision 1 of the settings was superseded, not written");
        assert_eq!(log.0.lock().unwrap()[0].result, Err(SUPERSEDED.into()));
    }

    #[test]
    fn a_remove_takes_the_file_away_and_an_absent_one_is_fine() {
        let log = Arc::new(Log(Mutex::new(Vec::new())));
        let path = dir("remove").join("d.json");
        let mut lane = ManualLane::new(log.clone());
        lane.write(put(PersistKey::Draft("d".into()), 1, &path, b"x", Duration::ZERO));
        lane.write(Write { key: PersistKey::Draft("d".into()), revision: 2, path: path.clone(), op: Op::Remove, delay: Duration::ZERO });
        lane.write(Write { key: PersistKey::Draft("d".into()), revision: 3, path: path.clone(), op: Op::Remove, delay: Duration::ZERO });
        lane.write_all();
        assert!(!path.exists());
        assert!(log.0.lock().unwrap().iter().all(|w| w.result.is_ok() || w.result == Err(SUPERSEDED.into())));
    }

    #[test]
    fn the_thread_writes_and_drains() {
        let log = Arc::new(Log(Mutex::new(Vec::new())));
        let path = dir("thread").join("s.json");
        let mut lane = Threaded::new("test", log.clone());
        lane.write(put(PersistKey::Settings, 1, &path, &marked("x"), Duration::ZERO));
        assert!(lane.drain(Duration::from_secs(5)));
        assert_eq!(std::fs::read(&path).unwrap(), marked("x"));
        assert_eq!(log.0.lock().unwrap().len(), 1);
    }

    /// A file that arrived after a settings save was queued, and is not the app's (another
    /// component's, by name or by link), is not written over: the save fails and says the path.
    #[test]
    fn a_settings_save_never_lands_over_a_file_that_is_not_the_apps() {
        let log = Arc::new(Log(Mutex::new(Vec::new())));
        let path = dir("guard").join("s.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let account = r#"[{"password":"SENTINEL-PASSWORD-3"}]"#;
        std::fs::write(&path, account).unwrap();
        let mut lane = ManualLane::new(log.clone());
        lane.write(put(PersistKey::Settings, 1, &path, &marked("mine"), Duration::ZERO));
        lane.write_all();
        // Bytes without a mark are not a settings save the store made: refused even onto nothing.
        let bare = path.with_file_name("bare.json");
        lane.write(put(PersistKey::Settings, 2, &bare, b"{}", Duration::ZERO));
        lane.write_all();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), account);
        assert!(!bare.exists());
        let said: Vec<String> = log.0.lock().unwrap().iter().filter_map(|w| w.result.clone().err()).collect();
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said.iter().all(|e| !e.contains("SENTINEL") && e.contains("left as it is")), "{said:?}");
    }

    #[test]
    fn a_delayed_write_waits_and_a_burst_is_one_write() {
        let log = Arc::new(Log(Mutex::new(Vec::new())));
        let path = dir("delay").join("d.json");
        let mut lane = Threaded::new("test-delay", log.clone());
        let key = || PersistKey::Draft("d".into());
        lane.write(put(key(), 1, &path, b"h", Duration::from_millis(300)));
        lane.write(put(key(), 2, &path, b"he", Duration::from_millis(300)));
        lane.write(put(key(), 3, &path, b"hel", Duration::from_millis(300)));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!path.exists(), "nothing lands before the delay");
        let deadline = Instant::now() + Duration::from_secs(5);
        while lane.waiting() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hel");
        let ok: Vec<u64> = log.0.lock().unwrap().iter().filter(|w| w.result.is_ok()).map(|w| w.revision).collect();
        assert_eq!(ok, vec![3], "one write for the burst");
        // A drain does not wait for a delay.
        lane.write(put(key(), 4, &path, b"hello", Duration::from_secs(30)));
        let started = Instant::now();
        assert!(lane.drain(Duration::from_secs(5)));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }
}
