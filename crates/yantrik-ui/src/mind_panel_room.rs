//! Keeping app windows off the mind panel.
//!
//! The panel is drawn inside the shell's one fullscreen window, so labwc does not know it is
//! there, and an app window placed at a decent size covered part of it: the Calendar cut every
//! row to "t detected", "t memories" in the sweep of 17 October (design/ui-sweep-2026-10-17.md,
//! root cause 4).
//!
//! The two ways out were to shrink the compositor's work area by the panel while it is shown, or
//! to fold the panel to its edge tab whenever a window could overlap it. Neither works alone with
//! what exists. labwc's `<margin>` (config/labwc/rc.xml) is how the status bar and the dock are
//! already kept clear, but it is one fixed number read at start; the panel is 334px open and 44px
//! as a strip, and a 334px margin would take a quarter of a small screen from every app even with
//! the panel folded. And the shell cannot see where app windows are (`wlrctl` lists titles, not
//! geometry), so it cannot fold "when a window would overlap".
//!
//! What it can see is whether an app window is in front (`windows::shell_in_front`), and that is
//! the only time an app window can be over the shell at all: the shell is an ordinary toplevel,
//! raised over every app when the person clicks it. So, both halves together:
//! - `<margin right>` reserves the strip's width, so placed, maximized and tiled windows never
//!   cover the strip;
//! - while an app window is in front, the panel is drawn as that strip ([`app_window_in_front`]
//!   feeds `app-window-in-front` in app.slint), and the person's choice comes back the moment the
//!   desktop is in front again. The saved choice is never changed by this.

/// The strip's width, which labwc keeps clear: `<margin right>` in rc.xml and the strip's room
/// (`mind-panel-reserve`, `mind-panel-width`) in app.slint. A test holds the three together.
pub const STRIP_PX: u32 = 44;

/// Whether the panel is held to its strip: an app window is in front. When the compositor cannot
/// say (`None`), the panel keeps the person's choice rather than folding on a guess.
pub fn app_window_in_front(shell_in_front: Option<bool>) -> bool {
    shell_in_front == Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_folds_only_when_an_app_window_is_known_to_be_in_front() {
        assert!(app_window_in_front(Some(false)));
        assert!(!app_window_in_front(Some(true)), "the desktop in front: the person's choice");
        assert!(!app_window_in_front(None), "not known: no fold on a guess");
    }

    #[test]
    fn labwc_keeps_the_strip_clear_and_the_panel_folds_to_it() {
        let rc = include_str!("../../../config/labwc/rc.xml");
        let at = rc.find("<margin ").expect("rc.xml reserves the shell's strips");
        let margin = &rc[at..at + rc[at..].find("/>").unwrap()];
        let right: u32 = margin.split("right=\"").nth(1).unwrap().split('"').next().unwrap().parse().unwrap();
        assert_eq!(right, STRIP_PX, "<margin right> is the strip's width");

        let app = include_str!("../../yantrik-ui-slint/ui/app.slint");
        assert!(app.contains(&format!(": {STRIP_PX}px;")), "app.slint's strip is {STRIP_PX}px wide");
        let open = app.split("property <bool> mind-panel-open:").nth(1).unwrap().split(';').next().unwrap();
        assert!(open.contains("!root.app-window-in-front"), "an app window in front folds the panel: {open}");
    }
}
