//! Background research: the companion reaching the web on its own, which is off until the person
//! allows it (#1171).
//!
//! An instinct that searches sends terms drawn from the person's interests and place to a search
//! service while nobody is at the keyboard. That is the one thing a packet capture of an idle
//! desktop would show, so nothing here starts it: `background_research` in the config is `false`
//! until the person presses Allow on the shell's card (`allow_background_research`, in
//! `yantrik-ui`), the one place that turns it on.
//!
//! The rule, as a class and not a list of words: **no tool that reaches the open web runs in a
//! turn nobody started, unless research is on** ([`crate::reach`], enforced once, in
//! `ToolRegistry::execute`). Three things hang on the switch:
//! - [`gate`] holds back the urges of every instinct that declares `searches_web`, so the model is
//!   never even asked;
//! - the registry refuses an unattended call to a web tool while it is off, and a browser tool
//!   always, and raises the offer card for it ([`wanted`]);
//! - when it is on, an unattended `web_search` goes to the person's SearXNG or else DuckDuckGo,
//!   never to Google in a browser; a turn the person started may use the browser.
//!
//! A person's "No" on the card is kept in a file ([`decline`]) so the card does not return after
//! a restart; it is asked again only when the person asks to be ([`forgive`]).
//!
//! The switch is process-wide state the shell sets from the config at start; a program that never
//! sets it is off.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::types::UrgeSpec;

static ON: AtomicBool = AtomicBool::new(false);
static DECLINED: AtomicBool = AtomicBool::new(false);
static CONFIG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Seconds since the person last spoke, as the think loop last saw it (the bits of an `f64`);
/// nobody is here until it says so.
static IDLE_SECS: AtomicU64 = AtomicU64::new(f64::INFINITY.to_bits());
/// What raises the offer card, installed by the companion (the card lives in a crate above this one).
static ON_WANTED: OnceLock<fn(&str)> = OnceLock::new();
/// What redraws a screen that shows the switch, installed by the shell.
static ON_CHANGED: OnceLock<fn()> = OnceLock::new();

/// Seconds since the person last spoke within which they count as at the desk.
pub const PRESENT_WITHIN_SECS: f64 = 600.0;

/// The file a person's No is kept in, under the person's config directory.
const DECLINED_FILE: &str = "yantrik/background-research-declined";

thread_local! {
    /// How many turns this thread is running that a person started.
    static PERSON_TURNS: Cell<usize> = const { Cell::new(0) };
}

/// What the card says is asking, and what the Allow means. Names what is sent and to whom.
pub const OFFER: &str = "Allow this desktop's companion to search the web on its own, from now on, while you are \
    not asking: it would send search terms drawn from your interests and your place (news, deals, local events) \
    to your own SearXNG when you run one, and otherwise to DuckDuckGo, which sees them and your internet address. \
    Nothing is searched in the background until you allow it, and what was sent cannot be undone. You can turn \
    it off again in Settings, under Minds.";

/// Set the switch from the config at start, remember which file it came from, and read whether the
/// person said No before.
pub fn init(on: bool, config_path: Option<PathBuf>) {
    ON.store(on, Ordering::SeqCst);
    DECLINED.store(declined_marker().is_some_and(|p| p.exists()), Ordering::SeqCst);
    *CONFIG_PATH.lock().unwrap_or_else(|e| e.into_inner()) = config_path;
}

/// The config file the shell was started with, when it was started with one.
pub fn config_path() -> Option<PathBuf> {
    CONFIG_PATH.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// May the companion reach the web with nobody asking.
pub fn is_on() -> bool {
    ON.load(Ordering::SeqCst)
}

/// Record that the person allowed it. **The shell's control file only** (`control_research.rs`);
/// its source-scan test keeps every other file from calling it.
pub fn set_on() {
    ON.store(true, Ordering::SeqCst);
    changed();
}

/// Record that the person turned it off. Same rule as [`set_on`].
pub fn set_off() {
    ON.store(false, Ordering::SeqCst);
    changed();
}

/// Install what redraws the screen that shows the switch and a No (Settings). It runs after the
/// switch or the No changes, whoever changed it: a card's answer, a mind's action or the row itself.
pub fn on_changed(redraw: fn()) {
    let _ = ON_CHANGED.set(redraw);
}

fn changed() {
    if let Some(redraw) = ON_CHANGED.get() {
        redraw();
    }
}

/// Did the person say No to the card, and has not asked to be asked again.
pub fn declined() -> bool {
    DECLINED.load(Ordering::SeqCst)
}

fn declined_marker() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(DECLINED_FILE))
}

/// Remember a No, here and in the file. Called by the offer when the card is answered No.
pub fn decline() -> std::io::Result<()> {
    DECLINED.store(true, Ordering::SeqCst);
    let kept = declined_marker().map_or(Ok(()), |p| write_marker(&p));
    changed();
    kept
}

/// Forget a No, so the card may be raised again. **Only when the person asks to be asked**, or
/// allows (`control_research.rs`).
pub fn forgive() -> std::io::Result<()> {
    DECLINED.store(false, Ordering::SeqCst);
    let forgotten = declined_marker().map_or(Ok(()), |p| clear_marker(&p));
    changed();
    forgotten
}

fn write_marker(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, "declined\n")
}

fn clear_marker(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Held while a turn the person started runs on this thread.
pub struct PersonTurn(());

/// Mark this thread's turn as one a person started, until the returned guard is dropped.
pub fn person_turn() -> PersonTurn {
    PERSON_TURNS.with(|n| n.set(n.get() + 1));
    PersonTurn(())
}

impl Drop for PersonTurn {
    fn drop(&mut self) {
        PERSON_TURNS.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// Is the call being made for a turn the person started. Anything that is not marked so (the
/// think timer, an instinct's urge, a queued task, a program on the socket) is unattended.
pub fn person_started() -> bool {
    PERSON_TURNS.with(|n| n.get() > 0)
}

/// This thread's mark, to be carried onto a thread that works for the turn (`adopt`), the way
/// `taint::carry` carries what the turn has read. A sub-agent the person's request fans out to is
/// still the person's request.
#[derive(Clone, Copy)]
pub struct Carried(bool);

/// Whether this thread is running a turn a person started, for `adopt` on another thread.
pub fn carry() -> Carried {
    Carried(person_started())
}

/// Continue a carried turn on this thread until the returned guard is dropped.
pub fn adopt(carried: Carried) -> Option<PersonTurn> {
    carried.0.then(person_turn)
}

/// Tell the tool layer how long the person has been silent, as the think loop last saw it.
pub fn note_idle(seconds: f64) {
    IDLE_SECS.store(seconds.to_bits(), Ordering::SeqCst);
}

/// Install what raises the offer card; the first caller wins.
pub fn on_wanted(raise: fn(&str)) {
    let _ = ON_WANTED.set(raise);
}

/// Something wanted to reach the web while research is off and `by` was held back: raise the offer
/// card, when the person is at the desk, has not said No and it is not already on.
pub fn wanted(by: &str) {
    let here = f64::from_bits(IDLE_SECS.load(Ordering::SeqCst)) < PRESENT_WITHIN_SECS;
    if let (true, false, false, Some(raise)) = (here, is_on(), declined(), ON_WANTED.get()) {
        raise(by);
    }
}

/// `urges` with those of instincts that search removed while research is off, and the names of the
/// instincts that were held back.
pub fn gate(urges: Vec<UrgeSpec>, on: bool) -> (Vec<UrgeSpec>, Vec<String>) {
    if on {
        return (urges, Vec::new());
    }
    let (held, kept): (Vec<UrgeSpec>, Vec<UrgeSpec>) = urges.into_iter().partition(|u| u.searches);
    (kept, held.into_iter().map(|u| u.instinct_name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urge(reason: &str, searches: bool) -> UrgeSpec {
        UrgeSpec { searches, ..UrgeSpec::new("test", reason, 0.7) }
    }

    #[test]
    fn it_is_off_until_something_sets_it() {
        assert!(!crate::config::CompanionConfig::default().background_research);
        let parsed: crate::config::CompanionConfig = serde_yaml::from_str("user_name: x\n").unwrap();
        assert!(!parsed.background_research, "a config that says nothing leaves it off");
        let on: crate::config::CompanionConfig = serde_yaml::from_str("background_research: true\n").unwrap();
        assert!(on.background_research);
    }

    #[test]
    fn off_holds_back_what_is_declared_to_search_whatever_its_words_say() {
        let urges = vec![
            urge("EXECUTE Use web_search to search for \"rust news\".", true),
            urge("EXECUTE Then look around for \"rust news\" and tell them.", true),
            urge("EXECUTE check the open loops and say what needs attention today.", false),
            urge("You have been working for four hours.", false),
            urge("EXECUTE Step 2: Use web_search to settle it.", false),
        ];
        let (kept, withheld) = gate(urges.clone(), false);
        assert_eq!(withheld, ["test", "test"], "the declaration decides");
        assert_eq!(kept.len(), 3, "an urge nobody declared is not read for words");
        let (all, withheld) = gate(urges, true);
        assert_eq!(all.len(), 5, "on keeps them");
        assert!(withheld.is_empty());
    }

    #[test]
    fn a_turn_is_unattended_unless_a_person_started_it() {
        assert!(!person_started(), "the default is unattended");
        {
            let _outer = person_turn();
            let _inner = person_turn();
            assert!(person_started());
        }
        assert!(!person_started(), "the mark ends with the turn");
        let _mine = person_turn();
        std::thread::spawn(|| assert!(!person_started(), "another thread's turn is not this one's")).join().unwrap();
    }

    #[test]
    fn a_turn_the_person_started_carries_onto_the_threads_that_work_for_it() {
        let _mine = person_turn();
        let carried = carry();
        std::thread::spawn(move || {
            assert!(!person_started());
            let _adopted = adopt(carried);
            assert!(person_started(), "a sub-agent of the person's request is the person's");
        })
        .join()
        .unwrap();
        let unattended = std::thread::spawn(carry).join().unwrap();
        std::thread::spawn(move || assert!(adopt(unattended).is_none() && !person_started())).join().unwrap();
    }

    #[test]
    fn a_screen_that_shows_the_switch_is_told_when_it_changes() {
        static REDRAWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        on_changed(|| {
            REDRAWS.fetch_add(1, Ordering::SeqCst);
        });
        let before = REDRAWS.load(Ordering::SeqCst);
        // Off is what the test process already is, so this changes nothing else a test reads.
        set_off();
        assert_eq!(REDRAWS.load(Ordering::SeqCst), before + 1, "turning it off redraws Settings");
    }

    #[test]
    fn a_no_is_kept_in_a_file_until_the_person_asks_to_be_asked() {
        let dir = std::env::temp_dir().join(format!("yantrik-declined-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let marker = dir.join(DECLINED_FILE);
        assert!(!marker.exists());
        write_marker(&marker).unwrap();
        assert!(marker.exists(), "the No survives a restart");
        clear_marker(&marker).unwrap();
        assert!(!marker.exists());
        clear_marker(&marker).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_offer_names_what_is_sent_and_to_whom() {
        for word in ["search terms", "SearXNG", "DuckDuckGo", "internet address", "cannot be undone", "Settings"] {
            assert!(OFFER.contains(word), "{word}");
        }
        assert!(!OFFER.contains("Google"), "nothing in the background goes to Google");
    }
}
