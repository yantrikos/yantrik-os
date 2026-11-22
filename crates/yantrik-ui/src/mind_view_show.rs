//! How Mind View first appears on the person's desktop (#1347).
//!
//! Mind View is the desk where the person watches a Mind work, so it is shown when a Mind starts
//! working. #427's reason still stands: it must never take the person's keyboard or cover what
//! they are typing in. So the window is placed small, in the bottom-right corner above the dock, by
//! a window rule in the person's compositor (`config/labwc/rc.xml`, the rule titled "Mind View"),
//! and whoever held the keyboard when it mapped gets it straight back, as the on-screen display
//! does (`wire::osd`).
//!
//! labwc 0.8.3, the version on the image (Debian trixie), has no window-rule property that keeps a
//! window from being focused as it maps: `ignoreFocusRequest` stops a window *asking* to be
//! activated (labwc-config(5)), and a window rule's actions are the placement actions only. So the
//! keyboard is handed back after the map, from the window read just before it.
//!
//! Only the first map is acted on. A window the person minimised stays minimised, and one they
//! closed is not put back in front by a later turn: [`note_gone`] is told when the person's close
//! (or its death) was noticed, and the next Mind View then starts out of the way, as it did
//! before this change, until the person asks for it (the bar's "working" chip, Super+Shift+V or
//! the taskbar).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Mind View's window, as the person's compositor names it: labwc's app id, and the title the
/// title library gives it (crates/yantrik-mind-view-title).
const WINDOW_MATCH: [&str; 2] = ["app_id:labwc", "title:Mind View"];

/// How long the keyboard is left with the new window before it is handed back: long enough for
/// the compositor to have focused it, the same wait the on-screen display uses.
const REFOCUS_AFTER: Duration = Duration::from_millis(150);

/// Whether the person closed Mind View, or it died, since the shell started.
static GONE: AtomicBool = AtomicBool::new(false);

/// Where Mind View lands when it maps.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    /// In the corner, shown, with the keyboard given back.
    Corner,
    /// Minimised: the person closed the last one, and a later turn does not reopen it.
    Aside,
}

fn landing(gone_before: bool) -> Landing {
    if gone_before {
        Landing::Aside
    } else {
        Landing::Corner
    }
}

/// The running Mind View is gone: the person closed it or it died. The next one starts aside.
pub fn note_gone() {
    GONE.store(true, Ordering::Relaxed);
}

/// Place Mind View's window as it first appears. `restore` is the window that held the keyboard
/// before it was started (`windows::focus_to_restore`). Off the UI thread: it waits for the window,
/// and wlrctl is a process.
pub fn when_mapped(restore: Option<String>, wait: Duration) {
    let landing = landing(GONE.load(Ordering::Relaxed));
    std::thread::spawn(move || {
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if wlrctl(&["find"]) {
                match landing {
                    Landing::Aside => {
                        if !wlrctl(&["minimize"]) {
                            tracing::warn!("Mind View appeared but could not be set aside; it may be over the person's work");
                        }
                    }
                    Landing::Corner => {
                        std::thread::sleep(REFOCUS_AFTER);
                        if let Some(title) = restore {
                            crate::windows::present(&title);
                        }
                    }
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        tracing::debug!("Mind View's window did not appear in time to be placed");
    });
}

fn wlrctl(verb: &[&str]) -> bool {
    Command::new("wlrctl")
        .arg("toplevel")
        .args(verb)
        .args(WINDOW_MATCH)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Bring Mind View forward and give it the keyboard: the person pressed the bar's chip, which is
/// their own act. Restores it first if it was minimised. A process: off the UI thread.
pub fn bring_forward() {
    std::thread::spawn(|| {
        if !crate::windows::present_app(crate::mind_view::APP_ID) {
            tracing::debug!("Mind View has no window to bring forward");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_mind_view_is_shown_and_one_the_person_closed_is_not_reopened() {
        assert_eq!(landing(false), Landing::Corner);
        assert_eq!(landing(true), Landing::Aside);
        note_gone();
        assert_eq!(landing(GONE.load(Ordering::Relaxed)), Landing::Aside);
        GONE.store(false, Ordering::Relaxed);
    }

    #[test]
    fn the_window_is_matched_by_the_names_the_rule_and_the_title_library_use() {
        assert!(WINDOW_MATCH.contains(&"title:Mind View"));
        assert_eq!(crate::mind_view::TITLE, "Mind View");
    }

    /// The rule that puts the window in the bottom-right corner: titled as the window is, placed
    /// by labwc's own actions (labwc-actions(5)), and not asking for the person's keyboard to be
    /// held away from them by any property labwc 0.8.3 does not have.
    #[test]
    fn the_desktop_places_mind_view_in_the_bottom_right_corner_above_the_dock() {
        let rc = include_str!("../../../config/labwc/rc.xml");
        let start = rc.find(r#"<windowRule title="Mind View""#).expect("a window rule for Mind View");
        let rule = &rc[start..start + rc[start..].find("</windowRule>").unwrap()];
        let edges: Vec<&str> = rule.match_indices("MoveToEdge").map(|(i, _)| &rule[i..i + 80]).collect();
        assert!(edges.iter().any(|e| e.contains(r#"direction="right""#)), "to the right edge");
        assert!(edges.iter().any(|e| e.contains(r#"direction="down""#)), "onto the dock's top");
        assert!(rule.contains("MoveRelative"), "a gap from the screen's edge");
        assert!(!rule.contains("Iconify"), "shown, not minimised");
        assert!(!rule.contains("ToggleAlwaysOnTop"), "it is not to cover the window being typed in");
    }

    #[test]
    fn the_keyboard_goes_back_to_who_had_it() {
        let src = include_str!("mind_view_show.rs");
        assert!(src.contains("crate::windows::present(&title)"));
        let mv = include_str!("mind_view.rs");
        assert!(mv.contains("focus_to_restore()"), "read before the window maps");
        assert!(mv.contains("mind_view_show::when_mapped"));
        assert!(mv.contains("mind_view_show::note_gone()"), "a window the person closed stays closed");
    }
}
