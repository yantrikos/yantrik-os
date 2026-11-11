//! Background research: the companion searching the web on its own, which is off until the person
//! allows it (#1171).
//!
//! An instinct whose urge is `EXECUTE … web_search` sends search terms drawn from the person's
//! interests and place to a search service while nobody is at the keyboard. That is the one thing
//! a packet capture of an idle desktop would show, so nothing here starts it: `background_research`
//! in the config is `false` until the person presses Allow on the shell's card
//! (`allow_background_research`, in `yantrik-ui`), the one place that turns it on.
//!
//! Three things hang on the switch:
//! - [`gate`] holds back every search urge while it is off (the per-instinct flags stay beneath it);
//! - the search tool refuses an unattended call while it is off, and when it is on sends an
//!   unattended call to the person's SearXNG or else DuckDuckGo, never to Google in a browser;
//! - a person-started turn ([`person_turn`]) may use the browser, because the person asked.
//!
//! The switch is process-wide state the shell sets from the config at start; a program that never
//! sets it is off.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::types::UrgeSpec;

static ON: AtomicBool = AtomicBool::new(false);
static CONFIG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    /// How many turns this thread is running that a person started.
    static PERSON_TURNS: Cell<usize> = const { Cell::new(0) };
}

/// What the card says is asking, and what the Allow means. Names what is sent and to whom.
pub const OFFER: &str = "Allow this desktop's companion to search the web on its own, from now on, while you are \
    not asking: it would send search terms drawn from your interests and your place (news, deals, local events) \
    to your own SearXNG when you run one, and otherwise to DuckDuckGo, which sees them and your internet address. \
    Nothing is searched in the background until you allow it, and what was sent cannot be undone. To turn it off \
    again, set `background_research: false` in the machine's config.yaml.";

/// Set the switch from the config at start, and remember which file it came from.
pub fn init(on: bool, config_path: Option<PathBuf>) {
    ON.store(on, Ordering::SeqCst);
    *CONFIG_PATH.lock().unwrap_or_else(|e| e.into_inner()) = config_path;
}

/// The config file the shell was started with, when it was started with one.
pub fn config_path() -> Option<PathBuf> {
    CONFIG_PATH.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// May the companion search the web with nobody asking.
pub fn is_on() -> bool {
    ON.load(Ordering::SeqCst)
}

/// Record that the person allowed it. **The shell's card handler only**; the source-scan test in
/// `yantrik-ui` keeps every other file from calling it.
pub fn set_on() {
    ON.store(true, Ordering::SeqCst);
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

/// Is the search being made for a turn the person started. Anything that is not marked so (the
/// think timer, an instinct's urge, a queued task, a program on the socket) is unattended.
pub fn person_started() -> bool {
    PERSON_TURNS.with(|n| n.get() > 0)
}

/// An urge that tells the model to search the web.
pub fn is_search_urge(urge: &UrgeSpec) -> bool {
    urge.reason.starts_with("EXECUTE ") && urge.reason.contains("web_search")
}

/// `urges` with the search urges removed while research is off, and the names of the instincts
/// that were held back.
pub fn gate(urges: Vec<UrgeSpec>, on: bool) -> (Vec<UrgeSpec>, Vec<String>) {
    if on {
        return (urges, Vec::new());
    }
    let (held, kept): (Vec<UrgeSpec>, Vec<UrgeSpec>) = urges.into_iter().partition(is_search_urge);
    (kept, held.into_iter().map(|u| u.instinct_name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urge(reason: &str) -> UrgeSpec {
        UrgeSpec::new("test", reason, 0.7)
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
    fn off_holds_back_every_search_urge_and_keeps_the_rest() {
        let urges = vec![
            urge("EXECUTE Step 2: Use web_search to search for \"rust news\"."),
            urge("EXECUTE check the open loops and say what needs attention today."),
            urge("You have been working for four hours."),
        ];
        let (kept, withheld) = gate(urges.clone(), false);
        assert_eq!(withheld, ["test"]);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|u| !is_search_urge(u)));
        let (all, withheld) = gate(urges, true);
        assert_eq!(all.len(), 3, "on keeps them");
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
    fn the_offer_names_what_is_sent_and_to_whom() {
        for word in ["search terms", "SearXNG", "DuckDuckGo", "internet address", "cannot be undone"] {
            assert!(OFFER.contains(word), "{word}");
        }
        assert!(!OFFER.contains("Google"), "nothing in the background goes to Google");
    }
}
