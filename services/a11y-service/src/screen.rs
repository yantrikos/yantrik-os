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
//! never carries the changed text. A text-changed signal brings the typed characters with it and
//! they are dropped at the door; whoever wants the content calls `a11y.describe`, which has its
//! own rules.
//!
//! # Not a transcript
//!
//! Accessibility is a firehose: a page load is thousands of signals. Changes inside a window are
//! therefore told at most once per [`COALESCE`]: the first one at once, so a reader learns within
//! milliseconds that the window is changing, and whatever follows inside the interval as one more
//! event with a count.
//!
//! Pure on purpose: no bus, no clock of its own, so every rule here has a test.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;

/// The longest a window's changes are held back to be told as one.
pub const COALESCE: Duration = Duration::from_millis(150);

/// How long "the front window lost the front" waits for the next window to take it, so a switch
/// between two windows is told as one change and not as two.
pub const FRONT_GRACE: Duration = Duration::from_millis(60);

/// Throttle records kept at most. Past this the oldest state is dropped wholesale: the cost is one
/// change told a little early, never one withheld.
const MAX_THROTTLED: usize = 512;

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
#[derive(Debug, Clone)]
pub enum Fact {
    /// Every window there is, from a walk of the registry. The model is reconciled to it.
    Listing(Vec<(Key, WindowInfo)>),
    Opened(Key, WindowInfo),
    Closed(Key),
    Activated(Key),
    Deactivated(Key),
    Renamed(Key, String),
    /// The keyboard focus moved to an element (its role and name), in a window if one was found.
    Focus { window: Option<Key>, role: String, name: String },
    /// Something inside a window changed. `window` is `None` when only the application is known.
    Touched { window: Option<Key>, app: String, what: What },
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
    /// Something inside the window changed; `signals` is how many announcements this stands for.
    ContentChanged { window: Option<String>, app: String, what: Vec<&'static str>, signals: u32 },
    /// Announcements were lost. What follows is true again, but a reader should read `a11y.now`.
    Gap,
}

/// Where the keyboard focus is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Focus {
    pub window: Option<String>,
    pub role: String,
    pub name: String,
}

/// Counted, never described: how much the model saw, told and folded away.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Counts {
    /// Announcements taken in.
    pub facts: u64,
    /// Events told.
    pub told: u64,
    /// Announcements folded into an event that stood for several.
    pub folded: u64,
    /// Times the model was reconciled to a full listing.
    pub listings: u64,
    /// Times announcements were lost.
    pub gaps: u64,
}

/// Changes inside one window (or one application) not yet told.
#[derive(Debug, Default)]
struct Held {
    text: bool,
    children: bool,
    name: bool,
    renamed: bool,
    signals: u32,
}

/// The throttle for one window or application: when it last spoke, and what it is holding.
#[derive(Debug)]
struct Throttle {
    last_told: Instant,
    held: Option<Held>,
}

/// Who a held change belongs to: a window, or an application whose window is not known.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Target {
    Window(Key),
    App(String),
}

#[derive(Default)]
pub struct Screen {
    windows: Vec<(Key, WindowInfo)>,
    front: Option<Key>,
    /// Set when the front window lost the front and nothing has taken it yet.
    front_lost_at: Option<Instant>,
    focus: Option<Focus>,
    throttles: HashMap<Target, Throttle>,
    counts: Counts,
}

impl Screen {
    pub fn knows(&self, key: &Key) -> bool {
        self.windows.iter().any(|(k, _)| k == key)
    }

    fn info(&self, key: &Key) -> Option<&WindowInfo> {
        self.windows.iter().find(|(k, _)| k == key).map(|(_, info)| info)
    }

    /// The name of the application on a connection, from any window of it that is known.
    pub fn app_on(&self, connection: &str) -> Option<&str> {
        self.windows.iter().find(|((bus, _), _)| bus == connection).map(|(_, info)| info.app.as_str())
    }

    /// Whether any known window belongs to this connection.
    pub fn has_connection(&self, connection: &str) -> bool {
        self.windows.iter().any(|((bus, _), _)| bus == connection)
    }

    /// Every window's key, for asking each one something.
    pub fn keys(&self) -> Vec<Key> {
        self.windows.iter().map(|(key, _)| key.clone()).collect()
    }

    pub fn windows(&self) -> impl Iterator<Item = &WindowInfo> {
        self.windows.iter().map(|(_, info)| info)
    }

    pub fn front(&self) -> Option<&WindowInfo> {
        self.front.as_ref().and_then(|key| self.info(key))
    }

    pub fn focus(&self) -> Option<&Focus> {
        self.focus.as_ref()
    }

    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// Take one fact in and say what a reader should be told at once.
    pub fn apply(&mut self, fact: Fact, now: Instant) -> Vec<Kind> {
        self.counts.facts += 1;
        let mut out = Vec::new();
        match fact {
            Fact::Listing(listing) => self.reconcile(listing, now, &mut out),
            Fact::Opened(key, info) => self.open(key, info, &mut out),
            Fact::Closed(key) => self.close(&key, now, &mut out),
            Fact::Activated(key) => self.activate(key, &mut out),
            Fact::Deactivated(key) => {
                if self.front.as_ref() == Some(&key) && self.front_lost_at.is_none() {
                    self.front_lost_at = Some(now);
                }
            }
            Fact::Renamed(key, title) => self.rename(key, title, now, &mut out),
            Fact::Focus { window, role, name } => {
                let focus = Focus { window: window.as_ref().and_then(|k| self.info(k)).map(|i| i.id.clone()), role, name };
                if self.focus.as_ref() != Some(&focus) {
                    out.push(Kind::FocusMoved { window: focus.window.clone(), role: focus.role.clone(), name: focus.name.clone() });
                    self.focus = Some(focus);
                }
            }
            Fact::Touched { window, app, what } => {
                // A window that has gone is not one to report on; its application still is.
                let target = match window.filter(|key| self.knows(key)) {
                    Some(key) => Target::Window(key),
                    None => Target::App(app),
                };
                self.touch(target, now, &mut out, |held| match what {
                    What::Text => held.text = true,
                    What::Children => held.children = true,
                    What::Name => held.name = true,
                });
            }
            Fact::Gap => {
                self.counts.gaps += 1;
                out.push(Kind::Gap);
            }
        }
        self.counts.told += out.len() as u64;
        out
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
        let due: Vec<Target> = self
            .throttles
            .iter()
            .filter(|(_, t)| t.held.is_some() && now.duration_since(t.last_told) >= COALESCE)
            .map(|(target, _)| target.clone())
            .collect();
        for target in due {
            let held = self.throttles.get_mut(&target).and_then(|t| {
                t.last_told = now;
                t.held.take()
            });
            if let Some(held) = held {
                self.tell_held(&target, held, &mut out);
            }
        }
        self.counts.told += out.len() as u64;
        out
    }

    /// When `flush` next has something to say, if anything is waiting.
    pub fn next_due(&self) -> Option<Instant> {
        let held = self.throttles.values().filter(|t| t.held.is_some()).map(|t| t.last_told + COALESCE);
        held.chain(self.front_lost_at.map(|at| at + FRONT_GRACE)).min()
    }

    fn open(&mut self, key: Key, info: WindowInfo, out: &mut Vec<Kind>) {
        if self.knows(&key) {
            return;
        }
        out.push(Kind::WindowOpened { window: info.clone() });
        self.windows.push((key, info));
    }

    fn close(&mut self, key: &Key, now: Instant, out: &mut Vec<Kind>) {
        let Some(at) = self.windows.iter().position(|(k, _)| k == key) else { return };
        let (key, info) = self.windows.remove(at);
        self.throttles.remove(&Target::Window(key.clone()));
        if self.focus.as_ref().is_some_and(|f| f.window.as_deref() == Some(info.id.as_str())) {
            self.focus = None;
        }
        if self.front.as_ref() == Some(&key) && self.front_lost_at.is_none() {
            self.front_lost_at = Some(now);
        }
        out.push(Kind::WindowClosed { window: info });
    }

    fn activate(&mut self, key: Key, out: &mut Vec<Kind>) {
        let Some(info) = self.info(&key).cloned() else { return };
        // Whatever takes the front ends the wait for a taker. The same window taking it back
        // inside the grace was never told as lost, so for a reader nothing changed.
        self.front_lost_at = None;
        if self.front.as_ref() != Some(&key) {
            out.push(Kind::FrontChanged { window: Some(info) });
            self.front = Some(key);
        }
    }

    fn rename(&mut self, key: Key, title: String, now: Instant, out: &mut Vec<Kind>) {
        let Some((_, info)) = self.windows.iter_mut().find(|(k, _)| *k == key) else { return };
        if info.title == title {
            return;
        }
        // The model has the new title at once; a reader is told through the throttle, so a
        // window that retitles itself every frame is one event per interval.
        info.title = title;
        self.touch(Target::Window(key), now, out, |held| held.renamed = true);
    }

    /// One change for `target`: told at once when the target has been quiet, held otherwise.
    fn touch(&mut self, target: Target, now: Instant, out: &mut Vec<Kind>, mark: impl FnOnce(&mut Held)) {
        if self.throttles.len() >= MAX_THROTTLED && !self.throttles.contains_key(&target) {
            self.throttles.retain(|_, t| t.held.is_some());
        }
        let quiet = self.throttles.get(&target).is_none_or(|t| t.held.is_none() && now.duration_since(t.last_told) >= COALESCE);
        if quiet {
            let mut held = Held { signals: 1, ..Held::default() };
            mark(&mut held);
            self.throttles.insert(target.clone(), Throttle { last_told: now, held: None });
            self.tell_held(&target, held, out);
            return;
        }
        let throttle = self.throttles.entry(target).or_insert(Throttle { last_told: now, held: None });
        let held = throttle.held.get_or_insert_with(Held::default);
        if held.signals > 0 {
            self.counts.folded += 1;
        }
        held.signals += 1;
        mark(held);
    }

    fn tell_held(&self, target: &Target, held: Held, out: &mut Vec<Kind>) {
        let (info, app) = match target {
            // A window that closed while its change was held has nothing left to report on.
            Target::Window(key) => match self.info(key) {
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
            out.push(Kind::ContentChanged { window: info.map(|i| i.id.clone()), app, what, signals: held.signals });
        }
    }

    /// Make the model equal to a full listing, telling each difference.
    fn reconcile(&mut self, listing: Vec<(Key, WindowInfo)>, now: Instant, out: &mut Vec<Kind>) {
        self.counts.listings += 1;
        let gone: Vec<Key> = self.windows.iter().filter(|(k, _)| !listing.iter().any(|(l, _)| l == k)).map(|(k, _)| k.clone()).collect();
        for key in gone {
            self.close(&key, now, out);
        }
        for (key, info) in listing {
            match self.info(&key).map(|known| known.title.clone()) {
                None => self.open(key, info, out),
                Some(title) if title != info.title => self.rename(key, info.title, now, out),
                Some(_) => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
