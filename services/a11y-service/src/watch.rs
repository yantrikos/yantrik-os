//! Listening, so nobody has to look (#1256).
//!
//! Every application with an accessibility bridge announces what changes in it: a window opened,
//! the focus moved, text changed. Screen readers live on those announcements. This thread
//! subscribes to them once and keeps the [`Screen`] model current, so "what is on screen" is
//! answered from memory and "what changed" is a wait on a ring, not a walk of anyone's tree.
//!
//! # Two threads, and why
//!
//! `a11y-signals` only reads the bus and sorts each signal into a [`Raw`] (`signals`): no calls,
//! no locks, nothing that can stall. `a11y-watch`, here, takes those and does the work that needs
//! a question asked of the application (what is this new window called, which window is this
//! element in). A slow application then delays the model, never the reading, and when the queue
//! between them fills the overflow is counted and told as a gap instead of backing up into the
//! bus.
//!
//! # What is asked for
//!
//! Applications emit only what some listener registered for, so the list in [`EVENTS`] is also a
//! cost on every application: it is kept to window life, focus, and "something changed here".
//!
//! # Private mode
//!
//! The model keeps following the screen while the person is private, because it has to be right
//! the moment they are not. Nothing is *told* meanwhile: an event that would be recorded is
//! counted and dropped, so nothing the person did while private is in the ring afterwards, and
//! the service refuses `a11y.now` and `a11y.since` for as long as it lasts.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use yantrik_service_sdk::ring::CursorRing;
use zbus::zvariant::ObjectPath;

use crate::atspi::Atspi;
use crate::handles::{self, Reference};
use crate::screen::{Fact, Key, Kind, Screen, What};
use crate::signals::{self, is_root, Raw, Tally};

/// What the applications are asked to announce. Each entry costs every application a little, so
/// each one has to earn its place.
const EVENTS: [&str; 8] = [
    "window:create",
    "window:destroy",
    "window:activate",
    "window:deactivate",
    "object:state-changed:focused",
    "object:children-changed",
    "object:text-changed",
    "object:property-change:accessible-name",
];

/// Events kept for readers that have not caught up.
const RING: usize = 1024;

/// Events handed back by one `a11y.since` at most. A reader that is further behind asks again.
const MAX_PAGE: usize = 256;

/// Signals the bus connection may hold for the reader thread.
const SIGNAL_QUEUE: usize = 256;

/// Sorted signals waiting for the watcher. Past this they are counted and told as a gap.
const RAW_QUEUE: usize = 4096;

/// The longest one question to an application may take. An application that does not answer
/// costs the model that one fact, not its liveness.
const CALL_LIMIT: Duration = Duration::from_millis(400);

/// The longest a full listing of every window may take.
const LIST_LIMIT: Duration = Duration::from_secs(5);

/// How long the watcher sleeps when nothing is waiting to be told.
const IDLE: Duration = Duration::from_secs(5);

/// Levels climbed at most to find the window an element is in.
const MAX_HOPS: usize = 24;

/// Climbs allowed per second. Past it a change is reported for its application only.
const WALKS_PER_SECOND: u32 = 40;

/// Elements whose window is remembered. Cleared whole when full, and when any window opens or
/// closes.
const MAX_REMEMBERED: usize = 4096;

/// The longest pause between attempts to reach the accessibility bus.
const RETRY_CAP: Duration = Duration::from_secs(30);

/// One change, as the ring holds it.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub seq: u64,
    /// When it was told, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Default)]
struct Meters {
    listening: AtomicBool,
    dropped_private: AtomicU64,
    /// The signal reader's counts, kept across reconnections.
    signals: Arc<Tally>,
}

/// The reader's side of the watcher: the model and the ring.
#[derive(Clone)]
pub struct Sight {
    screen: Arc<Mutex<Screen>>,
    ring: CursorRing<Event>,
    meters: Arc<Meters>,
}

impl Sight {
    /// A model nothing feeds: what a test hands the service.
    pub fn idle() -> Self {
        Self { screen: Arc::default(), ring: CursorRing::new(RING), meters: Arc::default() }
    }

    /// Start listening on a thread of its own. Returns at once; the bus is connected to lazily
    /// and retried, because accessibility often starts after the session does.
    pub fn start(handles: handles::Shared, private: fn() -> bool) -> Self {
        let sight = Self::idle();
        let feed = sight.clone();
        let spawned = std::thread::Builder::new().name("a11y-watch".into()).spawn(move || watch_thread(feed, handles, private));
        if let Err(e) = spawned {
            tracing::error!(error = %e, "No watcher thread; the screen model will stay empty");
        }
        sight
    }

    fn screen(&self) -> MutexGuard<'_, Screen> {
        self.screen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Put what the model said into the ring, unless the person is private. A window's handle
    /// is kept for as long as the window is open and forgotten when it closes, private or not.
    fn record(&self, handles: &handles::Shared, private: bool, told: Vec<Kind>) {
        for kind in told {
            match &kind {
                Kind::WindowOpened { window } => handles::pin(handles, &window.id),
                Kind::WindowClosed { window } => handles::forget(handles, &window.id),
                _ => {}
            }
            if private {
                self.meters.dropped_private.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            self.ring.push_with(|seq| Event { seq, at_ms: now_ms(), kind });
        }
    }

    /// What is on screen, from the model: no application is asked anything.
    pub fn now(&self) -> serde_json::Value {
        let listening = self.meters.listening.load(Ordering::Relaxed);
        let screen = self.screen();
        serde_json::json!({
            // The cursor for `a11y.since`: everything after this answer.
            "seq": self.ring.next_seq(),
            // False means the model is not being fed (no accessibility bus yet, or it went
            // away): what follows is the last it knew, and `a11y.windows` is the way to look.
            "listening": listening,
            "front": screen.front(),
            "focus": screen.focus(),
            "windows": screen.windows().collect::<Vec<_>>(),
            "counts": {
                "model": screen.counts(),
                "signals": self.meters.signals.seen.load(Ordering::Relaxed),
                "signals_dropped": self.meters.signals.dropped.load(Ordering::Relaxed),
                "dropped_private": self.meters.dropped_private.load(Ordering::Relaxed),
                "overwritten": self.ring.overwritten(),
            },
        })
    }

    /// What changed from `seq` on, waiting up to `wait` for the first change.
    pub fn since(&self, seq: u64, wait: Duration) -> serde_json::Value {
        let mut page = self.ring.since(seq, wait);
        let more = page.items.len() > MAX_PAGE;
        page.items.truncate(MAX_PAGE);
        let next_seq = if more { page.items.last().map_or(page.next_seq, |e| e.seq + 1) } else { page.next_seq };
        serde_json::json!({
            "events": page.items,
            "next_seq": next_seq,
            // Non-zero: this reader fell off the back of the ring, and should read `a11y.now`.
            "missed": page.missed,
            "more": more,
        })
    }
}

fn key_of(reference: &Reference) -> Key {
    (reference.0.clone(), reference.1.as_str().to_string())
}

fn reference_of(key: &Key) -> Option<Reference> {
    ObjectPath::try_from(key.1.as_str()).ok().map(|path| (key.0.clone(), path.into()))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A budget of climbs per second.
struct Walks {
    left: u32,
    refills_at: Instant,
}

impl Walks {
    fn take(&mut self, now: Instant) -> bool {
        if now >= self.refills_at {
            self.left = WALKS_PER_SECOND;
            self.refills_at = now + Duration::from_secs(1);
        }
        let allowed = self.left > 0;
        self.left = self.left.saturating_sub(1);
        allowed
    }
}

/// The watcher's own state for one connection to the bus.
struct Watcher<'a> {
    runtime: &'a tokio::runtime::Runtime,
    atspi: Atspi,
    sight: &'a Sight,
    handles: &'a handles::Shared,
    private: fn() -> bool,
    /// Which window an element is in, once found. `None` is remembered too: an element that
    /// climbs to no known window is not climbed again on every signal.
    remembered: HashMap<Key, Option<Key>>,
    walks: Walks,
}

impl Watcher<'_> {
    /// Ask an application one thing, for at most `limit`.
    fn ask<T>(&self, limit: Duration, question: impl Future<Output = T>) -> Option<T> {
        self.runtime.block_on(async { tokio::time::timeout(limit, question).await.ok() })
    }

    fn apply(&mut self, fact: Fact) {
        let told = self.sight.screen().apply(fact, Instant::now());
        self.tell(told);
    }

    fn flush(&mut self) {
        let told = self.sight.screen().flush(Instant::now());
        self.tell(told);
    }

    fn tell(&mut self, told: Vec<Kind>) {
        if told.is_empty() {
            return;
        }
        // A window that came or went changes which window an element climbs to: an element
        // that announced itself before its window did was remembered as being in none.
        if told.iter().any(|kind| matches!(kind, Kind::WindowOpened { .. } | Kind::WindowClosed { .. })) {
            self.remembered.clear();
        }
        self.sight.record(self.handles, (self.private)(), told);
    }

    /// Reconcile the model to a walk of every window: at the start, after a gap, and when an
    /// application left without saying which windows it took.
    fn relist(&mut self) {
        match self.ask(LIST_LIMIT, self.atspi.windows()) {
            Some(Ok(windows)) => {
                let listing = windows.into_iter().map(|(reference, window)| (key_of(&reference), window)).collect();
                self.apply(Fact::Listing(listing));
            }
            Some(Err(why)) => tracing::debug!(%why, "Could not list the windows"),
            None => tracing::debug!("Listing the windows took too long; the model keeps what it had"),
        }
    }

    /// Which window is in front, asked once at the start. Told only when exactly one window says
    /// it is active: two claims are not a fact.
    fn find_front(&mut self) {
        let keys: Vec<Key> = self.sight.screen().keys();
        let mut active = Vec::new();
        for key in keys {
            let Some(reference) = reference_of(&key) else { continue };
            if self.ask(CALL_LIMIT, self.atspi.is_active(&reference)) == Some(true) {
                active.push(key);
            }
        }
        if let [only] = active.as_slice() {
            self.apply(Fact::Activated(only.clone()));
        }
    }

    /// Learn a window the model has not seen, from the window itself.
    fn learn(&mut self, key: &Key) {
        if self.sight.screen().knows(key) {
            return;
        }
        let Some(reference) = reference_of(key) else { return };
        if let Some(Some(window)) = self.ask(CALL_LIMIT, self.atspi.window(&reference)) {
            self.apply(Fact::Opened(key.clone(), window));
        }
    }

    /// The window an element is in: itself, a remembered answer, or a climb up its parents.
    fn window_of(&mut self, source: &Key) -> Option<Key> {
        if self.sight.screen().knows(source) {
            return Some(source.clone());
        }
        if let Some(found) = self.remembered.get(source) {
            return found.clone();
        }
        if !self.walks.take(Instant::now()) {
            // Not remembered: a quieter second may still find it.
            return None;
        }
        let mut current = source.clone();
        let mut found = None;
        for _ in 0..MAX_HOPS {
            let Some(reference) = reference_of(&current) else { break };
            let Some(Some(parent)) = self.ask(CALL_LIMIT, self.atspi.parent_of(&reference)) else { break };
            let parent = key_of(&parent);
            if self.sight.screen().knows(&parent) {
                found = Some(parent);
                break;
            }
            if is_root(&parent.1) || parent.1.ends_with("/null") || parent == current {
                break;
            }
            current = parent;
        }
        if self.remembered.len() >= MAX_REMEMBERED {
            self.remembered.clear();
        }
        self.remembered.insert(source.clone(), found.clone());
        found
    }

    /// Something changed inside whatever `source` is in.
    fn touched(&mut self, source: &Key, what: What) {
        // An application with no window the model knows is not on screen: nothing to report.
        let Some(app) = self.sight.screen().app_on(&source.0).map(str::to_string) else { return };
        let window = self.window_of(source);
        self.apply(Fact::Touched { window, app, what });
    }

    fn take(&mut self, raw: Raw) {
        match raw {
            Raw::Created(source) => self.learn(&source),
            Raw::Activated(source) => {
                self.learn(&source);
                self.apply(Fact::Activated(source));
            }
            Raw::Deactivated(source) => self.apply(Fact::Deactivated(source)),
            Raw::Destroyed(source) => self.apply(Fact::Closed(source)),
            Raw::Focused(source) => {
                let Some(reference) = reference_of(&source) else { return };
                let Some(Some((role, name))) = self.ask(CALL_LIMIT, self.atspi.role_and_name(&reference)) else { return };
                let window = self.window_of(&source);
                self.apply(Fact::Focus { window, role, name });
            }
            Raw::Children { source, added, child } if is_root(&source.1) => match child {
                // The registry saying an application came or went. One that came announces its
                // own windows; one that went took them with it.
                Some(child) if is_root(&child.1) => {
                    if !added {
                        self.relist();
                    }
                }
                Some(child) if added => self.learn(&child),
                Some(child) => self.apply(Fact::Closed(child)),
                None => self.relist(),
            },
            Raw::Children { source, .. } => self.touched(&source, What::Children),
            Raw::Text(source) => self.touched(&source, What::Text),
            Raw::Name { source, name } => {
                if !self.sight.screen().knows(&source) {
                    return self.touched(&source, What::Name);
                }
                let title = name.or_else(|| reference_of(&source).and_then(|r| self.ask(CALL_LIMIT, self.atspi.name_of(&r)).flatten()));
                if let Some(title) = title {
                    self.apply(Fact::Renamed(source, title));
                }
            }
            Raw::Vanished(connection) => {
                if self.sight.screen().has_connection(&connection) {
                    self.relist();
                }
            }
        }
    }

    /// Follow the bus until its stream ends.
    fn follow(&mut self, raw: &Receiver<Raw>) {
        self.relist();
        self.find_front();
        loop {
            let wait = self.sight.screen().next_due().map_or(IDLE, |due| due.saturating_duration_since(Instant::now()));
            match raw.recv_timeout(wait) {
                Ok(signal) => self.take(signal),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if self.sight.meters.signals.overflowed.swap(false, Ordering::Relaxed) {
                // Signals were lost, so say so and then make the model right again.
                self.apply(Fact::Gap);
                self.relist();
            }
            self.flush();
        }
    }
}

/// Connect, subscribe, register, and follow until the stream ends.
fn session(runtime: &tokio::runtime::Runtime, sight: &Sight, handles: &handles::Shared, private: fn() -> bool) -> Result<(), String> {
    let atspi = runtime.block_on(Atspi::connect(handles.clone()))?;
    // Subscribed before anything is listed, so nothing that happens during the listing is lost.
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).build();
    let connection = zbus::blocking::Connection::from(atspi.connection().clone());
    let signals = zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(SIGNAL_QUEUE))
        .map_err(|e| format!("could not subscribe to accessibility signals: {e}"))?;
    let mut registered = 0;
    for event in EVENTS {
        match runtime.block_on(atspi.register(event)) {
            Ok(()) => registered += 1,
            Err(e) => tracing::warn!(event, error = %e, "The registry refused an event"),
        }
    }
    if registered == 0 {
        return Err("the accessibility registry registered none of the events".into());
    }

    let (raw_tx, raw_rx) = mpsc::sync_channel(RAW_QUEUE);
    let tally = Arc::clone(&sight.meters.signals);
    std::thread::Builder::new()
        .name("a11y-signals".into())
        .spawn(move || signals::read(signals, raw_tx, tally))
        .map_err(|e| format!("no signal reader thread: {e}"))?;

    sight.meters.listening.store(true, Ordering::Relaxed);
    tracing::info!(events = registered, "Listening to the accessibility bus");
    let walks = Walks { left: WALKS_PER_SECOND, refills_at: Instant::now() + Duration::from_secs(1) };
    Watcher { runtime, atspi, sight, handles, private, remembered: HashMap::new(), walks }.follow(&raw_rx);
    Ok(())
}

fn watch_thread(sight: Sight, handles: handles::Shared, private: fn() -> bool) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!(error = %e, "No runtime; the screen model will stay empty");
            return;
        }
    };
    let mut pause = Duration::from_secs(1);
    let mut last_why = String::new();
    loop {
        match session(&runtime, &sight, &handles, private) {
            Ok(()) => {
                tracing::warn!("The accessibility signal stream ended; connecting again");
                pause = Duration::from_secs(1);
            }
            // Said once per reason, not once per attempt: a machine with no accessibility bus
            // would otherwise write the same line every thirty seconds for ever.
            Err(why) if why != last_why => {
                tracing::warn!(%why, "Not listening to the accessibility bus; will keep trying");
                last_why = why;
            }
            Err(_) => {}
        }
        if sight.meters.listening.swap(false, Ordering::Relaxed) && !private() {
            // The model is on its own until the next listing: a reader should know.
            sight.ring.push_with(|seq| Event { seq, at_ms: now_ms(), kind: Kind::Gap });
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(RETRY_CAP);
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
