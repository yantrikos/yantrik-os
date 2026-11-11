//! The offer to let the companion search the web on its own (#1171).
//!
//! While background research is off, an instinct that wants to search is held back
//! (`yantrik_companion_core::background_research::gate`). The first time that happens with the
//! person at the desk, the shell shows an approval card, `allow_background_research`, saying what
//! would be sent and to whom, and its Allow turns the switch on. This is the same card
//! `get_weather` raises for `allow_location_lookup`: the shell writes the switch itself, so
//! nothing here (or any tool, mind or model) can answer for the person.
//!
//! The card waits as long as any card does (`APPROVAL_WAIT`, then the shell expires it). Nothing
//! is sent meanwhile, and when it lapses the next think tick with the person still here raises
//! it again; a No is remembered for this run.

use std::sync::atomic::{AtomicBool, Ordering};

use yantrik_companion_core::background_research;

use crate::app_ui::{act_asking, Desk};

/// The shell's action that holds the answer, and the app it belongs to.
const APP: &str = "shell";
const ACTION: &str = "allow_background_research";

/// A card is up, or was said No to: do not raise another.
static ASKING: AtomicBool = AtomicBool::new(false);
static REFUSED: AtomicBool = AtomicBool::new(false);

/// Put the offer in front of the person, off this thread (the card waits for an answer, and the
/// think loop must not). At most one card at a time, none after a No.
pub fn offer(wanted_by: &str) {
    if background_research::is_on() || REFUSED.load(Ordering::SeqCst) || ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let why = format!("The companion's {wanted_by} wanted to search the web and nothing was sent.");
    let spawned = std::thread::Builder::new().name("research-offer".into()).spawn(move || {
        let answer = act_asking(&Desk::session(), APP, ACTION, &serde_json::json!({}), "", &why);
        if let Err(said) = &answer {
            REFUSED.store(said.contains("said no"), Ordering::SeqCst);
            tracing::info!(%said, "the background-research offer was not accepted");
        }
        ASKING.store(false, Ordering::SeqCst);
    });
    if spawned.is_err() {
        ASKING.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_asks_the_shells_action_and_the_wording_is_the_cores() {
        assert_eq!(ACTION, "allow_background_research");
        let src = include_str!("research_offer.rs").split("#[cfg(test)]").next().unwrap();
        assert!(src.contains("act_asking("), "the offer is the approval card, not a new mechanism");
        assert!(!src.contains("set_on"), "this file never writes the switch");
    }

    #[test]
    fn a_no_ends_the_offers_and_a_card_already_up_is_not_doubled() {
        REFUSED.store(true, Ordering::SeqCst);
        offer("news watch");
        assert!(!ASKING.load(Ordering::SeqCst), "after a No no card is raised");
        REFUSED.store(false, Ordering::SeqCst);
        ASKING.store(true, Ordering::SeqCst);
        offer("news watch");
        assert!(ASKING.load(Ordering::SeqCst), "the one in flight is left alone");
        ASKING.store(false, Ordering::SeqCst);
    }
}
