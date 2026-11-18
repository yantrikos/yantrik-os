//! Holding back the instincts that search the web until the person has allowed it (#1171).
//!
//! `Companion::evaluate_instincts` hands the instincts to [`evaluate`], the one place an urge is
//! marked as a search (by what its instinct declares with `Instinct::searches_web`, not by what it
//! says) and dropped while research is off. The first time one is dropped with the person at the
//! desk, the offer card goes up (`yantrik_companion_tools::research_offer`); the same card is raised
//! when a tool is refused for reaching the web in a turn nobody started (`reach::check_call`).

use yantrik_companion_core::background_research;
use yantrik_companion_core::instincts::Instinct;
use yantrik_companion_core::{CompanionState, UrgeSpec};

/// Every instinct's urges for this tick, without those of the instincts that search while the
/// switch is off.
pub(crate) fn evaluate(instincts: &[Box<dyn Instinct>], state: &CompanionState) -> Vec<UrgeSpec> {
    // The tool layer raises the same card when a refused call finds the person here.
    background_research::on_wanted(yantrik_companion_tools::research_offer::offer);
    background_research::note_idle(state.idle_seconds);
    let mut urges = Vec::new();
    for instinct in instincts {
        urges.extend(marked(instinct.as_ref(), instinct.evaluate(state)));
    }
    held_back(urges, state.idle_seconds, background_research::is_on(), yantrik_companion_tools::research_offer::offer)
}

/// `urges` of `instinct`, each marked as a search when the instinct declares that it searches.
fn marked(instinct: &dyn Instinct, urges: Vec<UrgeSpec>) -> impl Iterator<Item = UrgeSpec> {
    let searches = instinct.searches_web();
    urges.into_iter().map(move |u| UrgeSpec { searches: u.searches || searches, ..u })
}

fn held_back(urges: Vec<UrgeSpec>, idle_seconds: f64, on: bool, offer: impl FnOnce(&str)) -> Vec<UrgeSpec> {
    let (kept, held) = background_research::gate(urges, on);
    if let Some(first) = held.first() {
        tracing::debug!(count = held.len(), "background research is off: search urges held back");
        if idle_seconds < background_research::PRESENT_WITHIN_SECS {
            offer(first);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn searching() -> Vec<UrgeSpec> {
        let mut news = UrgeSpec::new("NewsWatch", "EXECUTE Then search for \"news\".", 0.6);
        news.searches = true;
        vec![news, UrgeSpec::new("Scheduler", "EXECUTE remind the person about the dentist.", 0.9)]
    }

    #[test]
    fn off_no_declared_instinct_urges_a_search() {
        let kept = held_back(searching(), 5.0, false, |_| {});
        assert_eq!(kept.len(), 1, "the local-only urge stays");
        assert!(kept.iter().all(|u| !u.searches));
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
    fn an_instincts_declaration_marks_all_its_urges_and_evaluate_instincts_is_the_one_door() {
        struct Searcher;
        impl Instinct for Searcher {
            fn name(&self) -> &str {
                "Searcher"
            }
            fn searches_web(&self) -> bool {
                true
            }
            fn evaluate(&self, _: &CompanionState) -> Vec<UrgeSpec> {
                vec![]
            }
        }
        let said = vec![UrgeSpec::new("Searcher", "EXECUTE look around for news.", 0.5)];
        let marked: Vec<UrgeSpec> = marked(&Searcher, said).collect();
        assert!(marked.iter().all(|u| u.searches));
        assert!(held_back(marked, 5.0, false, |_| {}).is_empty(), "off: held, whatever its words say");
        let src = include_str!("companion.rs").split("#[cfg(test)]").next().unwrap();
        let body = src.split("pub fn evaluate_instincts").nth(1).unwrap().split("\n    }\n").next().unwrap();
        assert!(body.contains("crate::research::evaluate(&self.instincts, state)"), "{body}");
        let others = src.matches(".evaluate(state)").count() + src.matches(".evaluate(&state)").count();
        assert_eq!(others, 0, "no other loop over the instincts");
    }
}
