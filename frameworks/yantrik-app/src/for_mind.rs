//! Whether this copy of the app is drawn for a Mind: its window is in Mind View, where
//! `read_mind_view` turns every line on it into text for an agent.
//!
//! An app with content an agent must not read (a snippet's code, a note) masks it when
//! `Boot::for_mind` is set, and keeps what is safe to show: titles, tags, counts.
//!
//! It fails closed. A copy is drawn for a Mind when either says so:
//! - the launch carries the marker [`FOR_MIND_VAR`], which the shell sets on every app it starts
//!   on a mind's display; or
//! - its Wayland display is a Mind's by the shell's own record of displays
//!   (`yantrik_ipc_contracts::mind_view_displays`): a display Mind View lists, or any display but
//!   the one the shell recorded as the person's. There is no literal display name: a person on
//!   `wayland-1` is the person. A launch that lost the marker but landed on Mind View is still
//!   a Mind's.
//!
//! When the shell's record of the person's display is lost (missing, unreadable, not the shell's),
//! a copy is a Mind's only if its display is one Mind View lists, or the marker says so; see
//! `mind_view_displays` for the whole rule. An app's sentence must not say a Mind is looking when
//! only this rule decided it.
//!
//! The marker can only turn masking on. Nothing here reads a variable that turns it off.

use yantrik_ipc_contracts::mind_view_displays::Known;

pub use yantrik_ipc_contracts::mind_view::FOR_MIND_VAR;

/// The shell's record of the displays, from the session's socket directory.
pub fn known_displays() -> Known {
    Known::read(&yantrik_ipc_transport::server::socket_dir())
}

/// Whether a window with this launch is drawn for a Mind: `marker` is the value of
/// [`FOR_MIND_VAR`] and `display` that of `WAYLAND_DISPLAY`, both as the process has them.
///
/// Any value of the marker but an empty one counts, so `0` and `false` still mask. With no display
/// named the window is not on Wayland and the display rule has nothing to compare.
pub fn drawn_for_mind(marker: Option<&str>, display: Option<&str>) -> bool {
    drawn_for_mind_in(marker, display, &known_displays())
}

/// [`drawn_for_mind`] against the displays `known`.
pub fn drawn_for_mind_in(marker: Option<&str>, display: Option<&str>, known: &Known) -> bool {
    marker.is_some_and(|m| !m.is_empty()) || known.is_minds(display)
}

/// [`drawn_for_mind`], for this process.
pub fn from_env() -> bool {
    let marker = std::env::var(FOR_MIND_VAR).ok();
    let display = std::env::var("WAYLAND_DISPLAY").ok();
    drawn_for_mind(marker.as_deref(), display.as_deref())
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

    fn known(person: Option<&str>, minds: &[&str]) -> Known {
        Known { person: person.map(str::to_string), minds: minds.iter().map(|m| m.to_string()).collect() }
    }

    #[test]
    fn the_marker_turns_masking_on_whatever_the_display() {
        let k = known(Some("wayland-1"), &[]);
        assert!(drawn_for_mind_in(Some("1"), Some("wayland-1"), &k));
        assert!(drawn_for_mind_in(Some("1"), None, &k));
        assert!(drawn_for_mind_in(Some("0"), None, &k), "no value of it turns masking off");
    }

    #[test]
    fn a_display_that_is_not_the_persons_masks_without_the_marker() {
        let k = known(Some("wayland-0"), &[]);
        assert!(drawn_for_mind_in(None, Some("wayland-1"), &k));
        assert!(drawn_for_mind_in(Some(""), Some("wayland-mind"), &k));
    }

    /// A person on any display is the person: there is no literal `wayland-0`.
    #[test]
    fn the_persons_own_display_and_no_display_do_not_mask() {
        for person in ["wayland-0", "wayland-1", "wayland-5"] {
            let k = known(Some(person), &["wayland-9"]);
            assert!(!drawn_for_mind_in(None, Some(person), &k), "{person}");
            assert!(drawn_for_mind_in(None, Some("wayland-9"), &k), "a Mind on its seat");
        }
        let k = known(Some("wayland-1"), &[]);
        assert!(!drawn_for_mind_in(None, None, &k));
        assert!(!drawn_for_mind_in(Some(""), None, &k));
    }

    /// The shell's record is lost: only Mind View's own list, or the marker, makes a Mind's.
    #[test]
    fn with_the_record_lost_a_display_is_a_minds_only_if_mind_view_lists_it() {
        let lost = known(None, &["wayland-1"]);
        assert!(!drawn_for_mind_in(None, Some("wayland-0"), &lost), "absent from the seats");
        assert!(drawn_for_mind_in(None, Some("wayland-1"), &lost), "listed");
        assert!(drawn_for_mind_in(Some("1"), Some("wayland-0"), &lost), "the marker still says so");
    }

    #[test]
    fn a_window_on_another_display_shows_the_one_sentence_and_refuses_edits() {
        // The display rule alone decided: no marker, a display that is not the person's.
        let for_mind = drawn_for_mind_in(None, Some("wayland-mind"), &known(Some("wayland-0"), &[]));
        assert_eq!(shown(for_mind, "secret"), HIDDEN);
        assert_eq!(shown_if_any(for_mind, ""), "", "nothing to hide, nothing said");
        assert_eq!(refuse_if(for_mind, true).unwrap_err().sentence, READ_ONLY);
        assert!(refuse_if(for_mind, false).is_ok());
        assert_eq!(shown(false, "secret"), "secret");
        assert!(refuse_if(false, true).is_ok());
    }
}
