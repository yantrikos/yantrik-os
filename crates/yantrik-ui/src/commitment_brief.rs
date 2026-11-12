//! The person's answer about preparing promised things, as the shell shows it (#1184).
//!
//! The yes to "Shall I get these ready for promises like this?" is the person's alone. It is a real
//! button with its own callback (`PrepState.answer`, wired in `wire::commitment_prep`), on the
//! morning brief card and in Settings → Companion; never the expand toggle of a row. It reaches
//! `CompanionService::person_answers_commitment_prep` and nothing else does: no tool, recipe step or
//! Mind can call it, and no `control*.rs` action names it (`no_control_action_gives_the_answer`).

use yantrik_companion::commitment_consent::{no_for, BriefAction, YES};
use yantrik_companion::CompanionService;

use crate::bridge::MorningBriefSectionData;

/// What a press said back, and what the surfaces show after it.
pub type Reply = yantrik_companion::commitment_consent::Reply;

/// The rows to add to the brief card.
pub fn sections(companion: &CompanionService) -> Vec<MorningBriefSectionData> {
    companion.commitment_brief_actions().into_iter().map(row).collect()
}

/// The row for the question outstanding.
pub fn question_row(q: &yantrik_companion::commitment_consent::Question) -> MorningBriefSectionData {
    row(yantrik_companion::commitment_consent::question_action(q))
}

fn row(a: BriefAction) -> MorningBriefSectionData {
    MorningBriefSectionData { icon: a.icon.to_string(), label: a.label, content: a.content, expanded: true, action_id: a.action_id }
}

/// How the card draws a section: its buttons, when it is a question, and the action id its row
/// fires when clicked, which is empty for a question so that folding it answers nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    pub row_action_id: String,
    pub yes_id: String,
    pub yes_label: &'static str,
    pub no_id: String,
    pub no_label: &'static str,
}

pub fn drawn(action_id: &str) -> Drawn {
    use yantrik_companion::commitment_consent::TURN_ON;
    if action_id == TURN_ON {
        return Drawn { yes_id: action_id.into(), yes_label: "Turn on", ..Drawn::default() };
    }
    match no_for(action_id) {
        Some(no_id) if action_id.starts_with(YES) => Drawn { yes_id: action_id.into(), yes_label: "Yes", no_id, no_label: "No", ..Drawn::default() },
        _ => Drawn { row_action_id: action_id.into(), ..Drawn::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_has_buttons_and_its_row_fires_nothing_so_folding_it_never_answers() {
        let d = drawn("commitment_prep:yes:commitment-prep-3");
        assert_eq!(d.row_action_id, "", "the row's expand toggle has no action to fire");
        assert_eq!((d.yes_id.as_str(), d.no_id.as_str()), ("commitment_prep:yes:commitment-prep-3", "commitment_prep:no:commitment-prep-3"));
        let on = drawn("commitment_prep:on");
        assert_eq!((on.row_action_id.as_str(), on.yes_id.as_str(), on.no_id.as_str()), ("", "commitment_prep:on", ""));
    }

    #[test]
    fn other_sections_keep_their_row_action() {
        let d = drawn("navigate:email");
        assert_eq!((d.row_action_id.as_str(), d.yes_id.as_str()), ("navigate:email", ""));
        assert_eq!(drawn(""), Drawn::default());
    }

    /// LOW-6: the prepared bar says the draft cannot be sent while it carries the file.
    #[test]
    fn the_prepared_bar_says_it_cannot_be_sent_with_the_file() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui/email_prepared.slint")).unwrap();
        assert!(src.contains("Prepared by Yantrik. Attached: \" + root.file + \". Email can't send attachments yet; remove it to send.\""), "the bar's sentence");
    }

    /// Every button of the answer is a real press: pointer-only, and calling `PrepState.answer`.
    #[test]
    fn the_buttons_are_pointer_only_presses_of_their_own() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui/commitment_prep.slint")).unwrap();
        assert_eq!(src.matches("YButton {").count(), src.matches("pointer-only: true;").count());
        assert!(!src.contains("section-action"));
    }

    /// `section-action` is the row's callback: it must never answer. Only `PrepState.answer` does.
    #[test]
    fn the_rows_section_action_never_answers() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wire/morning_brief.rs")).unwrap();
        let (_, after) = src.split_once("fn wire_brief_section_action").expect("the handler");
        let handler = after.split("#[cfg(test)]").next().unwrap();
        for needle in ["answer_commitment_prep", "is_answer", "commitment_prep:"] {
            assert!(!handler.contains(needle), "section-action names {needle}");
        }
    }

    /// The answer is a press and nothing else: no published action of the shell's control surface
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
            for needle in ["person_answers_commitment_prep", "commitment_consent", "commitment_cards", "answer_commitment_prep"] {
                assert!(!src.contains(needle), "{name} names {needle}");
            }
        }
    }

    /// Only the one wire file listens to the buttons, and only through the person's press.
    #[test]
    fn only_the_press_callback_reaches_the_answer() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![dir.join("wire")];
        files.push(dir.clone());
        for d in files {
            for entry in std::fs::read_dir(d).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".rs") || name == "commitment_brief.rs" || name == "bridge.rs" || name == "commitment_prep.rs" {
                    continue;
                }
                let src = std::fs::read_to_string(entry.path()).unwrap();
                let src = src.split("#[cfg(test)]").next().unwrap_or("");
                assert!(!src.contains("answer_commitment_prep"), "{name} calls the answer");
            }
        }
        // In the one file that does: a press of `PrepState.answer` is the only thing that carries an
        // action id (brief card, shell cards and Settings all press it); the poll's call carries none.
        let wire = std::fs::read_to_string(dir.join("wire/commitment_prep.rs")).unwrap();
        let wire = wire.split("#[cfg(test)]").next().unwrap_or("");
        assert_eq!(wire.matches("on_answer(").count(), 1, "one listener");
        assert_eq!(wire.matches("answer_commitment_prep(").count(), 2, "the press, and the poll");
        assert!(wire.contains("answer_commitment_prep(String::new())"), "the poll answers nothing");
    }

    /// The shell cards (the question, and what was prepared with Turn off) are the person's alone too:
    /// drawn only by `PrepCard`, whose buttons are `PrepChoices` (pointer-only, `PrepState.answer`),
    /// never by a notification or a whisper action a socket program can invoke, and named by no
    /// file of the notification store's wiring or the control surface.
    #[test]
    fn the_shell_cards_are_pointer_only_person_only_and_named_by_no_notification_path() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui");
        let prep = std::fs::read_to_string(ui.join("commitment_prep.slint")).unwrap();
        let card = prep.split_once("export component PrepCard").expect("the card").1;
        assert!(card.contains("PrepChoices {") && !card.contains("YButton {") && !card.contains("TouchArea"), "its buttons are PrepChoices, nothing clickable of its own");
        let stack = std::fs::read_to_string(ui.join("components/whisper_card.slint")).unwrap();
        assert!(stack.contains("PrepCard { card: prep; }"), "the stack draws the card");
        assert!(!stack.contains("PrepState.answer"), "the stack never presses the answer");
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for name in ["wire/notifications.rs", "wire/for_you.rs", "wire/callbacks.rs", "cards.rs", "notice_inbox.rs"] {
            let whole = std::fs::read_to_string(src.join(name)).unwrap();
            let code = whole.split("#[cfg(test)]").next().unwrap_or("");
            for needle in ["commitment_cards", "answer_commitment_prep", "PrepState"] {
                assert!(!code.contains(needle), "{name} names {needle}");
            }
        }
    }

    /// LOW-2: a press that answered nothing says so, and is never dropped silently.
    #[test]
    fn a_press_on_a_stale_question_says_so() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wire/commitment_prep.rs")).unwrap();
        assert!(src.contains("None => stale(&ui, &status)"), "a press with no answer goes to stale");
        assert!(src.contains("That question has changed; the current one is in {WHERE}."));
    }
}
