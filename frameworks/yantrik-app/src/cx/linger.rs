//! An app that runs on after its last window closes, for a bounded while (`fx.linger`). Part of
//! [`Core`]; here so the core's own file stays readable.
//!
//! The last window closing ends the process (`ExitReason::LastWindowClosed`). An app with work
//! still in flight at that moment (Calendar's writes to calendar-service, #921) would never learn
//! how it went: the shutdown cancels the job, and whether the write landed is unknown. Hiding the
//! window instead leaves one the core counts as open and the person cannot see, and a second
//! launch shows it again without the app hearing of it. So the app closes the window as usual
//! and, in the same command, asks to linger:
//!
//! ```rust,ignore
//! Command::Close if m.writes_in_flight > 0 => {
//!     m.closing = true;
//!     fx.close(WindowId::PRIMARY);
//!     fx.linger(Duration::from_secs(10));
//! }
//! ```
//!
//! While it lingers:
//!
//! - No window is open and the process runs: commands, jobs, timers, the surface and the Mind's
//!   intents work as before. The app runs `fx.exit()` once its work has landed.
//! - A second launch (the dock, `present`, a launch handed over) opens the first window again,
//!   drawn whole from the model, and raises `Event::Presented`: the linger is over, the app runs
//!   on as before the close, and that event is where it cancels the exit it was waiting to make.
//!   A window the app opens itself (`fx.open_window`) ends the linger too.
//! - A logout is raised as ever (`Event::ExitRequested(Logout)`), and the signal deadline
//!   (`crate::quit::SIGNAL_DEADLINE`) still ends the app, whatever its backstop.
//! - The backstop, at most [`MAX_LINGER`], ends the app if nothing else has, and logs that it did.
//!
//! Only the command that closes the last open window may linger. Anywhere else the request is
//! ignored with a warning, and the last window closing ends the process as it always has.

use std::time::{Duration, Instant};

use crate::app::{App, ExitReason, WindowId};
use crate::timers::{Fired, LINGER};

use super::Core;

/// The longest a linger's backstop may be. Long enough for a service write that is slow but
/// alive; short enough that a process nobody can see is never left running for long.
pub const MAX_LINGER: Duration = Duration::from_secs(60);

/// A linger under way: since when, and its backstop.
#[derive(Debug, Clone, Copy)]
pub(super) struct Lingering {
    since: Instant,
    backstop: Duration,
}

impl<A: App> Core<A> {
    /// Whether the process runs on with no window (`fx.linger`), and nothing has ended it yet.
    pub fn lingering(&self) -> bool {
        self.linger.get().is_some() && self.exit.get().is_none()
    }

    /// What one of the core's timers fired: the app's command, applied as any other; or the
    /// framework's own deadline. The timers call this; so does `TestApp::advance`.
    pub fn fired(&self, fired: Fired<A::Command>) {
        match fired {
            Fired::Command(command) => self.run(command),
            Fired::LingerOver => self.linger_over(),
        }
    }

    /// After every other effect of a command: the windows it closed, then the end if none is
    /// left, or the linger it asked for in place of the end.
    pub(super) fn closed(&self, closing: Vec<WindowId>, linger: Option<Duration>) {
        let closed = !closing.is_empty();
        for window in closing {
            self.close_window(window);
        }
        if let Some(backstop) = linger {
            self.linger_if_last(closed, backstop);
        }
        if closed {
            self.exit_if_no_window();
        }
    }

    /// `fx.linger(backstop)`, taken only from the command that closed the last open window.
    fn linger_if_last(&self, closed: bool, asked: Duration) {
        if self.exit.get().is_some() {
            tracing::debug!(app = self.id, "a linger asked for while ending; nothing to keep running for");
            return;
        }
        let ignored = if !closed {
            Some("this command closed no window")
        } else if !self.windows_open().is_empty() {
            Some("a window is still open")
        } else if self.lingering() {
            Some("the app already lingers, under the backstop it asked for first")
        } else {
            None
        };
        if let Some(why) = ignored {
            tracing::warn!(
                app = self.id,
                "fx.linger ignored: {why}; only the command that closes the last window lingers"
            );
            return;
        }
        if asked > MAX_LINGER {
            tracing::warn!(app = self.id, ?asked, "a linger's backstop is capped at {MAX_LINGER:?}");
        }
        let backstop = asked.min(MAX_LINGER);
        self.linger.set(Some(Lingering { since: Instant::now(), backstop }));
        self.timers.borrow_mut().after(LINGER, backstop, Box::new(|| Fired::LingerOver));
        tracing::info!(
            app = self.id,
            ?backstop,
            "the last window closed; running on without one until the app ends"
        );
    }

    /// A window is open again (a second launch, or one the app opened): the linger is over, and
    /// its backstop with it. Nothing, when the app was not lingering.
    pub(super) fn linger_ended(&self) {
        if let Some(lingered) = self.linger.take() {
            self.timers.borrow_mut().stop(LINGER);
            tracing::info!(
                app = self.id,
                after = ?lingered.since.elapsed(),
                "a window is open again; the linger is over"
            );
        }
    }

    /// The backstop passed with the app still lingering: the framework ends it. A late fire, after
    /// a window came back or the app ended, does nothing.
    fn linger_over(&self) {
        let Some(lingered) = self.linger.take() else {
            return;
        };
        if self.exit.get().is_some() {
            return;
        }
        tracing::warn!(
            app = self.id,
            backstop = ?lingered.backstop,
            "the app lingered without a window past its backstop and had not ended; ending it now"
        );
        self.request_exit(ExitReason::LastWindowClosed);
    }
}
