//! What the app's windows have drawn, and the effects a command is given, which carry it. Part
//! of [`Core`]; here so the core's own file stays readable.
//!
//! A frame is counted when a window draws one: under the runner, at each winit
//! `RedrawRequested` ([`Core::hear_window`]); under yantrik-testing, at each frame its headless
//! platform draws. Every command's [`Effects::frames`] is the count when it ran, which is how an
//! app learns that what it last put on screen has not been drawn: its window is not on screen
//! (`crate::shown`). Counting is all the framework does with it.

use super::Core;
use crate::app::App;
use crate::effects::Effects;
use crate::intents::Invocation;

impl<A: App> Core<A> {
    /// A window of this app drew a frame. The runner's winit hook says it; a platform that draws
    /// the windows itself (yantrik-testing's headless one) says it after each frame it draws.
    pub fn frame_drawn(&self) {
        let mut windows = self.windows.borrow_mut();
        windows.frames = windows.frames.saturating_add(1);
    }

    /// Frames drawn by every window of this app since it started.
    pub fn frames_drawn(&self) -> u64 {
        self.windows.borrow().frames
    }

    /// The effects a command runs with: job ids continued, the invocation it runs under, the
    /// time on the app's clock and the frames drawn so far.
    pub(super) fn effects(&self, invocation: Option<Invocation>) -> Effects<A> {
        let now = self.timers.borrow().instant();
        Effects::new(self.next_job.get(), invocation, now).drawn(self.frames_drawn())
    }
}
