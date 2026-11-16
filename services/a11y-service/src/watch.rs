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
//! for a while, and when everyone together has used the total a connection that has used next to
//! nothing is still let in (`budget`). A window whose question was refused is not lost: it is
//! kept, asked again, and a listing is asked for (`pending`). Nothing is asked of a connection
//! the model has no window of and the last listing did not name (`plan`). Full listings are
//! spaced so that they take a quarter of the watcher's time at most, and ask applications
//! together, skipping the slow and those that owe their share (`listing`). What a signal says
//! about other connections' windows is not believed (`signals::root_change`).
//!
//! # Testable without a bus
//!
//! What the watcher asks of the bus is the trait [`Questions`], so its wiring (what it asks,
//! of whom, what it does with a refusal or a silence) is tested with a fake.
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
//! acted on wherever it is first seen: also by the fresh reading taken before something is
//! recorded, and by the one taken when a change begins to be held, so that a private time
//! shorter than the interval between looks leaves no keystroke held to be told afterwards. That
//! the person was private is kept on `Sight`, so a private time that began or ended while the
//! bus was away is told by the next session (`privacy`).

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::asking::{ask, key_of, reference_of, within, Asked, CALL_LIMIT};
use crate::budget::{Patience, Relists};
use crate::handles;
use crate::pending::Pending;
use crate::plan::{climb_blames_application, focus_fact, plan, Known, Step};
use crate::privacy::{edge, Edge, Privacy};
use crate::questions::Questions;
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

/// The longest finding the window an element is in may take, all its hops together.
const CLIMB_LIMIT: Duration = Duration::from_millis(400);

/// The longest a full listing of every window may take, the registry's own answer included.
const LIST_LIMIT: Duration = Duration::from_secs(2);

/// How long the watcher sleeps when nothing is waiting. Also how soon it notices, on a still
/// screen, that the person went private or stopped being.
const IDLE: Duration = Duration::from_secs(1);

/// How soon a window whose question was refused is asked about again.
const RETRY: Duration = Duration::from_millis(100);

/// Levels climbed at most to find the window an element is in.
const MAX_HOPS: usize = 24;

/// Elements whose window is remembered. Cleared whole when full, and when any window opens or
/// closes.
const MAX_REMEMBERED: usize = 4096;

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
pub struct Watcher<'a, Q: Questions> {
    runtime: &'a tokio::runtime::Runtime,
    questions: &'a Q,
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
    /// Whether the windows have been asked which of them is in front, once, all of them.
    front_asked: bool,
    /// The applications the last listing named, whether or not they have a window.
    named: HashSet<String>,
    /// Windows refused for lack of budget, and windows that took the front before the model
    /// knew them.
    pending: Pending,
    /// Whether the last listing failed, so that a run of failures is said once.
    listing_failed: bool,
}

impl<'a, Q: Questions> Watcher<'a, Q> {
    /// A watcher for one session. `registry` is the connection believed about applications.
    pub fn new(runtime: &'a tokio::runtime::Runtime, questions: &'a Q, sight: &'a Sight, handles: &'a handles::Shared, private: fn() -> bool, registry: String) -> Self {
        Self {
            runtime,
            questions,
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
            pending: Pending::default(),
            listing_failed: false,
        }
    }

    fn apply(&mut self, fact: Fact) {
        let mut screen = self.sight.screen();
        // Something else took the front, or this window lost it: what waited to take it is stale.
        match &fact {
            Fact::Activated(key) if screen.knows(key) => self.pending.front_taken(),
            Fact::Deactivated(key) => self.pending.deactivated(key),
            _ => {}
        }
        let told = if self.private { screen.apply_quietly(fact, Instant::now()) } else { screen.apply(fact, Instant::now()) };
        let started = screen.take_hold_started() && !self.private;
        if told.is_empty() && !started {
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

    /// Whether the person is private, read afresh, for something about to be recorded or begun
    /// to be held: an event told in the first moments of the private time must not be recorded,
    /// and nor must a keystroke held through a private time shorter than the interval between
    /// looks. If the person went private since the last look, that is acted on here, before
    /// anything is dropped (#1257 review, N5 and L1). Called with the model's lock held.
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
                let told = screen.private_ended();
                // Private again already: the gap is not told, and the next end tells it.
                let again = self.privacy.fresh(now);
                self.private = again;
                self.sight.record(self.handles, again, told);
                self.sight.set_private_seen(again);
            }
        }
    }

    /// A full listing, charged to everyone for the time it took, and the next one spaced by it.
    /// What comes back is the connections that did not answer, or `None` when there was no
    /// listing at all.
    fn list_now(&mut self) -> Option<HashSet<String>> {
        let started = Instant::now();
        let listed = self.relist();
        let now = Instant::now();
        let took = now.duration_since(started);
        self.patience.listing_took(took, now);
        self.relists.ran(listed.is_some(), took, now);
        listed
    }

    /// Reconcile the model to a walk of every window: at the start, after a gap, and when an
    /// application left without saying which windows it took. An application that did not answer
    /// keeps its windows and is asked again later.
    fn relist(&mut self) -> Option<HashSet<String>> {
        let now = Instant::now();
        let patience = &self.patience;
        let skip = |connection: &str| patience.is_busy(connection, now);
        match within(self.runtime, LIST_LIMIT, self.questions.windows(&skip)) {
            Some(Ok(listing)) => {
                self.listing_failed = false;
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
                // The windows that took the front before the model knew them, if a listing found
                // them and the front has not moved since.
                let found = {
                    let screen = self.sight.screen();
                    self.pending.after_listing(Instant::now(), |key| screen.knows(key))
                };
                for window in found {
                    self.apply(Fact::Activated(window));
                }
                Some(unanswered)
            }
            Some(Err(why)) => {
                self.listing_did_not_work(&why);
                None
            }
            None => {
                self.listing_did_not_work("it took too long");
                None
            }
        }
    }

    /// A listing that did not work leaves the model as stale as it was, and a reader should be
    /// able to see that (#1257 review, L7): it is counted, and said once per run of failures.
    fn listing_did_not_work(&mut self, why: &str) {
        self.sight.count_failed_listing();
        if !self.listing_failed {
            tracing::warn!(why, "Could not list the windows; the model keeps what it had, and will try again");
        }
        self.listing_failed = true;
    }

    /// Which window is in front, asked once at the start, of every application that answered.
    /// Told only when exactly one window says it is active: two claims are not a fact. True when
    /// every window was asked; one that was refused is a reason to ask again after the next
    /// listing, not to stay without a front (#1257 review, L3).
    fn find_front(&mut self, unanswered: &HashSet<String>) -> bool {
        let keys: Vec<Key> = self.sight.screen().keys();
        let (mut active, mut all_asked) = (Vec::new(), true);
        for key in keys.into_iter().filter(|key| !unanswered.contains(&key.0)) {
            let Some(reference) = reference_of(&key) else { continue };
            match ask(self.runtime, &mut self.patience, &key.0, self.questions.is_active(&reference)) {
                Asked::Answer(true) => active.push(key),
                Asked::Refused => all_asked = false,
                Asked::Answer(false) | Asked::Silent => {}
            }
        }
        if let [only] = active.as_slice() {
            self.apply(Fact::Activated(only.clone()));
        }
        all_asked
    }

    /// Learn a window the model has not seen, from the window itself. Refused when its
    /// connection may not be asked now.
    fn learn(&mut self, key: &Key) -> Asked<()> {
        if self.sight.screen().knows(key) {
            return Asked::Answer(());
        }
        let Some(reference) = reference_of(key) else { return Asked::Silent };
        match ask(self.runtime, &mut self.patience, &key.0, self.questions.window(&reference)) {
            Asked::Answer(Some(window)) => {
                self.apply(Fact::Opened(key.clone(), window));
                Asked::Answer(())
            }
            Asked::Answer(None) | Asked::Silent => Asked::Silent,
            Asked::Refused => Asked::Refused,
        }
    }

    /// Learn a window, and take it as the front if asked. A window whose question was refused is
    /// kept, and a listing is asked for: it does not depend on the budget, and finds the window
    /// in about a second at worst (#1257 review, H1).
    fn learn_or_keep(&mut self, key: Key, activate: bool) {
        if self.learn(&key) == Asked::Refused {
            self.pending.refused(key, activate, Instant::now());
            self.relists.want();
            return;
        }
        if activate {
            self.apply(Fact::Activated(key));
        }
    }

    /// Ask again about the windows whose question was refused, before anything else, now that
    /// their connections may be asked.
    fn retry_pending(&mut self) {
        for (key, activate) in self.pending.asks_due(Instant::now()) {
            if self.learn(&key) != Asked::Refused {
                self.pending.answered(&key);
                if activate {
                    self.apply(Fact::Activated(key));
                }
            }
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
            let parent = within(self.runtime, left, self.questions.parent_of(&reference));
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
        let learned = reference_of(&source).and_then(|reference| ask(self.runtime, &mut self.patience, &source.0, self.questions.role_and_name(&reference)).answer().flatten());
        let window = self.window_of(&source);
        self.apply(focus_fact(source, window, learned));
    }

    fn renamed(&mut self, source: Key, name: Option<String>) {
        if !self.sight.screen().knows(&source) {
            return self.touched(source, What::Name);
        }
        let title = name.or_else(|| {
            let reference = reference_of(&source)?;
            ask(self.runtime, &mut self.patience, &source.0, self.questions.name_of(&reference)).answer().flatten()
        });
        if let Some(title) = title {
            self.apply(Fact::Renamed(source, title));
        }
    }

    /// The registry has a new owner, which has forgotten what the old one was asked to have
    /// announced: ask again, and treat whatever happened meanwhile as a gap.
    fn registry_restarted(&mut self) {
        if let Some(owner) = within(self.runtime, CALL_LIMIT, self.questions.registry_owner()).flatten() {
            self.registry = owner;
        }
        let registered = register(self.runtime, self.questions);
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
            Step::Learn(window) => self.learn_or_keep(window, false),
            Step::LearnAndActivate(window) => self.learn_or_keep(window, true),
            Step::Apply(fact) => self.apply(fact),
            Step::Focus(source) => self.focused(source),
            Step::Touch(source, what) => self.touched(source, what),
            Step::Rename(source, name) => self.renamed(source, name),
            Step::Relist => self.relists.want(),
            Step::RelistThenActivate(window) => {
                self.pending.front_when_found(window, Instant::now());
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
            self.retry_pending();
            if self.relists.due(Instant::now()) {
                // What is already queued is older than the listing about to be read, and is
                // applied first: applied after it, an old title would replace the new one
                // (#1257 review, N10).
                for queued in raw.try_iter().take(RAW_QUEUE) {
                    self.take(queued);
                }
                let listed = self.list_now();
                // After the first listing that returned at all, asking the applications that
                // answered: one that never does must not leave the front unknown.
                if let (Some(unanswered), false, false) = (listed, self.front_asked, self.private) {
                    self.front_asked = self.find_front(&unanswered);
                    if !self.front_asked {
                        self.relists.want();
                    }
                }
            }
            let due = [self.sight.screen().next_due(), self.relists.next()].into_iter().flatten().min();
            let mut wait = due.map_or(IDLE, |at| at.saturating_duration_since(Instant::now()).min(IDLE));
            if self.pending.is_asking() {
                wait = wait.min(RETRY);
            }
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
pub fn register(runtime: &tokio::runtime::Runtime, questions: &impl Questions) -> usize {
    let mut registered = 0;
    for event in EVENTS {
        match within(runtime, CALL_LIMIT, questions.register(event)) {
            Some(Ok(())) => registered += 1,
            Some(Err(e)) => tracing::warn!(event, error = %e, "The registry refused an event"),
            None => tracing::warn!(event, "The registry did not answer in time"),
        }
    }
    registered
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
