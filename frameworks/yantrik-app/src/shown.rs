//! Whether an app's window is on screen, learnt from the frames it draws.
//!
//! An app that reads something on a timer (System Monitor samples the machine every two seconds)
//! has no reason to keep reading while nobody can see the window, and the desktop does not say
//! when that is. On Wayland a client is never told it was minimised or covered: winit 0.30 raises
//! no `Occluded` there, and the shell's Show desktop is the same minimise. What a window does
//! learn is that it was not let draw: winit holds each redraw until the compositor's frame
//! callback, and labwc sends none to a window it is not showing. So a change put on screen that
//! no frame has drawn by the next look means the window is off screen, and the first frame after
//! means it is back (`Effects::frames`, counted by the framework from winit's `RedrawRequested`).
//!
//! [`Shown`] keeps that one fact in the app's model:
//!
//! ```ignore
//! Command::Poll => {
//!     if !m.shown.on_screen(fx.frames()) {
//!         return Ok(()); // nobody can see a new reading: do not take one
//!     }
//!     ...
//! }
//! Command::Sampled { .. } => {
//!     if what_is_drawn_changed {
//!         m.shown.owe(fx.frames());
//!     }
//! }
//! ```
//!
//! Only a change that draws may be owed: a command that put nothing new on screen asks for no
//! frame, and a window that drew nothing because nothing changed is not off screen. A change to
//! something scrolled out of view draws nothing either; the window then counts as off screen
//! until its next frame, which any change in view, a hover or a scroll draws. Where the
//! compositor lets a window draw whether or not it is shown (X11), every look finds it on screen
//! and nothing changes.

/// What an app last put on screen, and whether a frame has drawn it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shown {
    /// The frame count when the last change was put on screen, while no frame has drawn it.
    owed: Option<u64>,
}

impl Shown {
    /// This command put a change on screen (`frames`: its [`Effects::frames`]): the next frame
    /// draws it. Before the first frame nothing is known about the window, so nothing is owed.
    ///
    /// [`Effects::frames`]: crate::effects::Effects::frames
    pub fn owe(&mut self, frames: u64) {
        if frames > 0 {
            self.owed = Some(frames);
        }
    }

    /// Whether the window is on screen, as of this command's `frames`: `false` only while the
    /// last change owed has not been drawn by any frame. A frame since then settles it.
    pub fn on_screen(&mut self, frames: u64) -> bool {
        match self.owed {
            Some(at) if frames == at => false,
            _ => {
                self.owed = None;
                true
            }
        }
    }

    /// The person brought the window back (`Event::Activated`, `Event::Presented`): what was
    /// owed is forgotten, so the next look counts the window on screen before its frame lands.
    pub fn back(&mut self) {
        self.owed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_no_frame_drew_by_the_next_look_is_a_window_off_screen() {
        let mut shown = Shown::default();
        shown.owe(4);
        assert!(!shown.on_screen(4), "nothing drawn since the change");
        assert!(!shown.on_screen(4), "and still nothing, look after look");
        assert!(shown.on_screen(5), "a frame drew it: on screen again");
        assert!(shown.on_screen(5), "and nothing is owed until the next change");
    }

    #[test]
    fn a_change_drawn_before_the_next_look_is_a_window_on_screen() {
        let mut shown = Shown::default();
        shown.owe(4);
        assert!(shown.on_screen(6));
        shown.owe(6);
        assert!(shown.on_screen(7));
    }

    #[test]
    fn the_latest_change_is_what_is_owed() {
        let mut shown = Shown::default();
        shown.owe(4);
        shown.owe(9);
        assert!(!shown.on_screen(9), "a frame drew the first change, none the second");
    }

    #[test]
    fn before_the_first_frame_nothing_is_known_and_nothing_is_owed() {
        // A window not yet drawn, or a test that never draws: every look is on screen.
        let mut shown = Shown::default();
        shown.owe(0);
        assert!(shown.on_screen(0));
    }

    #[test]
    fn a_window_brought_back_is_on_screen_before_its_frame_lands() {
        let mut shown = Shown::default();
        shown.owe(4);
        shown.back();
        assert!(shown.on_screen(4));
    }
}
