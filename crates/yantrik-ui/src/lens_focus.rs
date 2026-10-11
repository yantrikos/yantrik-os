//! Where the keyboard goes when the Lens's window opens and closes (#733).
//!
//! The Lens is a window of its own, beside the person's work, so opening it borrows the keyboard
//! from whatever they were using and closing it gives the keyboard back. Who that was comes from
//! the compositor's own focus stream (`toplevel_watch`), which never lists the Lens itself. Pure,
//! so every case is a test; wire/lens_window.rs does what these say.

use crate::toplevel_watch::FocusNow;

/// What opening the Lens does about the window the person was using.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Opening {
    /// The window to give the keyboard back to when the Lens closes, as (id, title).
    pub return_to: Option<(u64, String)>,
    /// A window to bring back in front before the Lens shows: the one a press on the shell's own
    /// dock or bar just put behind the fullscreen shell (labwc raises the window it is pressed
    /// in). Without it, opening the Lens from the dock still hid Notes behind the desktop.
    pub put_back: Option<u64>,
    /// The windows whose being in front at close means the person has not gone anywhere else:
    /// the one we return to, and the shell when the press that opened the Lens was on the shell.
    pub stayed: Vec<u64>,
}

/// Decide, as the Lens opens, whose keyboard it is borrowing. `shell` is the shell's own title.
///
/// The window in front, unless that is the shell and it took focus a moment ago: then the
/// press that opened the Lens was on the shell's dock or bar, and the person was using the
/// window before it.
pub fn on_open(seen: &FocusNow, shell: &str) -> Opening {
    let Some(current) = seen.current.clone() else { return Opening::default() };
    if current.1 == shell && seen.shell_just_now {
        if let Some(previous) = seen.previous.clone().filter(|(_, t)| t != shell) {
            return Opening { put_back: Some(previous.0), stayed: vec![previous.0, current.0], return_to: Some(previous) };
        }
    }
    Opening { put_back: None, stayed: vec![current.0], return_to: Some(current) }
}

/// Decide, as the Lens closes, which window takes the keyboard back: the one it was borrowed from,
/// unless the person has since gone to another window, which keeps it. `front_now` is the window
/// the focus stream has in front (the Lens is never it). `None`: leave the keyboard where it is.
pub fn on_close(opening: &Opening, front_now: Option<u64>) -> Option<u64> {
    let (id, _) = opening.return_to.as_ref()?;
    match front_now {
        Some(front) if !opening.stayed.contains(&front) => None,
        _ => Some(*id),
    }
}

/// What the shell is showing, before and after a press in the Lens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellShowing {
    pub screen: i32,
    /// One of the shell's own menus over its screen: the mind-mode menu or the Minds panel.
    pub menu: bool,
}

/// Whether a press in the Lens put something of the shell's own in front of the person — another
/// screen (open in Agents, History, a run, a setting) or one of its menus — which they can only
/// see if the shell's window comes forward. Until the shell's screens are windows of their own
/// (#733 phase 2), that is the one case the Lens still raises the shell for.
pub fn shows_the_shell(before: ShellShowing, after: ShellShowing) -> bool {
    before.screen != after.screen || (after.menu && !before.menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: &str = "Yantrik OS";

    fn seen(current: Option<(u64, &str)>, previous: Option<(u64, &str)>, shell_just_now: bool) -> FocusNow {
        FocusNow {
            current: current.map(|(i, t)| (i, t.to_string())),
            previous: previous.map(|(i, t)| (i, t.to_string())),
            shell_just_now,
        }
    }

    /// Super+K from inside Notes: the compositor never moved focus, so Notes is the window, and
    /// nothing has to be put back.
    #[test]
    fn opened_from_inside_an_app_it_returns_to_that_app() {
        let open = on_open(&seen(Some((7, "Notes")), Some((0, SHELL)), false), SHELL);
        assert_eq!(open.return_to, Some((7, "Notes".to_string())));
        assert_eq!(open.put_back, None);
        assert_eq!(on_close(&open, Some(7)), Some(7));
    }

    /// The dock's mind button pressed with Notes in front (the 520 check): the press raised the
    /// fullscreen shell over Notes a moment ago. Notes goes back in front before the Lens shows,
    /// and gets the keyboard back when it closes.
    #[test]
    fn opened_from_the_dock_it_puts_the_app_back_and_returns_to_it() {
        let open = on_open(&seen(Some((0, SHELL)), Some((7, "Notes")), true), SHELL);
        assert_eq!(open.put_back, Some(7));
        assert_eq!(open.return_to, Some((7, "Notes".to_string())));
        // The stream may still have the shell in front at close, or Notes after the put-back.
        assert_eq!(on_close(&open, Some(0)), Some(7));
        assert_eq!(on_close(&open, Some(7)), Some(7));
    }

    /// The person was on the desktop, and Notes is behind it from long ago: the desktop is what
    /// they were using, and nothing is brought over it.
    #[test]
    fn opened_on_the_desktop_it_returns_to_the_desktop() {
        let open = on_open(&seen(Some((0, SHELL)), Some((7, "Notes")), false), SHELL);
        assert_eq!(open.put_back, None);
        assert_eq!(open.return_to, Some((0, SHELL.to_string())));
        assert_eq!(on_close(&open, Some(0)), Some(0));
    }

    /// A press on the shell with nothing before it, or the shell before the shell (a retitle the
    /// log could not tell apart): nothing to put back.
    #[test]
    fn a_press_on_the_shell_with_no_app_before_it_puts_nothing_back() {
        assert_eq!(on_open(&seen(Some((0, SHELL)), None, true), SHELL).put_back, None);
        assert_eq!(on_open(&seen(Some((0, SHELL)), Some((3, SHELL)), true), SHELL).put_back, None);
    }

    /// While the Lens was open the person clicked into the Terminal and worked there. Closing the
    /// Lens must not take the keyboard from them and hand it to Notes.
    #[test]
    fn a_person_who_moved_on_keeps_the_window_they_moved_to() {
        let open = on_open(&seen(Some((7, "Notes")), None, false), SHELL);
        assert_eq!(on_close(&open, Some(9)), None);
    }

    /// The focus stream is not being followed: nothing is known, nothing is claimed, and the
    /// compositor's own choice on close stands.
    #[test]
    fn nothing_known_is_nothing_returned() {
        let open = on_open(&FocusNow::default(), SHELL);
        assert_eq!(open, Opening::default());
        assert_eq!(on_close(&open, None), None);
        // Opened knowing, closed not knowing: the window it was borrowed from.
        let open = on_open(&seen(Some((7, "Notes")), None, false), SHELL);
        assert_eq!(on_close(&open, None), Some(7));
    }

    #[test]
    fn a_press_that_changes_the_screen_or_opens_a_menu_shows_the_shell() {
        let desk = ShellShowing { screen: 1, menu: false };
        assert!(!shows_the_shell(desk, desk), "sending a message, allowing a card: the shell stays where it is");
        assert!(shows_the_shell(desk, ShellShowing { screen: 34, menu: false }), "open in Agents, History, a run");
        assert!(shows_the_shell(desk, ShellShowing { screen: 1, menu: true }), "the mode chip, the mind");
        let menu = ShellShowing { screen: 1, menu: true };
        assert!(!shows_the_shell(menu, desk), "putting a menu away shows nothing");
    }
}
