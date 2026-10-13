//! Approval precedence, the shell's half: with a card waiting, a window taking focus over the
//! shell brings it back in front, up to [`MAX_RAISES`] times, and a mind cannot make a window
//! fullscreen or always on top through the control surface at all.
//!
//! [`decide`] is given the focused window's title and nothing else: not its size, its layer, or
//! whether it is maximised, fullscreen or always on top. So the replay below cannot tell those
//! states apart and does not claim to; it shows that no window state is an input, and so none is
//! a way round the raise. Whether labwc then paints the focused shell over a fullscreen window,
//! or over one in its always-on-top layer (rc.xml's `ToggleAlwaysOnTop` window-menu row), is not
//! reachable from a unit test and is the live check named in the PR.
//!
//! The cap is real and pinned here: after [`MAX_RAISES`] (5) raises over windows that are not
//! Mind View, a sixth window taking focus is left in front and covers the card. From then on the
//! taskbar's Chat button, amber with a count, is what says a card waits.
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

/// The titles only name the windows: [`decide`] never sees their state, so these are three
/// focus changes, answered alike.
#[test]
fn focus_taken_by_any_window_brings_the_shell_back_since_its_state_is_never_read() {
    let raised = replay(&[(0, "Notes"), (5_000, "Video"), (10_000, "Clock")]);
    assert_eq!(raised, ["Notes", "Video", "Clock"]);
}

/// The cap: five raises over other windows, then the sixth focus change is left in front of the
/// card. Mind View still brings it back, since a mind's own desktop never uses up the raises.
#[test]
fn a_sixth_window_taking_focus_is_left_over_the_card_and_mind_view_never_is() {
    let titles: Vec<String> = (1..=MAX_RAISES + 1).map(|n| format!("Window {n}")).collect();
    let mut events: Vec<(u64, &str)> = titles.iter().enumerate().map(|(i, t)| (i as u64 * 5_000, t.as_str())).collect();
    let after = events.len() as u64 * 5_000;
    events.push((after, MIND_VIEW_TITLE));
    let raised = replay(&events);
    assert_eq!(MAX_RAISES, 5, "the evidence states a cap of five; change it there too");
    assert_eq!(raised.len(), MAX_RAISES as usize + 1, "{raised:?}");
    assert_eq!(raised[..MAX_RAISES as usize], titles[..MAX_RAISES as usize]);
    assert!(!raised.contains(&titles[MAX_RAISES as usize]), "the sixth window is raised over, past the cap");
    assert_eq!(raised.last().map(String::as_str), Some(MIND_VIEW_TITLE));
}

/// The shell taking the front back is not itself something to answer.
#[test]
fn the_shell_coming_back_is_left_alone() {
    assert!(replay(&[(0, SHELL_WINDOW_TITLE)]).is_empty());
}

/// No action the shell publishes makes a window fullscreen or always on top. The actions that
/// maximise, bring forward or open a window are held while a card waits; which ones is pinned by
/// `every_window_moving_action_asks_hold_windows_first` in card_watch.rs, since `hold_windows`
/// itself answers the same whatever action name it is given.
#[test]
fn the_shell_publishes_no_action_that_makes_a_window_fullscreen_or_always_on_top() {
    let published = crate::control::locked_state_tests::published_actions();
    for word in ["fullscreen", "always_on_top", "keep_above", "pin_window"] {
        assert!(
            !published.iter().any(|a| a.contains(word)),
            "the shell publishes an action with {word:?} in its name; it must call hold_windows and be listed in every_window_moving_action_asks_hold_windows_first"
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
