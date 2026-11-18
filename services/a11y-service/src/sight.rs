//! The reader's side of the screen model: the model itself, and the ring of what changed (#1256).
//!
//! The watcher (`watch`) writes here; `a11y.now` and `a11y.since` read. One rule ties the two
//! halves together: every push to the ring is made with the model's lock held, so an answer of
//! `a11y.now` and the `seq` it carries are from the same instant, and the first `a11y.since`
//! after it neither repeats nor skips a change.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use yantrik_service_sdk::ring::CursorRing;

use crate::handles;
use crate::screen::{Kind, Screen};
use crate::signals::Tally;
use crate::throttle::TEXT_COALESCE;

/// Events kept for readers that have not caught up.
const RING: usize = 1024;

/// Events handed back by one `a11y.since` at most. A reader that is further behind asks again.
const MAX_PAGE: usize = 256;

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
    /// How many times the watcher marked a connection slow. It moves when an application is
    /// slow or the watcher is late, never once per keystroke.
    slow_marks: AtomicU64,
    /// Full listings that did not work, so a model left stale is visible. A listing is rationed,
    /// so this never moves once per keystroke either.
    failed_listings: AtomicU64,
    /// Whether the person was private at the last look the watcher acted on, and no gap has been
    /// told since. Here and not in the watcher, so a new session tells it.
    private_seen: AtomicBool,
    /// The signal reader's count and flag, kept across reconnections.
    signals: Arc<Tally>,
}

/// The model and the ring, as the watcher and the service share them.
#[derive(Clone)]
pub struct Sight {
    screen: Arc<Mutex<Screen>>,
    ring: CursorRing<Event>,
    meters: Arc<Meters>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The time an event is stamped with. An event with text in it says which step of
/// [`TEXT_COALESCE`] it was told in and no more: one text change is one keystroke, so a
/// millisecond would be the rhythm of someone's typing, and that holds whatever else the event
/// says (#1257 review, N1 and M2). The model stays pure; the time is decided where the event
/// gets one, which is also why a text event's time can be a little before an earlier event's.
fn stamp(kind: &Kind, ms: u64) -> u64 {
    match kind {
        Kind::ContentChanged { what, .. } if what.contains(&"text") => {
            let step = TEXT_COALESCE.as_millis() as u64;
            ms / step * step
        }
        _ => ms,
    }
}

impl Sight {
    /// A model nothing feeds yet: what the watcher starts from, and what a test hands the
    /// service.
    pub fn idle() -> Self {
        Self { screen: Arc::default(), ring: CursorRing::new(RING), meters: Arc::default() }
    }

    /// The model. Whoever changes it and has something to tell calls [`Sight::record`] before
    /// letting go of this lock.
    pub fn screen(&self) -> MutexGuard<'_, Screen> {
        self.screen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Put what the model said into the ring, unless the person is private. A window's handle
    /// is kept for as long as the window is open and forgotten when it closes, private or not.
    /// Called with the model's lock held.
    pub fn record(&self, handles: &handles::Shared, private: bool, told: Vec<Kind>) {
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
            self.ring.push_with(|seq| Event { seq, at_ms: stamp(&kind, now_ms()), kind });
        }
    }

    /// The signal reader's count of what it dropped, and its overflow flag.
    pub fn tally(&self) -> Arc<Tally> {
        Arc::clone(&self.meters.signals)
    }

    /// Whether the reader dropped signals since this was last asked.
    pub fn take_overflow(&self) -> bool {
        self.meters.signals.overflowed.swap(false, Ordering::Relaxed)
    }

    /// The watcher's count of connections marked slow, so a reader can see whether honest
    /// applications are being marked because the watcher itself was late.
    pub fn set_slow_marks(&self, marks: u64) {
        self.meters.slow_marks.store(marks, Ordering::Relaxed);
    }

    /// One more full listing did not work.
    pub fn count_failed_listing(&self) {
        self.meters.failed_listings.fetch_add(1, Ordering::Relaxed);
    }

    /// Whether the person was private and the gap has not been told yet.
    pub fn private_seen(&self) -> bool {
        self.meters.private_seen.load(Ordering::Relaxed)
    }

    pub fn set_private_seen(&self, seen: bool) {
        self.meters.private_seen.store(seen, Ordering::Relaxed);
    }

    /// Say whether the model is being fed. Returns whether it was before.
    pub fn set_listening(&self, listening: bool) -> bool {
        self.meters.listening.swap(listening, Ordering::Relaxed)
    }

    /// What is on screen, from the model: no application is asked anything.
    pub fn now(&self) -> serde_json::Value {
        let listening = self.meters.listening.load(Ordering::Relaxed);
        let screen = self.screen();
        serde_json::json!({
            // The cursor for `a11y.since`: everything after this answer. Read under the model's
            // lock, which every push to the ring is made under.
            "seq": self.ring.next_seq(),
            // False means the model is not being fed (no accessibility bus yet, or it went
            // away): what follows is the last it knew, and `a11y.windows` is the way to look.
            "listening": listening,
            "front": screen.front(),
            "focus": screen.focus(),
            "windows": screen.windows().collect::<Vec<_>>(),
            // None of these moves once per keystroke: a counter that did would say how much was
            // typed to anyone who read it twice.
            "counts": {
                "model": screen.counts(),
                "signals_dropped": self.meters.signals.dropped.load(Ordering::Relaxed),
                "dropped_private": self.meters.dropped_private.load(Ordering::Relaxed),
                "slow_marks": self.meters.slow_marks.load(Ordering::Relaxed),
                "failed_listings": self.meters.failed_listings.load(Ordering::Relaxed),
                "overwritten": self.ring.overwritten(),
            },
        })
    }

    /// What changed from `seq` on, waiting up to `wait` for the first change.
    pub fn since(&self, seq: u64, wait: Duration) -> serde_json::Value {
        let page = self.ring.since(seq, wait, MAX_PAGE);
        serde_json::json!({
            "events": page.items,
            "next_seq": page.next_seq,
            // Non-zero: this reader fell off the back of the ring, and should read `a11y.now`.
            "missed": page.missed,
            "more": page.more,
            // True: `seq` was ahead of the ring (a service that restarted), so it was read from
            // the ring's own next event, and the reader should read `a11y.now`.
            "reset": page.reset,
        })
    }
}

#[cfg(test)]
#[path = "sight_tests.rs"]
mod tests;
