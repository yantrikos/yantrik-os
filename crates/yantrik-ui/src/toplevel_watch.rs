//! Which window had the keyboard, in the order the compositor says it moved — not as a poll last
//! saw it.
//!
//! # Why a poll cannot answer this
//!
//! Clicking the taskbar entry of the window in front should put that window away, the way every
//! other desktop's taskbar does. The question "was it in front?" cannot be asked at the moment of
//! the click. The taskbar is drawn inside the shell's own window, and labwc gives focus to
//! whatever is clicked, so by the time the click reaches us the shell is the activated toplevel —
//! every time, whichever window was in front a moment earlier. The reading `windows` keeps is up
//! to nine seconds old (`COMPOSITOR_TTL`), which is long enough to have missed the person clicking
//! another window, or the desktop.
//!
//! What does answer it is the order of events. This listens to the compositor's foreign-toplevel
//! stream (`zwlr_foreign_toplevel_manager_v1`, the protocol `wlrctl` itself speaks) on a thread of
//! its own, and keeps the window that has focus, the one that had it before, and when the change
//! happened. "The shell took focus a moment ago, and before that it was Notes" is "Notes was in
//! front when the person pressed its entry".
//!
//! It only observes. Every change to a window still goes through `windows`, so there is one path
//! that moves windows and this is not a second one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self as handle, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self as manager, ZwlrForeignToplevelManagerV1,
};

use crate::windows::SHELL_WINDOW_TITLE;

/// How recently the shell must have taken focus for it to count as the click's own doing.
///
/// The focus moves when the button goes down and the entry's `clicked` runs when it comes up, so
/// for a click the two are a few hundred milliseconds apart at most. Longer than this and the
/// shell was already in front — the person was looking at the desktop — and the window the log
/// remembers from before is behind it, which is a window to bring forward, not to put away. A
/// press held longer than this falls on that side too, and costs only that: the window is brought
/// forward, which is where it already was.
const CLICK_TOOK_FOCUS: Duration = Duration::from_millis(1000);

static LOG: Mutex<FocusLog<ObjectId>> = Mutex::new(FocusLog::new());

/// The windows the stream has finished describing, for [`windows`].
static SNAPSHOT: Mutex<Vec<TopWindow>> = Mutex::new(Vec::new());
/// What [`activate`] needs: the handle of each window, the seat to name, and the connection to flush.
static HANDLES: Mutex<Vec<(u64, ZwlrForeignToplevelHandleV1)>> = Mutex::new(Vec::new());
static SEAT: Mutex<Option<WlSeat>> = Mutex::new(None);
static CONN: Mutex<Option<Connection>> = Mutex::new(None);

/// Whether the stream is being followed. While it is not, nothing here claims to know anything.
static LIVE: AtomicBool = AtomicBool::new(false);

/// Start following the compositor, once. A compositor that does not offer the protocol, or a
/// shell that is not on Wayland, leaves this off and the taskbar behaving as it did before: a
/// click brings the window forward and never puts it away.
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new().name("yos-toplevels".to_string()).spawn(|| {
            if let Err(why) = follow() {
                tracing::warn!(%why, "not following window focus; taskbar entries will only bring windows forward");
            }
            LIVE.store(false, Ordering::Relaxed);
            if let Ok(mut log) = LOG.lock() {
                *log = FocusLog::new();
            }
            if let Ok(mut s) = SNAPSHOT.lock() {
                s.clear();
            }
            if let Ok(mut h) = HANDLES.lock() {
                h.clear();
            }
            if let Ok(mut c) = CONN.lock() {
                *c = None;
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "could not start the thread that follows window focus");
        }
    });
}

/// How many toplevels carry the shell's title. More than one means a window is wearing it, and a
/// title alone can no longer say the shell is in front (card_watch).
static SHELL_TITLED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn shell_titled_count() -> usize {
    SHELL_TITLED.load(Ordering::Relaxed)
}

/// The title of the window in front now, or `None` when the stream is not being followed.
pub fn front_title() -> Option<String> {
    if !LIVE.load(Ordering::Relaxed) {
        return None;
    }
    LOG.lock().ok().and_then(|log| log.current.as_ref().map(|(_, t)| t.clone()))
}

/// One window as the compositor's stream describes it. `id` is the toplevel's protocol id: it
/// names this window and no other for as long as it exists, which a title does not — two
/// terminals can wear the same one.
#[derive(Clone, Debug, PartialEq)]
pub struct TopWindow {
    pub id: u64,
    pub title: String,
    pub app_id: String,
}

/// What the stream knows now, in the order the compositor announced the windows, or `None` when it
/// is not being followed. No process is run and nothing waits: this is a copy of what the stream
/// thread keeps, which is why the switcher reads it instead of asking `wlrctl`.
pub fn windows() -> Option<Vec<TopWindow>> {
    if !LIVE.load(Ordering::Relaxed) {
        return None;
    }
    SNAPSHOT.lock().ok().map(|s| s.clone())
}

/// The window in front now as (id, title), or `None` when the stream is not being followed.
pub fn front() -> Option<(u64, String)> {
    if !LIVE.load(Ordering::Relaxed) {
        return None;
    }
    LOG.lock().ok().and_then(|log| log.current.as_ref().map(|(k, t)| (id_of(k), t.clone())))
}

/// The ids of the windows that have had focus, the most recent first (see [`recency`]).
pub fn recency_ids() -> Vec<u64> {
    if !LIVE.load(Ordering::Relaxed) {
        return Vec::new();
    }
    LOG.lock().map(|log| log.recent.iter().map(|(k, _)| id_of(k)).collect()).unwrap_or_default()
}

/// Ask the compositor to activate window `id` (and un-minimise it), the request `wlrctl
/// toplevel focus` makes, but for exactly this window. `false` when the id is not a live window or
/// the stream is not followed, so the caller can fall back to a title. It only queues a request on
/// the connection; the answer is the focus event that comes back through the stream.
pub fn activate(id: u64) -> bool {
    let Some(conn) = CONN.lock().ok().and_then(|c| c.clone()) else { return false };
    let Some(seat) = SEAT.lock().ok().and_then(|s| s.clone()) else { return false };
    let Some(handle) = HANDLES.lock().ok().and_then(|h| h.iter().find(|(k, _)| *k == id).map(|(_, h)| h.clone())) else {
        return false;
    };
    handle.activate(&seat);
    conn.flush().is_ok()
}

fn id_of(id: &ObjectId) -> u64 {
    u64::from(id.protocol_id())
}

/// The titles of the windows that have had focus, the most recent first, or nothing when the
/// stream is not being followed. Windows that never had focus since the shell started are not
/// here; the switcher lists them after these, in the compositor's order.
pub fn recency() -> Vec<String> {
    if !LIVE.load(Ordering::Relaxed) {
        return Vec::new();
    }
    LOG.lock().map(|log| log.recent.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default()
}

/// Whether the window called `title` was the one in front when the person pressed its taskbar
/// entry. `false` whenever that is not known, because the cost of a wrong `true` is a window put
/// away that the person asked to see.
pub fn was_in_front(title: &str) -> bool {
    LIVE.load(Ordering::Relaxed)
        && LOG.lock().is_ok_and(|log| log.was_in_front(title, SHELL_WINDOW_TITLE, Instant::now()))
}

/// The focus history that matters for one click: who has focus, who had it before, and since when.
///
/// Generic over the key only so the tests can use numbers for windows; the real key is the
/// protocol object, which names a window across retitles where its title would not.
#[derive(Debug)]
struct FocusLog<K> {
    current: Option<(K, String)>,
    previous: Option<(K, String)>,
    since: Option<Instant>,
    /// Every window that has had focus and still exists, the one in front first. The overview's order.
    recent: Vec<(K, String)>,
}

impl<K: PartialEq + Clone> FocusLog<K> {
    const fn new() -> Self {
        Self { current: None, previous: None, since: None, recent: Vec::new() }
    }

    /// The compositor's latest word on which window is activated.
    ///
    /// `None` is not recorded. When focus moves, labwc sends the old window's deactivation and
    /// the new one's activation as two separate updates, and between them nothing is activated;
    /// recording that gap would push the window that really had focus out of `previous`.
    /// Returns whether focus moved to another window (a retitle is not a move).
    fn observe(&mut self, front: Option<(K, String)>, now: Instant) -> bool {
        let Some((key, title)) = front else { return false };
        match &mut self.current {
            // The same window under a new title (Chromium retitles on every tab): not a change
            // of focus, so `previous` and `since` stay as they are. The recency list follows the
            // title, because the overview shows it.
            Some((k, t)) if *k == key => {
                *t = title.clone();
                if let Some(entry) = self.recent.iter_mut().find(|(k, _)| *k == key) {
                    entry.1 = title;
                }
                false
            }
            _ => {
                self.recent.retain(|(k, _)| *k != key);
                self.recent.insert(0, (key.clone(), title.clone()));
                self.previous = self.current.take();
                self.current = Some((key, title));
                self.since = Some(now);
                true
            }
        }
    }

    /// A window went away. It is no longer the answer to "what was in front", whatever it was.
    fn closed(&mut self, key: &K) {
        self.recent.retain(|(k, _)| k != key);
        if self.previous.as_ref().is_some_and(|(k, _)| k == key) {
            self.previous = None;
        }
        if self.current.as_ref().is_some_and(|(k, _)| k == key) {
            self.current = None;
        }
    }

    fn was_in_front(&self, title: &str, shell: &str, now: Instant) -> bool {
        let Some((_, current)) = &self.current else { return false };
        if current != shell {
            // The click did not move focus to the shell (or the compositor has not said so yet):
            // whatever is activated now is what was in front.
            return current == title;
        }
        let just_now = self.since.is_some_and(|at| now.saturating_duration_since(at) <= CLICK_TOOK_FOCUS);
        just_now && self.previous.as_ref().is_some_and(|(_, t)| t == title)
    }
}

/// One window as the stream has described it so far. The protocol sends a window's title and
/// state as separate events and then `done`; nothing counts until `done`.
#[derive(Default)]
struct Toplevel {
    title: String,
    app_id: String,
    activated: bool,
    pending_title: Option<String>,
    pending_app_id: Option<String>,
    pending_activated: Option<bool>,
}

#[derive(Default)]
struct Watch {
    bound: bool,
    finished: bool,
    windows: Vec<(ObjectId, Toplevel)>,
}

impl Watch {
    /// How many toplevels carry the shell's title, for card_watch. Taken from the whole list on
    /// every change and every close, so it can never be left over from a window that went.
    fn count_shell_titled(&self) {
        SHELL_TITLED.store(
            self.windows.iter().filter(|(_, w)| w.title == SHELL_WINDOW_TITLE).count(),
            Ordering::Relaxed,
        );
    }

    /// Copy what is known into [`SNAPSHOT`]. A window that has not had its first `done` has no
    /// title yet and is not listed.
    fn publish(&self) {
        if let Ok(mut snap) = SNAPSHOT.lock() {
            *snap = self
                .windows
                .iter()
                .filter(|(_, w)| !w.title.is_empty())
                .map(|(k, w)| TopWindow { id: id_of(k), title: w.title.clone(), app_id: w.app_id.clone() })
                .collect();
        }
    }

    fn window(&mut self, id: ObjectId) -> &mut Toplevel {
        if let Some(i) = self.windows.iter().position(|(k, _)| *k == id) {
            return &mut self.windows[i].1;
        }
        self.windows.push((id, Toplevel::default()));
        &mut self.windows.last_mut().expect("just pushed").1
    }

    fn front(&self) -> Option<(ObjectId, String)> {
        self.windows.iter().find(|(_, w)| w.activated).map(|(k, w)| (k.clone(), w.title.clone()))
    }
}

fn follow() -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("could not reach the compositor: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut watch = Watch::default();
    queue
        .roundtrip(&mut watch)
        .map_err(|e| format!("the compositor did not answer: {e}"))?;
    if !watch.bound {
        return Err("the compositor does not offer zwlr_foreign_toplevel_manager_v1".to_string());
    }
    if let Ok(mut c) = CONN.lock() {
        *c = Some(conn.clone());
    }
    LIVE.store(true, Ordering::Relaxed);
    while !watch.finished {
        queue
            .blocking_dispatch(&mut watch)
            .map_err(|e| format!("the compositor's window stream ended: {e}"))?;
    }
    Err("the compositor stopped sending window updates".to_string())
}

impl Dispatch<WlRegistry, ()> for Watch {
    fn event(
        watch: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            if interface == WlSeat::interface().name {
                let seat = registry.bind::<WlSeat, _, _>(name, version.min(1), qh, ());
                if let Ok(mut s) = SEAT.lock() {
                    s.get_or_insert(seat);
                }
            }
            if interface == ZwlrForeignToplevelManagerV1::interface().name && !watch.bound {
                registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(name, version.min(3), qh, ());
                watch.bound = true;
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for Watch {
    fn event(_: &mut Self, _: &WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Watch {
    fn event(
        watch: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Toplevel { toplevel } => {
                watch.window(toplevel.id());
                if let Ok(mut h) = HANDLES.lock() {
                    h.push((id_of(&toplevel.id()), toplevel));
                }
            }
            manager::Event::Finished => watch.finished = true,
            _ => {}
        }
    }

    event_created_child!(Watch, ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Watch {
    fn event(
        watch: &mut Self,
        toplevel: &ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = toplevel.id();
        match event {
            handle::Event::Title { title } => watch.window(id).pending_title = Some(title),
            handle::Event::AppId { app_id } => watch.window(id).pending_app_id = Some(app_id),
            handle::Event::State { state } => {
                watch.window(id).pending_activated = Some(is_activated(&state));
            }
            handle::Event::Done => {
                let w = watch.window(id);
                if let Some(title) = w.pending_title.take() {
                    w.title = title;
                }
                if let Some(app_id) = w.pending_app_id.take() {
                    w.app_id = app_id;
                }
                if let Some(activated) = w.pending_activated.take() {
                    w.activated = activated;
                }
                watch.count_shell_titled();
                watch.publish();
                let front = watch.front();
                let title = front.as_ref().map(|(_, t)| t.clone());
                let moved = LOG.lock().is_ok_and(|mut log| log.observe(front, Instant::now()));
                // A card waiting in the shell must not stay behind whatever just took focus, and
                // the shell coming forward, by whatever path, starts the press guard.
                if moved {
                    if let Some(title) = title {
                        if title == SHELL_WINDOW_TITLE {
                            crate::card_watch::shell_came_forward();
                        }
                        crate::card_watch::front_changed(&title);
                    }
                }
            }
            handle::Event::Closed => {
                watch.windows.retain(|(k, _)| *k != id);
                if let Ok(mut h) = HANDLES.lock() {
                    h.retain(|(k, _)| *k != id_of(&id));
                }
                watch.publish();
                // Counted again on a close too: a window that wore the shell's title and went
                // left the count at two, and the real shell coming forward then read as a cover
                // and spent the raises (second review).
                watch.count_shell_titled();
                if let Ok(mut log) = LOG.lock() {
                    log.closed(&id);
                }
                toplevel.destroy();
            }
            _ => {}
        }
    }
}

/// The protocol's state is an array of native-endian u32 values, one per state the window is in.
fn is_activated(state: &[u8]) -> bool {
    let activated = handle::State::Activated as u32;
    state
        .chunks_exact(4)
        .any(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]) == activated)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: &str = "Yantrik OS";

    fn log() -> (FocusLog<u32>, Instant) {
        (FocusLog::new(), Instant::now())
    }

    /// The bug: Notes in front, its entry pressed. The press gave the shell focus first, so asking
    /// at click time says "the shell". The order of events still says Notes.
    #[test]
    fn the_window_in_front_before_the_press_is_the_one_that_was_in_front() {
        let (mut log, t) = log();
        log.observe(Some((1, "Notes".into())), t);
        log.observe(Some((0, SHELL.into())), t + Duration::from_millis(5));
        assert!(log.was_in_front("Notes", SHELL, t + Duration::from_millis(150)));
        assert!(!log.was_in_front("Terminal", SHELL, t + Duration::from_millis(150)));
    }

    /// The person was on the desktop, and Notes is behind it. Its entry brings it forward.
    #[test]
    fn a_window_behind_the_desktop_was_not_in_front() {
        let (mut log, t) = log();
        log.observe(Some((1, "Notes".into())), t);
        log.observe(Some((0, SHELL.into())), t + Duration::from_secs(1));
        assert!(!log.was_in_front("Notes", SHELL, t + Duration::from_secs(5)));
    }

    /// If the compositor has not reported the shell taking focus yet, what is activated is still
    /// the window that was in front.
    #[test]
    fn focus_that_has_not_moved_yet_is_still_the_answer() {
        let (mut log, t) = log();
        log.observe(Some((1, "Notes".into())), t);
        assert!(log.was_in_front("Notes", SHELL, t + Duration::from_secs(30)));
        assert!(!log.was_in_front("Terminal", SHELL, t + Duration::from_secs(30)));
    }

    /// Between one window's deactivation and the next one's activation nothing is activated.
    /// That gap must not push the window that had focus out of `previous`.
    #[test]
    fn the_gap_between_two_windows_is_not_a_window() {
        let (mut log, t) = log();
        log.observe(Some((1, "Notes".into())), t);
        log.observe(None, t + Duration::from_millis(2));
        log.observe(Some((0, SHELL.into())), t + Duration::from_millis(3));
        assert!(log.was_in_front("Notes", SHELL, t + Duration::from_millis(100)));
    }

    /// A browser that retitles itself while in front is the same window, still in front, and
    /// the window before it is still the one before it.
    #[test]
    fn a_retitle_is_not_a_change_of_focus() {
        let (mut log, t) = log();
        log.observe(Some((1, "Notes".into())), t);
        log.observe(Some((2, "Inbox - Chromium".into())), t + Duration::from_secs(1));
        log.observe(Some((2, "News - Chromium".into())), t + Duration::from_secs(2));
        assert!(log.was_in_front("News - Chromium", SHELL, t + Duration::from_secs(3)));
        log.observe(Some((0, SHELL.into())), t + Duration::from_secs(4));
        assert!(log.was_in_front("News - Chromium", SHELL, t + Duration::from_millis(4100)));
        assert!(!log.was_in_front("Inbox - Chromium", SHELL, t + Duration::from_millis(4100)));
    }

    /// A window that closed was not in front of anything, even under a title a new one reuses.
    #[test]
    fn a_closed_window_is_not_in_front() {
        let (mut log, t) = log();
        log.observe(Some((1, "Notes".into())), t);
        log.observe(Some((0, SHELL.into())), t + Duration::from_millis(5));
        log.closed(&1);
        assert!(!log.was_in_front("Notes", SHELL, t + Duration::from_millis(100)));
    }

    /// The overview's order: the window that had focus last comes first, a retitle follows the window
    /// without moving it, and a closed window leaves the list.
    #[test]
    fn recency_is_most_recent_first_and_follows_retitles_and_closes() {
        let (mut log, t) = log();
        log.observe(Some((1, "Notes".into())), t);
        log.observe(Some((2, "Inbox - Chromium".into())), t);
        log.observe(Some((3, "Terminal".into())), t);
        log.observe(Some((1, "Notes".into())), t);
        fn titles(l: &FocusLog<u32>) -> Vec<&str> {
            l.recent.iter().map(|(_, t)| t.as_str()).collect()
        }
        assert_eq!(titles(&log), ["Notes", "Terminal", "Inbox - Chromium"]);
        log.observe(Some((1, "Notes: Handover".into())), t);
        assert_eq!(titles(&log), ["Notes: Handover", "Terminal", "Inbox - Chromium"]);
        log.closed(&3);
        assert_eq!(titles(&log), ["Notes: Handover", "Inbox - Chromium"]);
    }

    /// With nothing observed yet, nothing is claimed.
    #[test]
    fn nothing_known_is_nothing_claimed() {
        let (log, t) = log();
        assert!(!log.was_in_front("Notes", SHELL, t));
        assert!(!log.was_in_front(SHELL, SHELL, t));
    }

    #[test]
    fn the_activated_state_is_read_from_the_array() {
        let bytes = |v: &[u32]| v.iter().flat_map(|x| x.to_ne_bytes()).collect::<Vec<u8>>();
        assert!(is_activated(&bytes(&[0, 2])));
        assert!(!is_activated(&bytes(&[0, 1])));
        assert!(!is_activated(&[]));
    }
}
