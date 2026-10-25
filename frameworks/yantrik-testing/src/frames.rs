//! What an app costs with nobody touching it: the frames its windows ask for, the timers that
//! fire and the jobs they start, over a span of the virtual clock (the kit's rule: nothing
//! redraws at rest).
//!
//! A window asks to be drawn when a property one of its frames read has changed: a model set to
//! a new `ModelRc` with the same rows in it, a string set to a new value, an animation in flight.
//! None of those shows in a screenshot or in a functional test; each is a full repaint on the
//! software rasteriser of a machine with no GPU. So the check is the one the display makes: draw
//! the window whenever it asks, and count how often that was.
//!
//! Slint's own timers and animations keep real time, not the virtual clock; an animation that
//! never ends asks for a frame at every step, and is counted the same way. A finite one (a bar
//! easing to a new width) is drawn once by [`TestApp::at_rest`], at the end of its step, and to
//! its end by [`TestApp::on_screen`], which draws as a display does: a frame every
//! [`DISPLAY_FRAME`] of Slint's time while the window keeps asking.
//!
//! Every frame drawn here is told to the core (`Core::frame_drawn`), as the runner tells it of
//! each frame the compositor lets a window draw, so an app that asks whether its window is on
//! screen (`yantrik_app::shown`) hears what it would on the desktop. [`TestApp::hidden`] is the
//! window minimised: the clock moves and nothing is drawn.

use std::time::{Duration, Instant};

use slint::ComponentHandle as _;
use yantrik_app::app::App;

use crate::display_clock::{self, Hold};
use crate::{headless, TestApp};

/// How long a finite animation already in flight is given to end before a span is measured.
const SETTLE_FOR: Duration = Duration::from_secs(2);

/// The most jobs one step of the clock may start, one after another, before the span is called
/// a loop: a job whose result starts the next at once is never at rest.
const JOBS_PER_STEP: usize = 64;

/// A 60 Hz display's frame: how often an animation in flight is drawn by [`TestApp::on_screen`].
const DISPLAY_FRAME: Duration = Duration::from_millis(16);

/// What happened over a span of the virtual clock with no one touching the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AtRest {
    /// Frames the app's windows asked for and were drawn.
    pub frames: usize,
    /// Timers that fired, the app's and the framework's.
    pub fired: usize,
    /// Jobs that ran: what the timers started (a poll, a re-read), run as the runner's pool
    /// would, with what they returned applied.
    pub jobs: usize,
}

impl<A: App> TestApp<A> {
    /// Shows every open window at `width` × `height` physical pixels, as a person's window would
    /// be: the layout that decides what is on screen (a rail beside the content, or not) is the
    /// one a frame then reads.
    pub fn resize(&self, width: u32, height: u32) {
        self.with(move |u| {
            for id in u.core.windows_open() {
                if let Some(ui) = u.core.window(id) {
                    let _ = ui.show();
                    ui.window().set_size(slint::PhysicalSize::new(width, height));
                }
            }
        });
    }

    /// Draws each open window that asked to be drawn, after Slint's timers and animations have
    /// moved on: how many drew.
    pub fn draw(&self) -> usize {
        self.with(|u| {
            slint::platform::update_timers_and_animations();
            let drawn = u
                .core
                .windows_open()
                .into_iter()
                .filter_map(|id| u.core.window(id))
                .filter(|ui| headless::draw_if_needed(ui.window()))
                .count();
            for _ in 0..drawn {
                u.core.frame_drawn();
            }
            drawn
        })
    }

    /// Every frame the windows ask for until they ask for none, drawn as a 60 Hz display would:
    /// the one a change asks for, and one each [`DISPLAY_FRAME`] of Slint's time while an
    /// animation it started runs. Slint's clock is held meanwhile and stepped a display frame at
    /// a time, so the count is the display's however long each frame takes to render here.
    /// Bounded by [`SETTLE_FOR`] of that time: an animation that never ends is counted to there.
    pub fn draw_until_still(&self) -> usize {
        self.draw_on(&display_clock::hold())
    }

    fn draw_on(&self, display: &Hold) -> usize {
        let mut frames = 0;
        let mut shown_for = Duration::ZERO;
        loop {
            let drawn = self.draw();
            frames += drawn;
            if drawn == 0 || shown_for >= SETTLE_FOR {
                return frames;
            }
            display.step(DISPLAY_FRAME);
            shown_for += DISPLAY_FRAME;
        }
    }

    /// Everything owed drawn, and an animation already in flight (a theme's fade, a popover
    /// opening) given [`SETTLE_FOR`] of real time to end: `true` once a window asks for nothing.
    pub fn settle_frames(&self) -> bool {
        let until = Instant::now() + SETTLE_FOR;
        while Instant::now() < until {
            if self.draw() == 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        false
    }

    /// Moves the virtual clock over `span` in `step`s with nobody touching the app; after each
    /// step the jobs its timers started run and land, and whatever asks to be drawn is drawn:
    /// what that cost. What was owed before the span (the first frame, a reading that just
    /// landed) is drawn first and not counted.
    pub fn at_rest(&self, span: Duration, step: Duration) -> AtRest {
        self.settle_frames();
        self.measure(span, step, |app, _| app.draw())
    }

    /// [`TestApp::at_rest`] with every frame a display would draw counted: after each step, what
    /// the window asks for is drawn until it asks for nothing ([`TestApp::draw_until_still`]), so
    /// a change that starts a 300 ms animation costs the nineteen or so frames it does on screen,
    /// not one. What a window costs in front of a person while its numbers move.
    ///
    /// Slint's clock is held over the whole span and moves with it, by each step and each
    /// display frame, so a reading that lands is first drawn at the time it landed: after a
    /// step's frames the clock moves on to the next step, and Slint is told before anything
    /// lands in it (an animation starts at the time Slint last heard).
    pub fn on_screen(&self, span: Duration, step: Duration) -> AtRest {
        self.settle_frames();
        let display = display_clock::hold();
        self.measure(span, step, |app, by| {
            let frames = app.draw_on(&display);
            display.step(by);
            app.with(|_| slint::platform::update_timers_and_animations());
            frames
        })
    }

    /// The windows off screen (minimised, or covered), over `span`: the clock moves and the jobs
    /// its timers start run and land, and nothing is drawn, because a compositor lets a window it
    /// is not showing draw no frame. What the app does while nobody can see it. `frames` is 0.
    pub fn hidden(&self, span: Duration, step: Duration) -> AtRest {
        self.settle_frames();
        self.measure(span, step, |_, _| 0)
    }

    /// The span in steps: after each, the jobs its timers started run and land, then `draw`
    /// draws, told how far the clock moved.
    fn measure(
        &self,
        span: Duration,
        step: Duration,
        draw: impl Fn(&Self, Duration) -> usize,
    ) -> AtRest {
        assert!(!step.is_zero(), "a step of the clock moves it");
        let mut rest = AtRest::default();
        let mut moved = Duration::ZERO;
        while moved < span {
            let by = step.min(span - moved);
            rest.fired += self.advance_counting(by);
            let mut ran = 0;
            while self.run_one_job() {
                ran += 1;
                assert!(ran <= JOBS_PER_STEP, "{} jobs in one step at rest: one starts the next", ran);
            }
            rest.jobs += ran;
            rest.frames += draw(self, by);
            moved += by;
        }
        rest
    }
}

