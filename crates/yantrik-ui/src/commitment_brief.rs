//! The brief card's rows about preparing promised things (#1184, round 3: HIGH-B).
//!
//! The yes to "Shall I get these ready for promises like this?" is the person's alone: it is a
//! click on a row of the morning brief card, which the shell hands to
//! `CompanionService::person_answers_commitment_prep` (from `wire::morning_brief`, the Slint
//! callback a click arrives on). No tool, recipe step or Mind can call it, and no `control*.rs`
//! action names it (`no_control_action_gives_the_answer`).

use yantrik_companion::commitment_consent::{BriefAction, NO, TURN_ON, YES};
use yantrik_companion::CompanionService;

use crate::bridge::MorningBriefSectionData;

/// The rows to add to the brief card.
pub fn sections(companion: &CompanionService) -> Vec<MorningBriefSectionData> {
    companion.commitment_brief_actions().into_iter().map(row).collect()
}

fn row(a: BriefAction) -> MorningBriefSectionData {
    MorningBriefSectionData { icon: a.icon.to_string(), label: a.label, content: a.content, expanded: true, action_id: a.action_id }
}

/// Whether a section's action id is one of the three buttons of the answer.
pub fn is_answer(action_id: &str) -> bool {
    action_id.starts_with(YES) || action_id.starts_with(NO) || action_id == TURN_ON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_buttons_are_the_answer_and_navigation_is_not() {
        assert!(is_answer("commitment_prep:yes:commitment-prep-3"));
        assert!(is_answer("commitment_prep:no:commitment-prep-3"));
        assert!(is_answer("commitment_prep:on"));
        assert!(!is_answer("navigate:email"));
        assert!(!is_answer(""));
    }

    /// The answer is a click and nothing else: no published action of the shell's control surface
    /// may name it, or a caller on the socket could give the yes.
    #[test]
    fn no_control_action_gives_the_answer() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("control") || !name.ends_with(".rs") {
                continue;
            }
            let whole = std::fs::read_to_string(entry.path()).unwrap();
            let src = whole.split("#[cfg(test)]").next().unwrap_or("");
            for needle in ["person_answers_commitment_prep", "commitment_consent", "answer_commitment_prep"] {
                assert!(!src.contains(needle), "{name} names {needle}");
            }
        }
    }
}
