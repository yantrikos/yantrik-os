//! Approval precedence, the shell's half: with a card waiting, a maximised window, a fullscreen
//! window and an always-on-top window each taking focus over the shell bring it back in front,
//! and none of them can be made by a mind while the card waits. The decision is made on which
//! window took focus, never on that window's size or layer, so no window state is a way round it.
//!
//! What labwc then draws is not reachable from a unit test: whether a focused shell is painted
//! over a fullscreen window, and over a window in labwc's always-on-top layer (rc.xml's
//! `ToggleAlwaysOnTop` window-menu row), is the live check named in the PR.
use super::*;

/// Replays focus changes against [`decide`] on a clock of its own, the way `front_changed` and
/// its recheck use it, and answers which titles the shell came back in front of.
fn replay(events: &[(u64, &str)]) -> Vec<String> {
    let (mut raises, mut last, mut raised) = (0u32, None::<u64>, Vec::new());
    let mut queue: Vec<(u64, String)> = events.iter().map(|(t, w)| (*t, w.to_string())).collect();
    while !queue.is_empty() {
        queue.sort_by_key(|(t, _)| *t);
        let (now, title) = queue.remove(0);
        let since = last.map(|l| Duration::from_millis(now - l));
        match decide(true, &title, 1, since, raises) {
            Decision::Leave => {}
            Decision::Raise => {
                if title != MIND_VIEW_TITLE {
                    raises += 1;
                }
                last = Some(now);
                raised.push(title);
            }
            // The recheck looks at whoever is in front when the gap ends; here that is still it.
            Decision::Later(wait) => queue.push((now + wait.as_millis() as u64, title)),
        }
    }
    raised
}

#[test]
fn a_maximised_a_fullscreen_and_an_always_on_top_window_each_lose_the_front_to_a_waiting_card() {
    let raised = replay(&[(0, "Notes (maximised)"), (5_000, "Video (fullscreen)"), (10_000, "Clock (always on top)")]);
    assert_eq!(raised, ["Notes (maximised)", "Video (fullscreen)", "Clock (always on top)"]);
}

/// The shell taking the front back is not itself something to answer.
#[test]
fn the_shell_coming_back_is_left_alone() {
    assert!(replay(&[(0, SHELL_WINDOW_TITLE)]).is_empty());
}

/// None of the three states can be made through the control surface while a card waits: the
/// actions that maximise, bring forward or open a window are held, and the shell publishes no
/// action that makes a window fullscreen or always on top at all.
#[test]
fn a_mind_cannot_maximise_or_raise_a_window_over_a_waiting_card() {
    let _turn = super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    set_waiting(["precedence-card"]);
    for action in ["maximise_window", "focus_window", "show_app", "open_app"] {
        assert!(hold_windows(action).is_err(), "{action} runs while a card waits");
    }
    set_waiting([]);
    *RAISED_AT.lock().unwrap() = None;
    let published = crate::control::locked_state_tests::published_actions();
    for word in ["fullscreen", "always_on_top", "keep_above", "pin_window"] {
        assert!(
            !published.iter().any(|a| a.contains(word)),
            "the shell publishes an action with {word:?} in its name; it must call hold_windows and be listed here"
        );
    }
}

/// The raise is a request to labwc to focus the window titled exactly as the shell, after
/// restoring it if it was minimised: by title, so it cannot bring forward a window that only
/// shares an app id.
#[test]
fn the_raise_asks_labwc_to_restore_and_focus_the_shell_by_its_exact_title() {
    let src = include_str!("windows.rs");
    let at = src.find("pub fn raise_shell()").expect("windows::raise_shell");
    let body = &src[at..at + src[at..].find("\n}").unwrap()];
    let restore = body.find("restore_args(SHELL_WINDOW_TITLE)").expect("the shell is restored first");
    let focus = body.find("toplevel_args(\"focus\", SHELL_WINDOW_TITLE)").expect("then focused by title");
    assert!(restore < focus);
}
