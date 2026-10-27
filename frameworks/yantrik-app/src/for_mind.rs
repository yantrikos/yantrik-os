//! Whether a window is drawn for a Mind: it is in Mind View, where `read_mind_view` turns every
//! line on it into text for an agent.
//!
//! An app with content an agent must not read (a snippet's code, a note) masks it in such a
//! window, and keeps what is safe to show: titles, tags, counts. It is a property of the window,
//! not of the app (#963): one instance may have the person's window and a Mind's open at once,
//! and only the Mind's is masked ([`masked`]). A window opened for a Mind's launch says so in its
//! id ([`crate::WindowId::for_mind`]), decided from the launch's own environment by the rule
//! below (`crate::displays::asker`); a copy that runs on a Mind's display itself
//! (`Boot::for_mind`) masks every window it draws.
//!
//! It fails closed. A launch is a Mind's when either says so:
//! - the launch carries the marker [`FOR_MIND_VAR`], which the shell sets on every app it starts
//!   on a mind's display; or
//! - the window's Wayland display is not the person's own, by the same comparison `present` makes
//!   of two displays ([`present::elsewhere`]): a launch that lost the marker but landed on Mind
//!   View is still masked.
//!
//! The person's display is taken to be [`PERSON_DISPLAY`] (`wayland-0`), as every shipping setup
//! has it. A session on another display is masked throughout: that is the fail-closed side, so an
//! app's sentence must not say a Mind is looking when only this rule decided it.
//!
//! The marker can only turn masking on. Nothing here reads a variable that turns it off.

use crate::present;

pub use yantrik_ipc_contracts::mind_view::{FOR_MIND_VAR, PERSON_DISPLAY};

/// Whether a window with this launch is drawn for a Mind: `marker` is the value of
/// [`FOR_MIND_VAR`] and `display` that of `WAYLAND_DISPLAY`, both as the process has them.
///
/// Any value of the marker but an empty one counts, so `0` and `false` still mask. With no display
/// named the window is not on Wayland and the display rule has nothing to compare.
pub fn drawn_for_mind(marker: Option<&str>, display: Option<&str>) -> bool {
    marker.is_some_and(|m| !m.is_empty()) || present::elsewhere(display, Some(PERSON_DISPLAY))
}

/// [`drawn_for_mind`], for this process.
pub fn from_env() -> bool {
    let marker = std::env::var(FOR_MIND_VAR).ok();
    let display = std::env::var("WAYLAND_DISPLAY").ok();
    drawn_for_mind(marker.as_deref(), display.as_deref())
}

/// Whether `window` is masked: it is a Mind's ([`crate::WindowId::for_mind`]), or this whole copy
/// draws on a Mind's display (`instance`, from `Boot::for_mind`). The one rule every app's `render`
/// asks, per window, so the person's window is never masked because a Mind's is open.
pub fn masked(instance: bool, window: crate::WindowId) -> bool {
    instance || window.for_mind()
}

/// What a window shows where a Mind may be looking and the person's content would be: the one
/// sentence every app uses, so the mask reads the same in Snippets, Notes, Email and Text Editor.
/// It is the owner's wording and it is shown whenever the rule above masks, including a window
/// masked by its display alone, so it says that a Mind may be looking and does not assert one is.
pub const HIDDEN: &str = "Hidden while a Mind is looking.";

/// Why a command is refused in a masked window, and the one sentence for all of them. The rule is
/// that a window on a Mind's display is a window the Mind drives, so nothing may run through it
/// as the person:
/// - an edit (typing, paste, drag-in, IME, undo, redo, replace) would change content the window
///   cannot show;
/// - a file read or write (open, import, export, save, save as, print) would use the person's
///   rights over the person's files;
/// - sending or quoting (send, reply, forward) would carry content out.
///
/// Copy is here only where it duplicates content (Notes); the clipboard itself holds the mask.
pub const READ_ONLY: &str = "This is read-only while a Mind is looking.";

/// The content as the window shows it: the content itself for the person, [`HIDDEN`] for a Mind.
pub fn shown(for_mind: bool, content: &str) -> std::borrow::Cow<'_, str> {
    if for_mind {
        std::borrow::Cow::Borrowed(HIDDEN)
    } else {
        std::borrow::Cow::Borrowed(content)
    }
}

/// [`shown`] for content that may be empty: an empty field stays empty, since a mask over nothing
/// would say there is something to hide.
pub fn shown_if_any(for_mind: bool, content: &str) -> std::borrow::Cow<'_, str> {
    if content.is_empty() { std::borrow::Cow::Borrowed("") } else { shown(for_mind, content) }
}

/// A notice line as a window shows it: in a masked window, the framework's own sentences
/// ([`HIDDEN`], [`READ_ONLY`]) as they are, since they say nothing of the person's content, and any
/// other notice masked, since one raised by the person's own window may name what they work on.
pub fn shown_notice(masked: bool, notice: &str) -> std::borrow::Cow<'_, str> {
    if notice == READ_ONLY || notice == HIDDEN {
        std::borrow::Cow::Borrowed(notice)
    } else {
        shown_if_any(masked, notice)
    }
}

/// The read-only rule: a command that edits, duplicates, reads or writes a file, or sends out masked
/// content, refused with [`READ_ONLY`] where a Mind is looking. An app calls this before anything
/// else for each command of its own that would change or carry out the content, so the rule and
/// its sentence are not copied per app.
pub fn refuse_if(for_mind: bool, would_run_as_person: bool) -> Result<(), crate::Refused> {
    if for_mind && would_run_as_person {
        Err(crate::Refused::new("for_mind", READ_ONLY))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_turns_masking_on_whatever_the_display() {
        assert!(drawn_for_mind(Some("1"), Some(PERSON_DISPLAY)));
        assert!(drawn_for_mind(Some("1"), None));
        assert!(drawn_for_mind(Some("0"), None), "no value of it turns masking off");
    }

    #[test]
    fn a_display_that_is_not_the_persons_masks_without_the_marker() {
        assert!(drawn_for_mind(None, Some("wayland-1")));
        assert!(drawn_for_mind(Some(""), Some("wayland-mind")));
    }

    #[test]
    fn the_persons_own_display_and_no_display_do_not_mask() {
        assert!(!drawn_for_mind(None, Some(PERSON_DISPLAY)));
        assert!(!drawn_for_mind(None, None));
        assert!(!drawn_for_mind(Some(""), None));
    }

    #[test]
    fn only_a_minds_window_is_masked_unless_the_whole_copy_is_on_a_minds_display() {
        let person = crate::WindowId::PRIMARY;
        let mind = crate::WindowId(crate::WindowId::MIND | 7);
        assert!(!masked(false, person), "the person's window, beside a Mind's");
        assert!(masked(false, mind));
        assert!(masked(true, person), "a copy drawn on a Mind's display masks all it draws");
        assert!(mind.for_mind() && !person.for_mind());
    }

    #[test]
    fn a_window_on_another_display_shows_the_one_sentence_and_refuses_edits() {
        // The display rule alone decided: no marker, a display that is not the person's.
        let for_mind = drawn_for_mind(None, Some("wayland-mind"));
        assert_eq!(shown(for_mind, "secret"), HIDDEN);
        assert_eq!(shown_if_any(for_mind, ""), "", "nothing to hide, nothing said");
        assert_eq!(refuse_if(for_mind, true).unwrap_err().sentence, READ_ONLY);
        assert!(refuse_if(for_mind, false).is_ok());
        assert_eq!(shown(false, "secret"), "secret");
        assert!(refuse_if(false, true).is_ok());
        assert_eq!(shown_notice(true, READ_ONLY), READ_ONLY, "the refusal is said as it is");
        assert_eq!(shown_notice(true, "Sent to ana@example.org"), HIDDEN);
        assert_eq!(shown_notice(false, "Sent to ana@example.org"), "Sent to ana@example.org");
    }
}
