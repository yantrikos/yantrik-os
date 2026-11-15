//! Listening, so nobody has to look (#1256).
//!
//! Every application with an accessibility bridge announces what changes in it: a window opened,
//! the focus moved, text changed. Screen readers live on those announcements. This thread
//! subscribes to them once and keeps the screen model (`screen`) current, so "what is on screen" is
//! answered from memory and "what changed" is a wait on a ring, not a walk of anyone's tree.
//!
//! # Two threads, and why
//!
//! `a11y-signals` only reads the bus and sorts each signal into a [`Raw`] (`signals`): no calls,
//! no locks, nothing that can stall. `a11y-watch`, here, takes those and does the work that needs
//! a question asked of the application (what is this new window called, which window is this
//! element in). The reader is started before anything is asked of the bus, so a busy bus can
//! never fill the connection's queue while the watcher waits for an answer that is behind it.
//!
//! # Nobody on the bus is trusted with the watcher's time
//!
//! Any local program can join the accessibility bus. Every question has a time limit, each
//! connection has a ration of questions and is left alone for a while after it lets one time
//! out, and full listings are at most one a second however many are asked for (`budget`). What
//! a signal says about other connections' windows is not believed (`signals::root_change`).
//!
//! # What is asked for
//!
//! Applications emit only what some listener registered for, so the list in [`EVENTS`] is also a
//! cost on every application: it is kept to window life, focus, and "something changed here".
//!
//! # Private mode
//!
//! While the person is private the model follows which windows exist and nothing else, nothing
//! is recorded in the ring, no application is asked where the focus is or what changed, and the
//! service refuses `a11y.now` and `a11y.since`. When it ends, a reader is told a gap.

use std::collections::HashMap;
use std::future::Future;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use zbus::zvariant::ObjectPath;

use crate::atspi::Atspi;
use crate::budget::{Patience, Relists};
use crate::handles::{self, Reference};
use crate::screen::{Fact, Key, Kind, What};
use crate::sight::Sight;
use crate::signals::{self, is_root, root_change, Raw, RootChange};

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

/// Sorted signals waiting for the watcher. Past this they are counted and told as a gap.
const RAW_QUEUE: usize = 4096;

/// Sorted signals taken in before the model is flushed and the clocks are looked at again.
const BATCH: usize = 128;

/// The longest one question to an application, or to the bus, may take.
const CALL_LIMIT: Duration = Duration::from_millis(400);

/// The longest finding the window an element is in may take, all its hops together.
const CLIMB_LIMIT: Duration = Duration::from_millis(400);

/// The longest connecting to the accessibility bus may take.
const CONNECT_LIMIT: Duration = Duration::from_secs(5);

/// The longest a full listing of every window may take.
const LIST_LIMIT: Duration = Duration::from_secs(5);

/// How long the watcher sleeps when nothing is waiting. Also how soon it notices, on a still
/// screen, that the person went private or stopped being.
const IDLE: Duration = Duration::from_secs(1);

/// How long one reading of "is the person private" is trusted while signals pour in.
const PRIVATE_FRESH: Duration = Duration::from_millis(100);

/// Levels climbed at most to find the window an element is in.
const MAX_HOPS: usize = 24;

/// Elements whose window is remembered. Cleared whole when full, and when any window opens or
/// closes.
const MAX_REMEMBERED: usize = 4096;

/// The longest pause between attempts to reach the accessibility bus.
const RETRY_CAP: Duration = Duration::from_secs(30);

fn key_of(reference: &Reference) -> Key {
    (reference.0.clone(), reference.1.as_str().to_string())
}

fn reference_of(key: &Key) -> Option<Reference> {
    ObjectPath::try_from(key.1.as_str()).ok().map(|path| (key.0.clone(), path.into()))
}

/// Whether the person is private, read from its file at most once per [`PRIVATE_FRESH`]: a
/// firehose of signals must not become a firehose of file reads.
struct Privacy {
    read: fn() -> bool,
    value: bool,
    read_at: Option<Instant>,
}

impl Privacy {
    fn check(&mut self, now: Instant) -> bool {
        if self.read_at.is_none_or(|at| now.duration_since(at) >= PRIVATE_FRESH) {
            self.value = (self.read)();
            self.read_at = Some(now);
        }
        self.value
    }
}

/// Wait for one answer, for at most `limit`.
fn within<T>(runtime: &tokio::runtime::Runtime, limit: Duration, question: impl Future<Output = T>) -> Option<T> {
    runtime.block_on(async { tokio::time::timeout(limit, question).await.ok() })
}

/// Ask an application one thing, if it may be asked now, and remember it if it did not answer.
fn ask<T>(runtime: &tokio::runtime::Runtime, patience: &mut Patience, connection: &str, question: impl Future<Output = T>) -> Option<T> {
    if !patience.allows(connection, Instant::now()) {
        return None;
    }
    let answer = within(runtime, CALL_LIMIT, question);
    if answer.is_none() {
        patience.timed_out(connection, Instant::now());
    }
    answer
}

/// The watcher's own state for one connection to the bus.
struct Watcher<'a> {
    runtime: &'a tokio::runtime::Runtime,
    atspi: &'a Atspi,
    sight: &'a Sight,
    handles: &'a handles::Shared,
    privacy: Privacy,
    /// Whether the person was private at the last look. The edges are acted on in `follow`.
    private: bool,
    /// The connection that is the registry: the only one believed about applications.
    registry: String,
    /// Which window an element is in, once found. `None` is remembered too: an element that
    /// climbs to no known window is not climbed again on every signal.
    remembered: HashMap<Key, Option<Key>>,
    patience: Patience,
    relists: Relists,
    /// Whether the windows have been asked which of them is in front, once.
    front_asked: bool,
}

impl Watcher<'_> {
    fn apply(&mut self, fact: Fact) {
        let mut screen = self.sight.screen();
        let (told, private) = if self.private {
            (screen.apply_quietly(fact, Instant::now()), true)
        } else {
            // Read afresh, not from the last look: an event told in the first moments of the
            // private time must not be recorded.
            (screen.apply(fact, Instant::now()), (self.privacy.read)())
        };
        self.tell(told, private);
    }

    fn flush(&mut self) {
        if self.private {
            return;
        }
        let mut screen = self.sight.screen();
        let told = screen.flush(Instant::now());
        if !told.is_empty() {
            self.tell(told, (self.privacy.read)());
        }
    }

    fn tell(&mut self, told: Vec<Kind>, private: bool) {
        if told.is_empty() {
            return;
        }
        // A window that came or went changes which window an element climbs to: an element
        // that announced itself before its window did was remembered as being in none.
        if told.iter().any(|kind| matches!(kind, Kind::WindowOpened { .. } | Kind::WindowClosed { .. })) {
            self.remembered.clear();
        }
        self.sight.record(self.handles, private, told);
    }

    /// Notice the person going private or stopping, and do what each edge needs.
    fn look_at_privacy(&mut self) {
        let private = self.privacy.check(Instant::now());
        if private == self.private {
            return;
        }
        self.private = private;
        let mut screen = self.sight.screen();
        if private {
            screen.private_began();
        } else {
            let told = screen.private_ended();
            self.sight.record(self.handles, (self.privacy.read)(), told);
        }
    }

    /// Reconcile the model to a walk of every window: at the start, after a gap, and when an
    /// application left without saying which windows it took. True when every application
    /// answered; an application that did not keeps its windows and is asked again later.
    fn relist(&mut self) -> bool {
        match within(self.runtime, LIST_LIMIT, self.atspi.windows()) {
            Some(Ok(listing)) => {
                let now = Instant::now();
                for connection in &listing.unanswered {
                    self.patience.timed_out(connection, now);
                }
                let everyone_answered = listing.unanswered.is_empty();
                let windows = listing.windows.into_iter().map(|(reference, window)| (key_of(&reference), window)).collect();
                self.apply(Fact::Listing { windows, unanswered: listing.unanswered });
                everyone_answered
            }
            Some(Err(why)) => {
                tracing::debug!(%why, "Could not list the windows");
                false
            }
            None => {
                tracing::debug!("Listing the windows took too long; the model keeps what it had");
                false
            }
        }
    }

    /// Which window is in front, asked once at the start. Told only when exactly one window says
    /// it is active: two claims are not a fact.
    fn find_front(&mut self) {
        let keys: Vec<Key> = self.sight.screen().keys();
        let mut active = Vec::new();
        for key in keys {
            let Some(reference) = reference_of(&key) else { continue };
            if ask(self.runtime, &mut self.patience, &key.0, self.atspi.is_active(&reference)) == Some(true) {
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
        if let Some(Some(window)) = ask(self.runtime, &mut self.patience, &key.0, self.atspi.window(&reference)) {
            self.apply(Fact::Opened(key.clone(), window));
        }
    }

    /// The window an element is in: itself, a remembered answer, or a climb up its parents with
    /// one time limit for the whole climb.
    fn window_of(&mut self, source: &Key) -> Option<Key> {
        if self.sight.screen().knows(source) {
            return Some(source.clone());
        }
        if let Some(found) = self.remembered.get(source) {
            return found.clone();
        }
        if !self.patience.allows(&source.0, Instant::now()) {
            // Not remembered: a quieter second may still find it.
            return None;
        }
        let deadline = Instant::now() + CLIMB_LIMIT;
        let mut current = source.clone();
        let mut found = None;
        for _ in 0..MAX_HOPS {
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(reference) = reference_of(&current) else { break };
            let Some(parent) = within(self.runtime, left, self.atspi.parent_of(&reference)) else {
                self.patience.timed_out(&source.0, Instant::now());
                break;
            };
            let Some(parent) = parent.map(|parent| key_of(&parent)) else { break };
            // A parent on another connection is not this element's window, whatever it says.
            if parent.0 != source.0 || parent == current {
                break;
            }
            if self.sight.screen().knows(&parent) {
                found = Some(parent);
                break;
            }
            if is_root(&parent.1) || parent.1.ends_with("/null") {
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
    fn touched(&mut self, source: Key, what: What) {
        // Nobody is asked anything on the person's account while they are private.
        if self.private {
            return;
        }
        // An application with no window the model knows is not on screen: nothing to report.
        let Some(app) = self.sight.screen().app_on(&source.0).map(str::to_string) else { return };
        let window = self.window_of(&source);
        self.apply(Fact::Touched { source, window, app, what });
    }

    fn focused(&mut self, source: Key) {
        if self.private {
            return;
        }
        let Some(reference) = reference_of(&source) else { return };
        let Some(Some((role, name))) = ask(self.runtime, &mut self.patience, &source.0, self.atspi.role_and_name(&reference)) else { return };
        let window = self.window_of(&source);
        self.apply(Fact::Focus { source, window, role, name });
    }

    fn renamed(&mut self, source: Key, name: Option<String>) {
        if !self.sight.screen().knows(&source) {
            return self.touched(source, What::Name);
        }
        let title = name.or_else(|| {
            let reference = reference_of(&source)?;
            ask(self.runtime, &mut self.patience, &source.0, self.atspi.name_of(&reference)).flatten()
        });
        if let Some(title) = title {
            self.apply(Fact::Renamed(source, title));
        }
    }

    /// The registry has a new owner, which has forgotten what the old one was asked to have
    /// announced: ask again, and treat whatever happened meanwhile as a gap.
    fn registry_restarted(&mut self) {
        if let Some(owner) = within(self.runtime, CALL_LIMIT, self.atspi.registry_owner()).flatten() {
            self.registry = owner;
        }
        let registered = register(self.runtime, self.atspi);
        tracing::info!(events = registered, "The accessibility registry restarted; registered again");
        self.apply(Fact::Gap);
        self.relists.want();
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
            Raw::Focused(source) => self.focused(source),
            Raw::Children { source, added, child } if is_root(&source.1) => match root_change(&source, added, child, &self.registry) {
                RootChange::Relist => self.relists.want(),
                RootChange::Learn(window) => self.learn(&window),
                RootChange::Closed(window) => self.apply(Fact::Closed(window)),
                RootChange::Ignore => {}
            },
            Raw::Children { source, .. } => self.touched(source, What::Children),
            Raw::Text(source) => self.touched(source, What::Text),
            Raw::Name { source, name } => self.renamed(source, name),
            Raw::Vanished(connection) => {
                if self.sight.screen().has_connection(&connection) {
                    self.relists.want();
                }
            }
            Raw::RegistryRestarted => self.registry_restarted(),
        }
    }

    /// Follow the bus until its stream ends.
    fn follow(&mut self, raw: &Receiver<Raw>) {
        self.relists.want();
        loop {
            self.look_at_privacy();
            if self.relists.due(Instant::now()) {
                let everyone_answered = self.relist();
                self.relists.ran(everyone_answered, Instant::now());
                if everyone_answered && !self.front_asked && !self.private {
                    self.front_asked = true;
                    self.find_front();
                }
            }
            let due = [self.sight.screen().next_due(), self.relists.next()].into_iter().flatten().min();
            let wait = due.map_or(IDLE, |at| at.saturating_duration_since(Instant::now()).min(IDLE));
            match raw.recv_timeout(wait) {
                Ok(first) => {
                    self.take(first);
                    // Whatever else is already waiting, up to a batch, before the model is
                    // flushed and the clocks are looked at again.
                    for more in raw.try_iter().take(BATCH - 1) {
                        self.take(more);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if self.sight.take_overflow() {
                // Signals were lost: say so, and have the model made right again.
                self.apply(Fact::Gap);
                self.relists.want();
            }
            self.flush();
        }
    }
}

/// Ask the registry to have each of [`EVENTS`] announced. Returns how many it took.
fn register(runtime: &tokio::runtime::Runtime, atspi: &Atspi) -> usize {
    let mut registered = 0;
    for event in EVENTS {
        match within(runtime, CALL_LIMIT, atspi.register(event)) {
            Some(Ok(())) => registered += 1,
            Some(Err(e)) => tracing::warn!(event, error = %e, "The registry refused an event"),
            None => tracing::warn!(event, "The registry did not answer in time"),
        }
    }
    registered
}

/// Read, subscribe, register, and follow until the stream ends.
fn listen(runtime: &tokio::runtime::Runtime, atspi: &Atspi, sight: &Sight, handles: &handles::Shared, private: fn() -> bool) -> Result<(), String> {
    // The reader first, before anything is asked of the bus. Every message that arrives on the
    // connection goes through its queue, and a full queue stops the answers arriving too: a
    // busy bus must never be able to fill it while the watcher waits for an answer behind it.
    let connection = zbus::blocking::Connection::from(atspi.connection().clone());
    let messages = zbus::blocking::MessageIterator::from(&connection);
    let (raw_tx, raw_rx) = mpsc::sync_channel(RAW_QUEUE);
    let tally = sight.tally();
    std::thread::Builder::new()
        .name("a11y-signals".into())
        .spawn(move || signals::read(messages, raw_tx, tally))
        .map_err(|e| format!("no signal reader thread: {e}"))?;

    // Subscribed before anything is listed, so nothing that happens during the listing is lost.
    within(runtime, CALL_LIMIT, atspi.subscribe()).ok_or("the accessibility bus did not take the subscription in time")??;
    let registry = within(runtime, CALL_LIMIT, atspi.registry_owner()).flatten().ok_or("the accessibility registry is not on the bus")?;
    let registered = register(runtime, atspi);
    if registered == 0 {
        return Err("the accessibility registry registered none of the events".into());
    }

    sight.set_listening(true);
    tracing::info!(events = registered, "Listening to the accessibility bus");
    let now = Instant::now();
    let mut watcher = Watcher {
        runtime,
        atspi,
        sight,
        handles,
        privacy: Privacy { read: private, value: false, read_at: None },
        private: false,
        registry,
        remembered: HashMap::new(),
        patience: Patience::new(now),
        relists: Relists::new(now),
        front_asked: false,
    };
    watcher.follow(&raw_rx);
    Ok(())
}

/// One connection to the bus, from first to last. The connection is closed on the way out
/// whatever happened, which is what ends its reader thread: a session that failed half-way must
/// not leave a reader behind for the next attempt to add to.
fn session(runtime: &tokio::runtime::Runtime, sight: &Sight, handles: &handles::Shared, private: fn() -> bool) -> Result<(), String> {
    let atspi = within(runtime, CONNECT_LIMIT, Atspi::connect(handles.clone())).ok_or("the accessibility bus did not answer in time")??;
    let outcome = listen(runtime, &atspi, sight, handles, private);
    let _ = within(runtime, CALL_LIMIT, atspi.connection().clone().close());
    outcome
}

/// Start listening on a thread of its own, and hand back the reader's side. Returns at once;
/// the bus is connected to lazily and retried, because accessibility often starts after the
/// session does.
pub fn start(handles: handles::Shared, private: fn() -> bool) -> Sight {
    let sight = Sight::idle();
    let feed = sight.clone();
    let spawned = std::thread::Builder::new().name("a11y-watch".into()).spawn(move || watch_thread(feed, handles, private));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "No watcher thread; the screen model will stay empty");
    }
    sight
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
        if sight.set_listening(false) {
            // The model is on its own until the next listing: a reader should know.
            let _model = sight.screen();
            sight.record(&handles, private(), vec![Kind::Gap]);
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(RETRY_CAP);
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
