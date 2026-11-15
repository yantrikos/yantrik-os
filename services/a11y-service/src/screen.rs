//! The screen as a model, kept current by being told (#1256).
//!
//! `a11y.windows` and `a11y.describe` answer by walking the accessibility tree when asked: a
//! camera, however cheap. A reader that wants to know when something changes has to keep asking,
//! and pays for every look that finds nothing new.
//!
//! This is the other half. The watcher (`watch`) listens to what the applications announce and
//! turns each announcement into a [`Fact`]; this model takes the facts, stays current, and says
//! what stopped being true as a short list of [`Kind`]s. A reader then asks two cheap questions:
//! "what is on screen now" (answered from here, with no call to any application) and "what
//! changed since my cursor" (answered from the ring, and it can wait there).
//!
//! # What an event says, and what it does not
//!
//! An event says *that* something changed and *where*: which window, what kind of change. It
//! never carries the changed text, and it never says how many changes it stands for: one
//! text-changed signal is one keystroke, so a count would be the length of whatever was typed.
//! Changes in a focused password field are not told at all, and neither are changes in a focused
//! field whose role could not be learned: when in doubt it is a password.
//!
//! Typing is still told as activity, but not as keystrokes (`throttle`): "text is changing in
//! this window" at most once per [`TEXT_COALESCE`](crate::throttle::TEXT_COALESCE), stamped to the whole second by the ring
//! (`sight`). Anyone who can read the stream learns that someone typed, and roughly when; not
//! how fast, how much, or how often they corrected themselves.
//!
//! # Not a transcript
//!
//! Accessibility is a firehose: a page load is thousands of signals. Changes inside a window are
//! therefore told at most once per 150 ms (children, names, titles) or once per 2 s (text); see `throttle`
//! for the two lanes.
//!
//! # Bounded
//!
//! Any program can announce any number of windows. The model keeps at most [`MAX_PER_CONNECTION`]
//! for one connection and [`MAX_WINDOWS`] in all, and counts what it refused. When the model is
//! full, a connection with few windows still gets one: the newest window of the connection with
//! the most is closed to make room, told as an ordinary `window_closed`, so eight busy connections
//! cannot lock every other application out.
//!
//! # Private mode
//!
//! While the person is private the model still follows which windows exist, because it has to
//! be right the moment they are not. It follows nothing else: no focus, no changes inside
//! windows, nothing held to be told later ([`Screen::apply_quietly`]).
//!
//! Pure on purpose: no bus, no clock of its own, so every rule here has a test.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::throttle::{Held, Target, Throttles};

/// How long "the front window lost the front" waits for the next window to take it, so a switch
/// between two windows is told as one change and not as two.
pub const FRONT_GRACE: Duration = Duration::from_millis(60);

/// Windows kept for one connection at most.
pub const MAX_PER_CONNECTION: usize = 64;

/// Windows kept in all at most.
pub const MAX_WINDOWS: usize = 512;

/// The role a toolkit gives a field whose text is hidden as it is typed.
const PASSWORD: &str = "password text";

/// The role of a focused element whose role could not be learned. The password rule treats it as
/// a password: not knowing is not a reason to tell.
pub const UNKNOWN_ROLE: &str = "unknown";

/// An accessibility object: which connection, and where on it. Plain strings, so the model needs
/// no bus types.
pub type Key = (String, String);

/// One window a person is looking at, as `a11y.windows`, `a11y.now` and every event describe it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowInfo {
    /// The application's own name for itself: `zenity`, `firefox`, `soffice`.
    pub app: String,
    pub title: String,
    pub role: String,
    pub pid: u32,
    /// Opaque handle for `a11y.describe` and `a11y.act`, the same for as long as the window is
    /// open. Deliberately not a path a caller could construct: the only ones that work are ones
    /// we handed out.
    pub id: String,
}

/// What kind of change happened inside a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Text,
    Children,
    Name,
}

/// One thing the watcher learned, already resolved to windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// Every window there is, from a walk of the registry. The model is reconciled to it, except
    /// for the windows of `unanswered` connections: an application that did not answer in time
    /// has not closed its windows.
    Listing { windows: Vec<(Key, WindowInfo)>, unanswered: Vec<String> },
    Opened(Key, WindowInfo),
    Closed(Key),
    Activated(Key),
    Deactivated(Key),
    Renamed(Key, String),
    /// The keyboard focus moved to the element `source` (its role and name), in a window if one
    /// was found.
    Focus { source: Key, window: Option<Key>, role: String, name: String },
    /// Something changed in the element `source`. `window` is `None` when only its application
    /// is known.
    Touched { source: Key, window: Option<Key>, app: String, what: What },
    /// Signals were lost, so the model may be wrong until the next `Listing`.
    Gap,
}

/// One change, as a reader is told it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    WindowOpened { window: WindowInfo },
    WindowClosed { window: WindowInfo },
    /// The window has a new title; `window.title` is the new one.
    WindowRenamed { window: WindowInfo },
    /// Another window is in front. `None`: the front window is one that publishes no tree.
    FrontChanged { window: Option<WindowInfo> },
    /// The keyboard focus is on another element. `window` is the id of the window it is in.
    FocusMoved { window: Option<String>, role: String, name: String },
    /// Something inside the window changed. One event stands for any number of changes.
    ContentChanged { window: Option<String>, app: String, what: Vec<&'static str> },
    /// The stream has a hole here: signals were lost, the bus went away, or the person was
    /// private. What follows is true again, but a reader should read `a11y.now`.
    Gap,
}

/// Where the keyboard focus is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Focus {
    pub window: Option<String>,
    pub role: String,
    pub name: String,
}

/// Counted, never described. Nothing here moves once per keystroke.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Counts {
    /// Events told.
    pub told: u64,
    /// Times the model was reconciled to a full listing.
    pub listings: u64,
    /// Times the stream had a hole.
    pub gaps: u64,
    /// Windows announced and not kept, because their connection or the model was full, and
    /// windows closed again to make room for a connection with fewer.
    pub refused_windows: u64,
}

/// What the model knows about one connection: how many windows it has, and its application.
struct Connection {
    windows: usize,
    app: String,
}

#[derive(Default)]
pub struct Screen {
    windows: HashMap<Key, WindowInfo>,
    /// The windows in the order they were learned, which is the order they are listed in.
    order: Vec<Key>,
    connections: HashMap<String, Connection>,
    front: Option<Key>,
    /// Set when the front window lost the front and nothing has taken it yet.
    front_lost_at: Option<Instant>,
    focus: Option<Focus>,
    /// The focused element itself, for the rules that depend on what it is.
    focus_source: Option<Key>,
    throttles: Throttles,
    counts: Counts,
}

impl Screen {
    pub fn knows(&self, key: &Key) -> bool {
        self.windows.contains_key(key)
    }

    /// The name of the application on a connection, when the model has a window of it.
    pub fn app_on(&self, connection: &str) -> Option<&str> {
        self.connections.get(connection).map(|c| c.app.as_str())
    }

    /// Whether any known window belongs to this connection.
    pub fn has_connection(&self, connection: &str) -> bool {
        self.connections.contains_key(connection)
    }

    /// Every window's key, for asking each one something.
    pub fn keys(&self) -> Vec<Key> {
        self.order.clone()
    }

    pub fn windows(&self) -> impl Iterator<Item = &WindowInfo> {
        self.order.iter().filter_map(|key| self.windows.get(key))
    }

    pub fn front(&self) -> Option<&WindowInfo> {
        self.front.as_ref().and_then(|key| self.windows.get(key))
    }

    pub fn focus(&self) -> Option<&Focus> {
        self.focus.as_ref()
    }

    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// Take one fact in and say what a reader should be told at once.
    pub fn apply(&mut self, fact: Fact, now: Instant) -> Vec<Kind> {
        let mut out = Vec::new();
        match fact {
            Fact::Listing { windows, unanswered } => self.reconcile(windows, &unanswered, now, &mut out),
            Fact::Opened(key, info) => self.open(key, info, now, &mut out),
            Fact::Closed(key) => self.close(&key, now, &mut out),
            Fact::Activated(key) => self.activate(key, &mut out),
            Fact::Deactivated(key) => {
                if self.front.as_ref() == Some(&key) && self.front_lost_at.is_none() {
                    self.front_lost_at = Some(now);
                }
            }
            Fact::Renamed(key, title) => self.rename(key, title, now, &mut out),
            Fact::Focus { source, window, role, name } => {
                let focus = Focus { window: window.as_ref().and_then(|k| self.windows.get(k)).map(|i| i.id.clone()), role, name };
                self.focus_source = Some(source);
                if self.focus.as_ref() != Some(&focus) {
                    out.push(Kind::FocusMoved { window: focus.window.clone(), role: focus.role.clone(), name: focus.name.clone() });
                    self.focus = Some(focus);
                }
            }
            Fact::Touched { source, window, app, what } => {
                // Typing in a password field is one text change per character. Neither that it
                // happened nor when is anyone's to know, and a field whose role is not known is
                // treated as one (#1257 review, H5 and N1).
                let secret = what == What::Text && self.focus_source.as_ref() == Some(&source) && self.focus.as_ref().is_some_and(|f| f.role == PASSWORD || f.role == UNKNOWN_ROLE);
                if !secret {
                    // A window that has gone is not one to report on; its application still is.
                    let target = match window.filter(|key| self.knows(key)) {
                        Some(key) => Target::Window(key),
                        None => Target::App(app),
                    };
                    self.touch(target, what, now, &mut out);
                }
            }
            Fact::Gap => {
                self.counts.gaps += 1;
                out.push(Kind::Gap);
            }
        }
        self.counts.told += out.len() as u64;
        out
    }

    /// Take one fact in while the person is private: follow which windows exist and which is
    /// in front, and nothing else. Nothing is told, nothing is held, and the focus is forgotten.
    ///
    /// What comes back is only the windows that came and went, for the caller's own
    /// bookkeeping (a window's handle lives as long as the window). It is not for telling.
    pub fn apply_quietly(&mut self, fact: Fact, now: Instant) -> Vec<Kind> {
        let mut unheard = Vec::new();
        match fact {
            Fact::Listing { windows, unanswered } => self.reconcile(windows, &unanswered, now, &mut unheard),
            Fact::Opened(key, info) => self.open(key, info, now, &mut unheard),
            Fact::Closed(key) => self.close(&key, now, &mut unheard),
            Fact::Activated(key) => {
                if self.knows(&key) {
                    self.front = Some(key);
                }
            }
            Fact::Deactivated(key) => {
                if self.front.as_ref() == Some(&key) {
                    self.front = None;
                }
            }
            Fact::Renamed(key, title) => {
                if let Some(info) = self.windows.get_mut(&key) {
                    info.title = title;
                }
            }
            Fact::Focus { .. } | Fact::Touched { .. } | Fact::Gap => {}
        }
        self.forget_held();
        self.forget_focus();
        unheard.retain(|kind| matches!(kind, Kind::WindowOpened { .. } | Kind::WindowClosed { .. }));
        unheard
    }

    /// The person went private: whatever was waiting to be told is never told, and the focus is
    /// not kept.
    pub fn private_began(&mut self) {
        self.forget_held();
        self.forget_focus();
    }

    /// The person is no longer private. A reader that follows the stream has a hole where the
    /// private time was, and is told so, so it reads the screen again instead of trusting a
    /// picture from before.
    pub fn private_ended(&mut self) -> Vec<Kind> {
        self.forget_held();
        self.forget_focus();
        self.counts.gaps += 1;
        self.counts.told += 1;
        vec![Kind::Gap]
    }

    fn forget_held(&mut self) {
        self.throttles.forget_held();
        // A front that was lost and not yet taken is settled now, without a word.
        if self.front_lost_at.take().is_some() {
            self.front = None;
        }
    }

    fn forget_focus(&mut self) {
        self.focus = None;
        self.focus_source = None;
    }

    /// What became due by `now`: held changes whose interval has passed, and a front that was
    /// lost and not taken.
    pub fn flush(&mut self, now: Instant) -> Vec<Kind> {
        let mut out = Vec::new();
        if self.front_lost_at.is_some_and(|at| now.duration_since(at) >= FRONT_GRACE) {
            self.front_lost_at = None;
            self.front = None;
            out.push(Kind::FrontChanged { window: None });
        }
        for (target, held) in self.throttles.due(now) {
            self.tell_held(&target, held, &mut out);
        }
        self.counts.told += out.len() as u64;
        out
    }

    /// When `flush` next has something to say, if anything is waiting.
    pub fn next_due(&self) -> Option<Instant> {
        self.throttles.next_due().into_iter().chain(self.front_lost_at.map(|at| at + FRONT_GRACE)).min()
    }

    fn open(&mut self, key: Key, info: WindowInfo, now: Instant, out: &mut Vec<Kind>) {
        if self.knows(&key) {
            return;
        }
        let on_connection = self.connections.get(&key.0).map_or(0, |c| c.windows);
        if on_connection >= MAX_PER_CONNECTION {
            self.counts.refused_windows += 1;
            return;
        }
        if self.windows.len() >= MAX_WINDOWS && !self.make_room_for(&key.0, on_connection, now, out) {
            self.counts.refused_windows += 1;
            return;
        }
        let connection = self.connections.entry(key.0.clone()).or_insert_with(|| Connection { windows: 0, app: info.app.clone() });
        connection.windows += 1;
        out.push(Kind::WindowOpened { window: info.clone() });
        self.order.push(key.clone());
        self.windows.insert(key, info);
    }

    /// The model is full. A newcomer from a connection with fewer windows than an equal share is
    /// let in, by closing the newest window of the connection that has the most (#1257 review,
    /// N3). Without this, eight connections with 64 windows each lock every other application
    /// out for as long as they stay. True when there is room now.
    fn make_room_for(&mut self, connection: &str, holds: usize, now: Instant, out: &mut Vec<Kind>) -> bool {
        let sharing = self.connections.len() + usize::from(!self.connections.contains_key(connection));
        if holds >= MAX_WINDOWS / sharing.max(1) {
            return false;
        }
        let Some(biggest) = self.connections.iter().filter(|(name, _)| name.as_str() != connection).max_by(|a, b| (a.1.windows, a.0).cmp(&(b.1.windows, b.0))).map(|(name, _)| name.clone()) else {
            return false;
        };
        let Some(newest) = self.order.iter().rev().find(|key| key.0 == biggest).cloned() else { return false };
        self.counts.refused_windows += 1;
        self.close(&newest, now, out);
        true
    }

    fn close(&mut self, key: &Key, now: Instant, out: &mut Vec<Kind>) {
        let Some(info) = self.windows.remove(key) else { return };
        self.order.retain(|k| k != key);
        self.throttles.remove(&Target::Window(key.clone()));
        let last_of_its_connection = match self.connections.get_mut(&key.0) {
            Some(connection) if connection.windows > 1 => {
                connection.windows -= 1;
                false
            }
            _ => {
                self.connections.remove(&key.0);
                true
            }
        };
        // Focus does not outlive its window, nor an application with no window left.
        let in_this_window = self.focus.as_ref().is_some_and(|f| f.window.as_deref() == Some(info.id.as_str()));
        let in_this_application = last_of_its_connection && self.focus_source.as_ref().is_some_and(|source| source.0 == key.0);
        if in_this_window || in_this_application {
            self.forget_focus();
        }
        if self.front.as_ref() == Some(key) && self.front_lost_at.is_none() {
            self.front_lost_at = Some(now);
        }
        out.push(Kind::WindowClosed { window: info });
    }

    fn activate(&mut self, key: Key, out: &mut Vec<Kind>) {
        let Some(info) = self.windows.get(&key).cloned() else { return };
        // Whatever takes the front ends the wait for a taker. The same window taking it back
        // inside the grace was never told as lost, so for a reader nothing changed.
        self.front_lost_at = None;
        if self.front.as_ref() != Some(&key) {
            out.push(Kind::FrontChanged { window: Some(info) });
            self.front = Some(key);
        }
    }

    fn rename(&mut self, key: Key, title: String, now: Instant, out: &mut Vec<Kind>) {
        let Some(info) = self.windows.get_mut(&key) else { return };
        if info.title == title {
            return;
        }
        // The model has the new title at once; a reader is told through the throttle, so a
        // window that retitles itself every frame is one event per interval.
        info.title = title;
        if let Some(held) = self.throttles.quick(Target::Window(key.clone()), now, |held| held.renamed = true) {
            self.tell_held(&Target::Window(key), held, out);
        }
    }

    /// One change for `target`. Text waits its own, longer interval; the rest is told at once
    /// when the target has been quiet, and held otherwise.
    fn touch(&mut self, target: Target, what: What, now: Instant, out: &mut Vec<Kind>) {
        if what == What::Text {
            self.throttles.text(target, now);
            return;
        }
        let held = self.throttles.quick(target.clone(), now, |held| match what {
            What::Children => held.children = true,
            What::Name => held.name = true,
            What::Text => held.text = true,
        });
        if let Some(held) = held {
            self.tell_held(&target, held, out);
        }
    }

    fn tell_held(&self, target: &Target, held: Held, out: &mut Vec<Kind>) {
        let (info, app) = match target {
            // A window that closed while its change was held has nothing left to report on.
            Target::Window(key) => match self.windows.get(key) {
                Some(info) => (Some(info), info.app.clone()),
                None => return,
            },
            Target::App(app) => (None, app.clone()),
        };
        if held.renamed {
            if let Some(info) = info {
                out.push(Kind::WindowRenamed { window: info.clone() });
            }
        }
        let what: Vec<&'static str> =
            [(held.text, "text"), (held.children, "children"), (held.name, "name")].into_iter().filter(|(set, _)| *set).map(|(_, word)| word).collect();
        if !what.is_empty() {
            out.push(Kind::ContentChanged { window: info.map(|i| i.id.clone()), app, what });
        }
    }

    /// Make the model equal to a full listing, telling each difference.
    fn reconcile(&mut self, listing: Vec<(Key, WindowInfo)>, unanswered: &[String], now: Instant, out: &mut Vec<Kind>) {
        self.counts.listings += 1;
        let gone: Vec<Key> = {
            let listed: HashSet<&Key> = listing.iter().map(|(key, _)| key).collect();
            self.order.iter().filter(|key| !listed.contains(key) && !unanswered.contains(&key.0)).cloned().collect()
        };
        for key in gone {
            self.close(&key, now, out);
        }
        for (key, info) in listing {
            match self.windows.get(&key).map(|known| known.title != info.title) {
                None => self.open(key, info, now, out),
                Some(true) => self.rename(key, info.title, now, out),
                Some(false) => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
