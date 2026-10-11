//! Show desktop (#241): the button at the far right of the taskbar, Super+D and the shell's
//! `show_desktop` action. One press puts every app window away and shows the desktop; the next
//! brings back exactly the windows it put away.
//!
//! Which windows go and which come back is decided from titles alone, apart from the compositor,
//! so the rule is tested without one. The first press remembers what it minimised together with
//! every window open at that moment, and the second press restores them only if that is still the
//! set of windows open. Anything opened or closed in between makes the next press a fresh "show
//! desktop", which is what a person who has moved on expects: Windows behaves the same way.

use std::cell::RefCell;

use slint::ComponentHandle;

use crate::App;

/// What the last "show desktop" put away, and every window that was open at that moment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shown {
    pub minimised: Vec<String>,
    pub open: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum Press {
    /// Put these windows away and show the desktop.
    Show(Vec<String>),
    /// Bring these windows back.
    Restore(Vec<String>),
}

/// What a press does, given the titles of the windows open now (the shell's own left out) and
/// what the last press put away.
pub fn decide(open_now: &[String], last: Option<&Shown>) -> Press {
    if let Some(last) = last {
        let (mut then, mut now) = (last.open.clone(), open_now.to_vec());
        then.sort();
        now.sort();
        if !last.minimised.is_empty() && then == now {
            return Press::Restore(last.minimised.clone());
        }
    }
    Press::Show(open_now.to_vec())
}

thread_local! {
    static LAST: RefCell<Option<Shown>> = const { RefCell::new(None) };
}

/// Press it: put the windows away and show the desktop, or bring them back. The answer says which,
/// and names any window the compositor would not move.
///
/// `may_restore` is asked before windows are brought back, and only then: bringing them back can
/// cover a waiting approval card, putting them away never can (card_watch). Refused, nothing
/// moves and what the first press put away is still remembered, so the press after the card is
/// answered brings it back.
pub fn press(ui: &App, may_restore: impl FnOnce() -> Result<(), String>) -> Result<serde_json::Value, String> {
    let open: Vec<String> = crate::windows::shell_windows()
        .into_iter()
        .map(|w| w.title)
        .filter(|t| !t.is_empty() && t != crate::windows::SHELL_WINDOW_TITLE)
        .collect();
    let last = LAST.with(|l| l.borrow().clone());
    match decide(&open, last.as_ref()) {
        Press::Show(titles) => {
            let (mut minimised, mut refused) = (Vec::new(), Vec::new());
            for title in titles {
                match crate::windows::minimise(&title) {
                    Ok(()) => minimised.push(title),
                    Err(why) => {
                        tracing::warn!(window = %title, %why, "Show desktop could not put a window away");
                        refused.push(title);
                    }
                }
            }
            ui.set_app_grid_open(false);
            if ui.get_current_screen() != 1 {
                ui.set_current_screen(1);
                ui.invoke_navigate(1);
            }
            // The desktop is part of the shell's own window; with nothing left in front of it,
            // it still has to be asked forward, or a window the compositor would not minimise
            // stays over it.
            let raised = crate::windows::raise_shell().is_ok();
            LAST.with(|l| *l.borrow_mut() = Some(Shown { minimised: minimised.clone(), open }));
            Ok(serde_json::json!({
                "did": "show_desktop",
                "minimised": minimised,
                "could_not_minimise": refused,
                "raised": raised,
            }))
        }
        Press::Restore(titles) => {
            may_restore()?;
            let restored: Vec<String> = titles.into_iter().filter(|t| crate::windows::present(t)).collect();
            LAST.with(|l| *l.borrow_mut() = None);
            Ok(serde_json::json!({"did": "restore_windows", "restored": restored}))
        }
    }
}

pub fn wire(ui: &App) {
    let weak = ui.as_weak();
    ui.on_show_desktop(move || {
        if let Some(ui) = weak.upgrade() {
            // The taskbar's corner button is the person's own hand on this machine, not a call
            // on the surface, so nothing holds it.
            match press(&ui, || Ok(())) {
                Ok(outcome) => tracing::info!(%outcome, "Show desktop pressed"),
                Err(why) => tracing::warn!(%why, "Show desktop pressed and refused"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_first_press_puts_every_window_away() {
        let open = titles(&["Editor", "Terminal", "Images"]);
        assert_eq!(decide(&open, None), Press::Show(open.clone()));
    }

    #[test]
    fn the_second_press_brings_back_exactly_what_the_first_put_away() {
        let open = titles(&["Editor", "Terminal", "Images"]);
        let last = Shown { minimised: titles(&["Editor", "Terminal", "Images"]), open: open.clone() };
        // The same windows, listed in another order, are the same windows.
        let now = titles(&["Images", "Editor", "Terminal"]);
        assert_eq!(decide(&now, Some(&last)), Press::Restore(titles(&["Editor", "Terminal", "Images"])));
    }

    #[test]
    fn a_window_opened_in_between_makes_the_next_press_show_the_desktop_again() {
        let last = Shown { minimised: titles(&["Editor"]), open: titles(&["Editor"]) };
        let now = titles(&["Editor", "Notes"]);
        assert_eq!(decide(&now, Some(&last)), Press::Show(now.clone()));
    }

    #[test]
    fn a_window_closed_in_between_does_too() {
        let last = Shown { minimised: titles(&["Editor", "Terminal"]), open: titles(&["Editor", "Terminal"]) };
        let now = titles(&["Editor"]);
        assert_eq!(decide(&now, Some(&last)), Press::Show(now.clone()));
    }

    #[test]
    fn nothing_put_away_is_nothing_to_bring_back() {
        // A desktop with no windows: the press shows the desktop, and so does the next one.
        let last = Shown { minimised: vec![], open: vec![] };
        assert_eq!(decide(&[], Some(&last)), Press::Show(vec![]));
    }

    /// Final review of the card fix: the second press brought every window back over a waiting
    /// card. Only bringing back is asked about, never putting away, and it is asked before
    /// anything is restored or forgotten.
    #[test]
    fn only_bringing_windows_back_asks_first_and_a_refusal_forgets_nothing() {
        let source = include_str!("show_desktop.rs");
        let start = source.find(concat!("Press::", "Restore(titles) =>")).expect("the restore branch");
        let branch = &source[start..];
        let asked = branch.find(concat!("may_", "restore()?")).expect("the restore branch asks");
        assert!(asked < branch.find(concat!("windows::", "present(")).unwrap(), "asked before anything moves");
        assert!(asked < branch.find(concat!("borrow_mut() = ", "None")).unwrap(), "and before the memory goes");
        let show = source.find(concat!("Press::", "Show(titles) =>")).expect("the show branch");
        assert!(!source[show..start].contains(concat!("may_", "restore")), "putting away is never held");
    }

    /// The desktop is part of the shell's own window, which a Wayland client cannot raise itself.
    #[test]
    fn showing_the_desktop_brings_the_shell_forward() {
        let source = include_str!("show_desktop.rs");
        let start = source.find(concat!("pub fn ", "press(")).expect("press");
        let end = start + source[start..].find("\npub fn wire").expect("its end");
        assert!(source[start..end].contains(concat!("crate::windows::raise_", "shell()")));
    }
}
