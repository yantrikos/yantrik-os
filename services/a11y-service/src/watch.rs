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
//! Any local program can join the accessibility bus. Every question has a time limit, and every
//! answer is charged for the time it took, to its connection and to everyone together: a
//! connection that has used its share waits, one that answers slowly or not at all is left alone
//! for a while (`budget`). Nothing is asked of a connection the model has no window of and the
//! last listing did not name (`plan`). Full listings are at most one a second however many are
//! asked for, and ask applications together, skipping the slow (`listing`). What a signal says
//! about other connections' windows is not believed (`signals::root_change`).
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
//! service refuses `a11y.now` and `a11y.since`. When it ends, a reader is told a gap. The edge is
//! acted on wherever it is first seen, also by the fresh reading taken before something is
//! recorded, and that the person was private is kept on `Sight`, so a private time that began or
//! ended while the bus was away is told by the next session (`privacy`).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use zbus::zvariant::ObjectPath;

use crate::atspi::Atspi;
use crate::budget::{Patience, Relists};
use crate::handles::{self, Reference};
use crate::plan::{climb_blames_application, focus_fact, plan, Known, Step};
use crate::privacy::{edge, Edge, Privacy};
use crate::screen::{Fact, Key, Kind, Screen, What};
use crate::sight::Sight;
use crate::signals::{is_root, Raw};

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
pub const RAW_QUEUE: usize = 4096;

/// Sorted signals taken in before the model is flushed and the clocks are looked at again.
const BATCH: usize = 128;

/// The longest one question to an application, or to the bus, may take.
pub const CALL_LIMIT: Duration = Duration::from_millis(400);

/// The longest finding the window an element is in may take, all its hops together.
const CLIMB_LIMIT: Duration = Duration::from_millis(400);

/// The longest a full listing of every window may take.
const LIST_LIMIT: Duration = Duration::from_secs(5);

/// How long the watcher sleeps when nothing is waiting. Also how soon it notices, on a still
/// screen, that the person went private or stopped being.
const IDLE: Duration = Duration::from_secs(1);

/// Levels climbed at most to find the window an element is in.
const MAX_HOPS: usize = 24;

/// Elements whose window is remembered. Cleared whole when full, and when any window opens or
/// closes.
const MAX_REMEMBERED: usize = 4096;

fn key_of(reference: &Reference) -> Key {
    (reference.0.clone(), reference.1.as_str().to_string())
}

fn reference_of(key: &Key) -> Option<Reference> {
    ObjectPath::try_from(key.1.as_str()).ok().map(|path| (key.0.clone(), path.into()))
}

/// Wait for one answer, for at most `limit`.
pub fn within<T>(runtime: &tokio::runtime::Runtime, limit: Duration, question: impl Future<Output = T>) -> Option<T> {
    runtime.block_on(async { tokio::time::timeout(limit, question).await.ok() })
}

/// Ask an application one thing, if it may be asked now, and charge it for the time it took. One
/// that did not answer is also remembered as slow.
fn ask<T>(runtime: &tokio::runtime::Runtime, patience: &mut Patience, connection: &str, question: impl Future<Output = T>) -> Option<T> {
    if !patience.allows(connection, Instant::now()) {
        return None;
    }
    let started = Instant::now();
    let answer = within(runtime, CALL_LIMIT, question);
    let now = Instant::now();
    patience.spent(connection, now.duration_since(started), now);
    if answer.is_none() {
        patience.timed_out(connection, now);
    }
    answer
}

/// What the model and the last listing know about connections, for `plan`.
struct View<'a> {
    screen: &'a Screen,
    named: &'a HashSet<String>,
}

impl Known for View<'_> {
    fn has_window(&self, connection: &str) -> bool {
        self.screen.has_connection(connection)
    }

    fn named(&self, connection: &str) -> bool {
        self.named.contains(connection)
    }
}

/// The watcher's own state for one connection to the bus.
pub struct Watcher<'a> {
    runtime: &'a tokio::runtime::Runtime,
    atspi: &'a Atspi,
    sight: &'a Sight,
    handles: &'a handles::Shared,
    privacy: Privacy,
    /// Whether the person was private at the last look the watcher acted on. Begins as what the
    /// last session left on `Sight`, so a private time that outlived a connection is not lost.
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
    /// The applications the last listing named, whether or not they have a window.
    named: HashSet<String>,
    /// A window that took the front before the model knew it: taken as the front once a listing
    /// has found it.
    front_after_listing: Option<Key>,
}

impl<'a> Watcher<'a> {
    /// A watcher for one session. `registry` is the connection believed about applications.
    pub fn new(runtime: &'a tokio::runtime::Runtime, atspi: &'a Atspi, sight: &'a Sight, handles: &'a handles::Shared, private: fn() -> bool, registry: String) -> Self {
        Self {
            runtime,
            atspi,
            sight,
            handles,
            privacy: Privacy::new(private),
            // What the last session left: a private time that outlived a connection is not lost.
            private: sight.private_seen(),
            registry,
            remembered: HashMap::new(),
            patience: Patience::new(Instant::now()),
            relists: Relists::new(Instant::now()),
            front_asked: false,
            named: HashSet::new(),
            front_after_listing: None,
        }
    }

    fn apply(&mut self, fact: Fact) {
        let mut screen = self.sight.screen();
        let told = if self.private { screen.apply_quietly(fact, Instant::now()) } else { screen.apply(fact, Instant::now()) };
        if told.is_empty() {
            // Most facts in a burst tell nothing, and must not each cost a read of the file.
            return;
        }
        let private = self.settle_privacy(&mut screen);
        self.tell(told, private);
    }

    fn flush(&mut self) {
        if self.private {
            return;
        }
        let mut screen = self.sight.screen();
        let told = screen.flush(Instant::now());
        if !told.is_empty() {
            let private = self.settle_privacy(&mut screen);
            self.tell(told, private);
        }
    }

    /// Whether the person is private, read afresh, for something about to be recorded: an event
    /// told in the first moments of the private time must not be recorded. If the person went
    /// private since the last look, that is acted on here, before anything is dropped, so that a
    /// private time shorter than the interval between looks leaves no focus and no held change
    /// behind (#1257 review, N5). Called with the model's lock held.
    fn settle_privacy(&mut self, screen: &mut Screen) -> bool {
        if !self.private && edge(false, self.privacy.fresh(Instant::now())) == Some(Edge::Began) {
            self.private = true;
            self.sight.set_private_seen(true);
            screen.private_began();
        }
        self.private
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
        let now = Instant::now();
        let private = self.privacy.check(now);
        let Some(edge) = edge(self.private, private) else { return };
        let mut screen = self.sight.screen();
        match edge {
            Edge::Began => {
                self.private = true;
                self.sight.set_private_seen(true);
                screen.private_began();
            }
            Edge::Ended => {
                self.private = false;
                let told = screen.private_ended();
                // Private again already: the gap is not told, and the next end tells it.
                let again = self.privacy.fresh(now);
                self.sight.record(self.handles, again, told);
                self.sight.set_private_seen(again);
            }
        }
    }

    /// Reconcile the model to a walk of every window: at the start, after a gap, and when an
    /// application left without saying which windows it took. What comes back is the connections
    /// that did not answer, or `None` when there was no listing at all. An application that did
    /// not answer keeps its windows and is asked again later.
    fn relist(&mut self) -> Option<Vec<String>> {
        let now = Instant::now();
        let patience = &self.patience;
        match within(self.runtime, LIST_LIMIT, self.atspi.windows(|connection| patience.is_slow(connection, now))) {
            Some(Ok(listing)) => {
                let now = Instant::now();
                for asked in &listing.asked {
                    self.patience.listed(&asked.connection, asked.took, now);
                    if !asked.answered {
                        self.patience.timed_out(&asked.connection, now);
                    }
                }
                self.named = listing.apps.into_iter().collect();
                let unanswered = listing.unanswered;
                let windows = listing.windows.into_iter().map(|(reference, window)| (key_of(&reference), window)).collect();
                self.apply(Fact::Listing { windows, unanswered: unanswered.clone() });
                if let Some(window) = self.front_after_listing.take() {
                    self.apply(Fact::Activated(window));
                }
                Some(unanswered)
            }
            Some(Err(why)) => {
                tracing::debug!(%why, "Could not list the windows");
                None
            }
            None => {
                tracing::debug!("Listing the windows took too long; the model keeps what it had");
                None
            }
        }
    }

    /// Which window is in front, asked once at the start, of every application that answered. Told
    /// only when exactly one window says it is active: two claims are not a fact.
    fn find_front(&mut self, unanswered: &[String]) {
        let keys: Vec<Key> = self.sight.screen().keys();
        let mut active = Vec::new();
        for key in keys.into_iter().filter(|key| !unanswered.contains(&key.0)) {
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
            let hop_started = Instant::now();
            let left = deadline.saturating_duration_since(hop_started);
            let Some(reference) = reference_of(&current) else { break };
            if left.is_zero() {
                break;
            }
            let parent = within(self.runtime, left, self.atspi.parent_of(&reference));
            let now = Instant::now();
            let took = now.duration_since(hop_started);
            self.patience.spent(&source.0, took, now);
            let Some(parent) = parent else {
                // A climb that ran out of its shared limit on quick answers is not the
                // application's fault.
                if climb_blames_application(took, CLIMB_LIMIT) {
                    self.patience.timed_out(&source.0, now);
                }
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
        // Not learning what the element is does not leave the old focus in place: it is told as
        // an element of unknown role, which is treated as a password (`plan::focus_fact`).
        let learned = reference_of(&source).and_then(|reference| ask(self.runtime, &mut self.patience, &source.0, self.atspi.role_and_name(&reference)).flatten());
        let window = self.window_of(&source);
        self.apply(focus_fact(source, window, learned));
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
        let step = {
            let screen = self.sight.screen();
            plan(raw, &self.registry, &View { screen: &screen, named: &self.named })
        };
        match step {
            Step::Learn(window) => self.learn(&window),
            Step::LearnAndActivate(window) => {
                self.learn(&window);
                self.apply(Fact::Activated(window));
            }
            Step::Apply(fact) => self.apply(fact),
            Step::Focus(source) => self.focused(source),
            Step::Touch(source, what) => self.touched(source, what),
            Step::Rename(source, name) => self.renamed(source, name),
            Step::Relist => self.relists.want(),
            Step::RelistThenActivate(window) => {
                self.front_after_listing = Some(window);
                self.relists.want();
            }
            Step::RegistryRestarted => self.registry_restarted(),
            Step::Nothing => {}
        }
    }

    /// Follow the bus until its stream ends.
    pub fn follow(&mut self, raw: &Receiver<Raw>) {
        self.relists.want();
        loop {
            self.look_at_privacy();
            if self.relists.due(Instant::now()) {
                // What is already queued is older than the listing about to be read, and is
                // applied first: applied after it, an old title would replace the new one
                // (#1257 review, N10).
                for queued in raw.try_iter().take(RAW_QUEUE) {
                    self.take(queued);
                }
                let listed = self.relist();
                self.relists.ran(listed.as_ref().is_some_and(Vec::is_empty), Instant::now());
                // After the first listing that returned at all, asking the applications that
                // answered: one that never does must not leave the front unknown.
                if let (Some(unanswered), false, false) = (listed, self.front_asked, self.private) {
                    self.front_asked = true;
                    self.find_front(&unanswered);
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
            self.sight.set_slow_marks(self.patience.marks());
        }
    }
}

/// Ask the registry to have each of [`EVENTS`] announced. Returns how many it took.
pub fn register(runtime: &tokio::runtime::Runtime, atspi: &Atspi) -> usize {
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
