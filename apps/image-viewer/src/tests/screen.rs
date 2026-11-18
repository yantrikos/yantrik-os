//! What the screen's markup must keep saying, read from its text: a test of the app cannot see
//! these through the window, because they are how the window decides.

const SCREEN: &str = include_str!("../../ui/image_viewer.slint");
const WINDOW: &str = include_str!("../../ui/app.slint");
const VIEW: &str = include_str!("../view.rs");

/// With no picture the window's view controls are off and say why; the control surface refuses
/// the same actions with the same words (tested in `opening.rs`).
#[test]
fn the_window_and_the_control_surface_give_the_same_reason() {
    assert!(
        SCREEN.contains(&format!("property <string> no-picture: \"{}\";", crate::NO_PICTURE)),
        "the window and the control surface must give the same reason"
    );
    assert_eq!(
        SCREEN.matches("enabled: root.has-picture; off-reason: root.no-picture;").count(),
        9,
        "every view control"
    );
}

/// The defect: both flip buttons lit up and left the picture as it was. The screen set the
/// properties itself and drew only the rotation. The pixels now arrive turned; a transform on
/// top would turn them twice, and a property set by the screen would be out of step with them.
#[test]
fn the_screen_draws_the_pixels_it_is_given_and_never_turns_them_itself() {
    assert!(!SCREEN.contains("transform-rotation"), "the screen must not turn the picture again");
    for prop in ["root.viewer-rotation =", "root.viewer-flip-h =", "root.viewer-flip-v ="] {
        assert!(!SCREEN.contains(prop), "the screen sets {prop} itself, out of step with the pixels");
    }
}

/// Nothing drawn does nothing: every callback the window declares is bound in `view::wire`.
#[test]
fn every_callback_the_window_declares_is_wired() {
    let declared: Vec<&str> = WINDOW
        .lines()
        .filter_map(|l| l.trim().strip_prefix("callback "))
        .map(|rest| rest.split(|c: char| c == ' ' || c == '(' || c == ';').next().unwrap_or_default())
        .collect();
    assert_eq!(declared.len(), 13, "read {declared:?} from ui/app.slint");
    for name in declared {
        let wired = format!("on_{}(", name.replace('-', "_"));
        assert!(VIEW.contains(&wired), "`{name}` is declared and never wired ({wired})");
    }
}

/// The view's own state reaches the app however it changed, so `describe` reads what is shown.
#[test]
fn every_change_of_the_view_is_told_to_the_app() {
    for prop in ["zoomed", "fit-contain", "viewer-info-open", "viewer-slideshow-active", "viewer-slideshow-paused"] {
        assert!(
            WINDOW.contains(&format!("changed {prop} => {{ root.view-changed(); }}")),
            "a change of `{prop}` is not told to the app"
        );
    }
}
