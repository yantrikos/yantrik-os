//! Holding back the instincts that search the web until the person has allowed it (#1171).
//!
//! `Companion::evaluate_instincts` hands every instinct's urges through [`hold_back_searches`],
//! the one place an `EXECUTE … web_search` urge can be dropped. The first time one is dropped with
//! the person at the desk, the offer card goes up (`yantrik_companion_tools::research_offer`).

use yantrik_companion_core::background_research;
use yantrik_companion_core::{CompanionState, UrgeSpec};

/// Seconds since the person last spoke within which they count as at the desk.
const PRESENT_WITHIN_SECS: f64 = 600.0;

/// `urges` without the search urges, while the switch is off.
pub(crate) fn hold_back_searches(urges: Vec<UrgeSpec>, state: &CompanionState) -> Vec<UrgeSpec> {
    held_back(urges, state.idle_seconds, background_research::is_on(), yantrik_companion_tools::research_offer::offer)
}

fn held_back(urges: Vec<UrgeSpec>, idle_seconds: f64, on: bool, offer: impl FnOnce(&str)) -> Vec<UrgeSpec> {
    let (kept, held) = background_research::gate(urges, on);
    if let Some(first) = held.first() {
        tracing::debug!(count = held.len(), "background research is off: search urges held back");
        if idle_seconds < PRESENT_WITHIN_SECS {
            offer(first);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn searching() -> Vec<UrgeSpec> {
        vec![
            UrgeSpec::new("NewsWatch", "EXECUTE Use web_search to search for \"news\".", 0.6),
            UrgeSpec::new("Scheduler", "EXECUTE remind the person about the dentist.", 0.9),
        ]
    }

    #[test]
    fn off_no_instinct_urges_a_search() {
        let kept = held_back(searching(), 5.0, false, |_| {});
        assert!(kept.iter().all(|u| !background_research::is_search_urge(u)));
        assert_eq!(kept.len(), 1, "the local-only urge stays");
    }

    #[test]
    fn the_offer_is_raised_only_with_the_person_here_and_only_when_something_was_held() {
        let asked = |idle, urges, on| {
            let mut who = None;
            held_back(urges, idle, on, |name| who = Some(name.to_string()));
            who
        };
        assert_eq!(asked(30.0, searching(), false).as_deref(), Some("NewsWatch"));
        assert_eq!(asked(7200.0, searching(), false), None, "nobody here, so no card");
        assert_eq!(asked(30.0, searching(), true), None, "already allowed");
        assert_eq!(asked(30.0, vec![UrgeSpec::new("Scheduler", "EXECUTE remind.", 0.9)], false), None, "nothing wanted to search");
    }

    #[test]
    fn evaluate_instincts_is_the_one_door() {
        let src = include_str!("companion.rs").split("#[cfg(test)]").next().unwrap();
        let body = src.split("pub fn evaluate_instincts").nth(1).unwrap().split("\n    }\n").next().unwrap();
        assert!(body.contains("crate::research::hold_back_searches(all_urges, state)"), "{body}");
        let others = src.matches(".evaluate(state)").count() + src.matches(".evaluate(&state)").count();
        assert_eq!(others, 1, "no other loop over the instincts");
    }
}
