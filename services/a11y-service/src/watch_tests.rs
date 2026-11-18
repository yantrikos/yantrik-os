//! The watcher's wiring, tested without a bus: what it asks, of whom, and what it does with a
//! refusal or a silence (#1257 review, L6, H1, N1.2, N5, N6, N9, N10, L1 to L4 and L7).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;

use super::*;
use crate::handles::Reference;
use crate::listing::Listing;
use crate::screen::WindowInfo;
use crate::typing::UNKNOWN_ROLE;

fn key(connection: &str, n: u32) -> Key {
    (connection.to_string(), format!("/org/a11y/atspi/accessible/{n}"))
}

fn info(n: u32, title: &str) -> WindowInfo {
    WindowInfo { id: format!("w{n}"), app: "gedit".into(), title: title.into(), role: "frame".into(), pid: 4242 }
}

/// What the bus would say, and which connections would say nothing, ever.
#[derive(Default)]
struct Fake {
    roles: RefCell<HashMap<Key, (String, String)>>,
    windows: RefCell<HashMap<Key, WindowInfo>>,
    active: RefCell<Vec<Key>>,
    names: RefCell<HashMap<Key, String>>,
    /// What a full listing finds, whom it names, and who did not answer.
    listed: RefCell<Vec<(Key, WindowInfo)>>,
    apps: RefCell<Vec<String>>,
    unanswered: RefCell<Vec<String>>,
    listing_fails: Cell<bool>,
    silent: RefCell<Vec<String>>,
    asked: RefCell<Vec<String>>,
    /// How long a listing blocks the watcher, and which applications it was told to skip.
    listing_takes: Cell<Duration>,
    skipped: RefCell<Vec<String>>,
}

impl Fake {
    fn hears(&self, what: &str, connection: &str) -> bool {
        self.asked.borrow_mut().push(format!("{what} {connection}"));
        !self.silent.borrow().iter().any(|silent| silent == connection)
    }

    fn knows_window(&self, key: Key, window: WindowInfo) {
        self.windows.borrow_mut().insert(key, window);
    }
}

impl Questions for Fake {
    async fn role_and_name(&self, r: &Reference) -> Option<(String, String)> {
        if !self.hears("role", &r.0) {
            std::future::pending::<()>().await;
        }
        self.roles.borrow().get(&key_of(r)).cloned()
    }

    async fn window(&self, r: &Reference) -> Option<WindowInfo> {
        if !self.hears("window", &r.0) {
            std::future::pending::<()>().await;
        }
        self.windows.borrow().get(&key_of(r)).cloned()
    }

    async fn parent_of(&self, r: &Reference) -> Option<Reference> {
        if !self.hears("parent", &r.0) {
            std::future::pending::<()>().await;
        }
        None
    }

    async fn is_active(&self, r: &Reference) -> bool {
        if !self.hears("active", &r.0) {
            std::future::pending::<()>().await;
        }
        self.active.borrow().contains(&key_of(r))
    }

    async fn name_of(&self, r: &Reference) -> Option<String> {
        if !self.hears("name", &r.0) {
            std::future::pending::<()>().await;
        }
        self.names.borrow().get(&key_of(r)).cloned()
    }

    async fn windows(&self, skip: &dyn Fn(&str) -> bool) -> Result<Listing, String> {
        std::thread::sleep(self.listing_takes.get());
        self.skipped.borrow_mut().extend(self.apps.borrow().iter().filter(|app| skip(app)).cloned());
        if self.listing_fails.get() {
            return Err("the registry is gone".into());
        }
        let windows = self.listed.borrow().iter().filter_map(|(key, window)| Some((reference_of(key)?, window.clone()))).collect();
        Ok(Listing { windows, apps: self.apps.borrow().clone(), unanswered: self.unanswered.borrow().iter().cloned().collect(), asked: Vec::new() })
    }

    async fn registry_owner(&self) -> Option<String> {
        Some(":1.1".into())
    }

    async fn register(&self, _event: &str) -> Result<(), String> {
        Ok(())
    }
}

fn never_private() -> bool {
    false
}

/// A runtime in which a question nobody answers is given up on at once, not after 400 ms.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().start_paused(true).build().expect("a runtime")
}

fn open(sight: &Sight, key: Key, window: WindowInfo) {
    sight.screen().apply(Fact::Opened(key, window), Instant::now());
}

fn kinds(sight: &Sight) -> Vec<String> {
    sight.since(0, Duration::ZERO)["events"].as_array().into_iter().flatten().filter_map(|e| e["kind"].as_str().map(str::to_string)).collect()
}

/// Run `body` with a watcher over `fake`, on a model that is not private.
fn with(fake: &Fake, private: fn() -> bool, body: impl FnOnce(&mut Watcher<Fake>, &Sight)) {
    let (runtime, sight, handles) = (runtime(), Sight::idle(), handles::Shared::default());
    let mut watcher = Watcher::new(&runtime, fake, &sight, &handles, private, ":1.1".into());
    body(&mut watcher, &sight);
}

/// Follow what is sent until it is all taken: the sender is dropped, so `follow` returns.
fn follow(watcher: &mut Watcher<Fake>, raws: Vec<Raw>) {
    let (tx, rx) = mpsc::sync_channel(RAW_QUEUE);
    for raw in raws {
        tx.send(raw).expect("room in the queue");
    }
    drop(tx);
    watcher.follow(&rx);
}

fn created(key: Key) -> Raw {
    Raw::Created(key)
}

// ---- N1.2 (M05): a focus that could not be learned is not a place to type in the clear ----

/// #1257 review, N1.2: not learning what the element is must not leave the old focus in place.
#[test]
fn a_focus_whose_role_cannot_be_learned_is_told_as_unknown_and_typing_in_it_is_not_held() {
    let fake = Fake::default();
    fake.silent.borrow_mut().push(":1.7".into());
    with(&fake, never_private, |watcher, sight| {
        open(sight, key(":1.7", 1), info(1, "login"));
        watcher.take(Raw::Focused(key(":1.7", 40)));
        assert_eq!(sight.screen().focus().map(|f| f.role.as_str()), Some(UNKNOWN_ROLE), "the focus moved, to something unknown");
        watcher.take(Raw::Text(key(":1.7", 40)));
        assert!(sight.screen().next_due().is_none(), "and its text changes are not held to be told");
    });
}

#[test]
fn typing_in_a_field_that_answered_is_held_to_be_told_as_activity() {
    let fake = Fake::default();
    fake.roles.borrow_mut().insert(key(":1.7", 40), ("text".into(), "Search".into()));
    with(&fake, never_private, |watcher, sight| {
        open(sight, key(":1.7", 1), info(1, "browser"));
        watcher.take(Raw::Focused(key(":1.7", 40)));
        watcher.take(Raw::Text(key(":1.7", 40)));
        assert!(sight.screen().next_due().is_some());
    });
}

// ---- N5 (M11, M12) and L1, L2: the private time ----

static BEGAN: AtomicBool = AtomicBool::new(false);

fn read_began() -> bool {
    BEGAN.load(Ordering::SeqCst)
}

/// #1257 review, N5 (M11): the person went private since the watcher last looked, and something
/// is about to be recorded. The fresh reading acts on the edge: nothing is recorded, and the
/// focus is forgotten.
#[test]
fn a_fact_told_in_the_first_moments_of_the_private_time_is_not_recorded() {
    let fake = Fake::default();
    fake.roles.borrow_mut().insert(key(":1.7", 40), ("text".into(), "Search".into()));
    with(&fake, read_began, |watcher, sight| {
        open(sight, key(":1.7", 1), info(1, "bank"));
        open(sight, key(":1.7", 2), info(2, "notes"));
        watcher.take(Raw::Focused(key(":1.7", 40)));
        assert!(sight.screen().focus().is_some());
        BEGAN.store(true, Ordering::SeqCst);
        watcher.take(Raw::Destroyed(key(":1.7", 2)));
        assert!(watcher.private, "the edge was acted on where it was first seen");
        assert!(sight.private_seen());
        assert!(sight.screen().focus().is_none(), "the focus was not kept into the private time");
        assert_eq!(kinds(sight), ["focus_moved"], "only what was told before: nothing of the private time was recorded");
        assert_eq!(sight.now()["counts"]["dropped_private"], 1);
        BEGAN.store(false, Ordering::SeqCst);
    });
}

static ENDED: AtomicBool = AtomicBool::new(false);

fn read_ended() -> bool {
    ENDED.load(Ordering::SeqCst)
}

/// #1257 review, N5 (M12): what the last session left on `Sight` is where this one starts, so a
/// private time that outlived a connection is told as a gap when it ends.
#[test]
fn a_session_starts_as_private_as_the_last_one_left_the_person() {
    let fake = Fake::default();
    let (runtime, sight, handles) = (runtime(), Sight::idle(), handles::Shared::default());
    sight.set_private_seen(true);
    let mut watcher = Watcher::new(&runtime, &fake, &sight, &handles, read_ended, ":1.1".into());
    assert!(watcher.private);
    watcher.look_at_privacy();
    assert_eq!(kinds(&sight), ["gap"], "the person is not private any more: the hole is told");
    assert!(!sight.private_seen());
}

static HERE_AND_BACK: AtomicU32 = AtomicU32::new(0);

fn read_private_again() -> bool {
    HERE_AND_BACK.fetch_add(1, Ordering::SeqCst) >= 1
}

/// #1257 review, L2: private again by the time the end is acted on.
#[test]
fn a_private_time_that_ends_and_begins_again_is_still_private() {
    let fake = Fake::default();
    let (runtime, sight, handles) = (runtime(), Sight::idle(), handles::Shared::default());
    sight.set_private_seen(true);
    let mut watcher = Watcher::new(&runtime, &fake, &sight, &handles, read_private_again, ":1.1".into());
    watcher.look_at_privacy();
    assert!(watcher.private, "the fresh reading said private again");
    assert!(sight.private_seen());
    assert!(kinds(&sight).is_empty(), "and the gap waits for the real end");
    watcher.take(Raw::Focused(key(":1.7", 1)));
    assert!(fake.asked.borrow().is_empty(), "nobody is asked where the focus is");
}

static SHORT_PRIVATE: AtomicBool = AtomicBool::new(false);

fn read_short_private() -> bool {
    SHORT_PRIVATE.load(Ordering::SeqCst)
}

/// #1257 review, L1: private goes on and off between two looks, a keystroke inside it. The
/// fresh reading is taken when the change begins to be held, so it is forgotten, not told two
/// seconds later.
#[test]
fn a_keystroke_held_through_a_short_private_time_is_forgotten() {
    let fake = Fake::default();
    with(&fake, read_short_private, |watcher, sight| {
        open(sight, key(":1.7", 1), info(1, "mail"));
        assert!(!watcher.private);
        SHORT_PRIVATE.store(true, Ordering::SeqCst);
        watcher.take(Raw::Text(key(":1.7", 40)));
        assert!(watcher.private && sight.private_seen(), "seen when the change began to be held");
        assert!(sight.screen().next_due().is_none(), "and not held: nothing will be told of it");
        SHORT_PRIVATE.store(false, Ordering::SeqCst);
    });
}

// ---- H1: a refused window is not lost ----

/// A connection that owes more than its share, until it has drained.
fn owing(watcher: &mut Watcher<Fake>, connection: &str) {
    watcher.patience.spent(connection, Duration::from_millis(60), Instant::now());
}

/// #1257 review, H1: the dialog's `window:create` is refused at `ask`; nothing tried again and
/// nothing asked for a listing.
#[test]
fn a_window_refused_for_lack_of_time_is_learned_later() {
    let fake = Fake::default();
    fake.knows_window(key(":1.9", 1), info(9, "Confirm payment"));
    fake.apps.borrow_mut().push(":1.9".into());
    with(&fake, never_private, |watcher, sight| {
        watcher.named.insert(":1.9".into());
        owing(watcher, ":1.9");
        watcher.take(created(key(":1.9", 1)));
        assert!(!sight.screen().knows(&key(":1.9", 1)), "refused");
        assert!(watcher.pending.is_asking(), "but kept");
        assert!(watcher.relists.due(Instant::now()), "and a listing, which the budget does not gate, is asked for");
        assert!(fake.asked.borrow().is_empty(), "nobody was asked");
        std::thread::sleep(Duration::from_millis(250));
        watcher.retry_pending();
        assert!(sight.screen().knows(&key(":1.9", 1)), "asked again once the connection had drained");
        assert!(!watcher.pending.is_asking());
        assert_eq!(kinds(sight), ["window_opened"]);
    });
}

#[test]
fn a_refused_window_that_took_the_front_takes_it_when_it_is_learned() {
    let fake = Fake::default();
    fake.knows_window(key(":1.9", 1), info(9, "Confirm payment"));
    with(&fake, never_private, |watcher, sight| {
        watcher.named.insert(":1.9".into());
        owing(watcher, ":1.9");
        watcher.take(Raw::Activated(key(":1.9", 1)));
        assert!(sight.screen().front().is_none());
        std::thread::sleep(Duration::from_millis(250));
        watcher.retry_pending();
        assert_eq!(sight.screen().front().map(|w| w.id.as_str()), Some("w9"));
    });
}

/// A window refused, then found by a listing before the retry: nothing is asked of it again.
#[test]
fn a_refused_window_that_a_listing_found_is_not_asked_about() {
    let fake = Fake::default();
    fake.listed.borrow_mut().push((key(":1.9", 1), info(9, "Confirm payment")));
    fake.apps.borrow_mut().push(":1.9".into());
    with(&fake, never_private, |watcher, sight| {
        watcher.named.insert(":1.9".into());
        owing(watcher, ":1.9");
        watcher.take(created(key(":1.9", 1)));
        follow(watcher, Vec::new());
        assert!(sight.screen().knows(&key(":1.9", 1)), "the listing found it");
        assert!(!watcher.pending.is_asking());
        assert!(fake.asked.borrow().is_empty());
    });
}

// ---- N6 (M17) and L3: the front ----

/// #1257 review, N6: one application that never answers must not leave the front unknown.
#[test]
fn the_front_is_found_though_one_application_never_answers_its_listing() {
    let fake = Fake::default();
    fake.listed.borrow_mut().push((key(":1.7", 1), info(1, "notes")));
    fake.apps.borrow_mut().extend([":1.7".to_string(), ":1.9".to_string()]);
    fake.unanswered.borrow_mut().push(":1.9".into());
    fake.active.borrow_mut().push(key(":1.7", 1));
    with(&fake, never_private, |watcher, sight| {
        follow(watcher, Vec::new());
        assert_eq!(sight.screen().front().map(|w| w.id.as_str()), Some("w1"));
    });
}

/// #1257 review, L3: a question that was refused did not find the front, and is not forgotten.
#[test]
fn the_front_is_asked_for_again_when_a_question_was_refused() {
    let fake = Fake::default();
    fake.listed.borrow_mut().push((key(":1.7", 1), info(1, "notes")));
    fake.apps.borrow_mut().push(":1.7".into());
    fake.active.borrow_mut().push(key(":1.7", 1));
    with(&fake, never_private, |watcher, sight| {
        watcher.patience.timed_out(":1.7", Instant::now());
        follow(watcher, Vec::new());
        assert!(sight.screen().front().is_none(), "nobody could be asked");
        assert!(!watcher.front_asked, "so it is not marked as done");
        assert!(watcher.relists.next().is_some(), "and another listing is wanted");
    });
}

// ---- N9 (M15): marking slow is counted where a reader can see it ----

#[test]
fn a_connection_that_does_not_answer_is_counted_where_a_reader_can_see_it() {
    let fake = Fake::default();
    fake.listed.borrow_mut().push((key(":1.7", 1), info(1, "notes")));
    fake.apps.borrow_mut().push(":1.7".into());
    fake.silent.borrow_mut().push(":1.7".into());
    with(&fake, never_private, |watcher, sight| {
        open(sight, key(":1.7", 1), info(1, "notes"));
        follow(watcher, Vec::new());
        watcher.take(Raw::Focused(key(":1.7", 40)));
        // A pass of the loop that has something to take ends by saying how many were marked.
        follow(watcher, vec![Raw::Vanished(":1.99".into())]);
        assert_eq!(sight.now()["counts"]["slow_marks"], 1);
    });
}

// ---- N10 (M16): a title that was queued before a listing is older than it ----

#[test]
fn a_title_signalled_before_a_listing_does_not_replace_the_title_the_listing_read() {
    let fake = Fake::default();
    fake.listed.borrow_mut().push((key(":1.7", 1), info(1, "title 2")));
    fake.apps.borrow_mut().push(":1.7".into());
    with(&fake, never_private, |watcher, sight| {
        open(sight, key(":1.7", 1), info(1, "title 0"));
        follow(watcher, vec![Raw::Name { source: key(":1.7", 1), name: Some("title 1".into()) }]);
        assert_eq!(sight.screen().windows().next().map(|w| w.title.as_str()), Some("title 2"), "the listing is the newer");
    });
}

// ---- L4: the front is not taken after the person moved ----

#[test]
fn a_window_that_took_the_front_before_it_was_known_does_not_take_it_back_from_a_later_one() {
    let fake = Fake::default();
    fake.listed.borrow_mut().extend([(key(":1.7", 1), info(1, "notes")), (key(":1.9", 1), info(9, "popup"))]);
    fake.apps.borrow_mut().push(":1.7".into());
    with(&fake, never_private, |watcher, sight| {
        open(sight, key(":1.7", 1), info(1, "notes"));
        // The popup claims the front, from a connection nobody knows; then the person goes to
        // notes.
        follow(watcher, vec![Raw::Activated(key(":1.9", 1)), Raw::Activated(key(":1.7", 1))]);
        assert_eq!(sight.screen().front().map(|w| w.id.as_str()), Some("w1"));
    });
}

#[test]
fn a_window_that_took_the_front_before_it_was_known_takes_it_once_a_listing_has_found_it() {
    let fake = Fake::default();
    fake.listed.borrow_mut().push((key(":1.9", 1), info(9, "popup")));
    with(&fake, never_private, |watcher, sight| {
        follow(watcher, vec![Raw::Activated(key(":1.9", 1))]);
        assert_eq!(sight.screen().front().map(|w| w.id.as_str()), Some("w9"));
    });
}

// ---- L7: a listing that keeps failing is counted ----

#[test]
fn a_listing_that_failed_is_counted_where_a_reader_can_see_it() {
    let fake = Fake::default();
    fake.listing_fails.set(true);
    with(&fake, never_private, |watcher, sight| {
        follow(watcher, Vec::new());
        assert_eq!(sight.now()["counts"]["failed_listings"], 1);
        assert!(watcher.relists.next().is_some(), "and it is tried again");
    });
}

/// The loop itself asks about the windows that were refused, not only a caller that knows to.
#[test]
fn the_loop_asks_again_about_a_window_that_was_refused() {
    let fake = Fake::default();
    fake.knows_window(key(":1.9", 1), info(9, "Confirm payment"));
    with(&fake, never_private, |watcher, sight| {
        watcher.named.insert(":1.9".into());
        owing(watcher, ":1.9");
        watcher.take(created(key(":1.9", 1)));
        assert!(!sight.screen().knows(&key(":1.9", 1)));
        std::thread::sleep(Duration::from_millis(250));
        fake.unanswered.borrow_mut().push(":1.9".into());
        follow(watcher, Vec::new());
        assert!(sight.screen().knows(&key(":1.9", 1)), "the listing found nothing, so it was the retry");
    });
}

/// #1257 review, M1: a listing blocks the watcher, and what it blocks it for is charged to
/// everyone together and spaces the next listing.
#[test]
fn a_listing_is_charged_for_the_time_it_took_and_the_next_waits_three_times_as_long() {
    let fake = Fake::default();
    fake.listing_takes.set(Duration::from_millis(400));
    with(&fake, never_private, |watcher, _| {
        watcher.patience.spent(":1.7", Duration::from_millis(40), Instant::now());
        follow(watcher, Vec::new());
        assert!(!watcher.patience.allows(":1.7", Instant::now()), "400 ms was charged: the total is spent, and it still owes more than a tenth of its share");
        watcher.relists.want();
        let wait = watcher.relists.next().map(|at| at.saturating_duration_since(Instant::now()));
        assert!(wait.is_some_and(|wait| wait > Duration::from_millis(1100)), "three times 400 ms: {wait:?}");
    });
}

/// #1257 review, M1: a listing leaves out the applications that owe their share.
#[test]
fn a_listing_is_told_to_skip_the_applications_that_owe_their_share() {
    let fake = Fake::default();
    fake.apps.borrow_mut().extend([":1.7".to_string(), ":1.8".to_string()]);
    with(&fake, never_private, |watcher, _| {
        owing(watcher, ":1.7");
        follow(watcher, Vec::new());
        assert_eq!(*fake.skipped.borrow(), [":1.7"]);
    });
}

/// #1257 review, L4: a window that lost the front before a listing found it is not made the front.
#[test]
fn a_window_that_deactivated_before_it_was_known_is_not_made_the_front() {
    let fake = Fake::default();
    fake.listed.borrow_mut().push((key(":1.9", 1), info(9, "popup")));
    with(&fake, never_private, |watcher, sight| {
        follow(watcher, vec![Raw::Activated(key(":1.9", 1)), Raw::Deactivated(key(":1.9", 1))]);
        assert!(sight.screen().knows(&key(":1.9", 1)));
        assert!(sight.screen().front().is_none());
    });
}
