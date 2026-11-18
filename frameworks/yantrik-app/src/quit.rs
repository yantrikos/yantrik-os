//! Quitting: the ways an app is asked to end while its windows are open, and who asked.
//!
//! Closing the last window ends the process (`ExitReason::LastWindowClosed`), unless the command
//! that closed it asked to linger (`fx.linger`): the app then runs on with no window until it
//! runs `fx.exit()` or its backstop passes, and what follows applies during a linger too. Two
//! more ways reach the app as `Event::ExitRequested`, and its reason says whether anyone is there
//! to answer:
//!
//! - **A person** (`ExitReason::Person`): the app's `fx.request_quit()`, which it raises from
//!   Ctrl+Q or a Quit menu. The app's close flow runs over every window and may ask about unsaved
//!   work, once; the person may cancel, and the app runs on. Nothing ends until the flow runs
//!   `fx.exit()`. An app that takes the default (`None`) ends at once.
//! - **The session or the system** (`ExitReason::Logout`): SIGTERM (a shutdown, a logout, a
//!   kill), SIGHUP (the session hung up), or the display going (the compositor exited, which on
//!   this OS's shutdown happens at the same moment as the SIGTERM to the whole session). No one
//!   is there to answer, and the process will be killed after a grace period whatever the app
//!   does. The app saves without asking (a draft of every unsaved edit, say) and runs
//!   `fx.exit()`. It is asked once, whichever comes first and however many come: nothing
//!   stacks, and a person's quit during it asks nothing. A signal still queued when the event
//!   loop ends, because the display went first, is heard before the end, and so is the display
//!   going: the logout's writes need no display. [`SIGNAL_DEADLINE`] is a backstop for a flow
//!   that never finishes: past it, the framework ends the app.
//!
//! The signal handler is async-signal-safe: `signal-hook`'s self-pipe, whose handler only writes
//! a byte. A thread of its own reads the pipe and posts to the core's inbox on the reserved lane,
//! so a full inbox of commands cannot lose a shutdown, and the post wakes the UI thread, where
//! the event is raised. SIGINT keeps its default: Ctrl+C in a terminal ends the app as it always
//! did, without the logout's save (it could join this path later).

use std::time::{Duration, Instant};

use crate::app::App;
use crate::cx::Inbound;
use crate::queue::Post;

/// After the first signal, the backstop: an app that has not ended by then is ended. Only a flow
/// that never finishes reaches it; an app that saves and exits is long gone. The shutdown's
/// writes may use what is left of it ([`lane_grace`]), never less than the usual grace. Well
/// under the 90 s systemd gives the session between SIGTERM and SIGKILL (no `TimeoutStopSec` is
/// set on it); a shorter stop timeout on the session unit should shorten this with it.
pub const SIGNAL_DEADLINE: Duration = Duration::from_secs(20);

/// The logout the core raised: the signal that asked for it (`None`: the display went), and when.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LoggedOut {
    pub(crate) signal: Option<Signal>,
    pub(crate) at: Instant,
}

/// How long the shutdown's writes may take: `grace`, or after a signal heard `since` ago, what is
/// left of [`SIGNAL_DEADLINE`] if that is longer.
pub(crate) fn lane_grace(grace: Duration, since_signal: Option<Duration>) -> Duration {
    match since_signal {
        Some(since) => SIGNAL_DEADLINE.saturating_sub(since).max(grace),
        None => grace,
    }
}

/// A signal that asks the process to end, as the core hears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// SIGTERM: a shutdown, a logout, a kill.
    Terminate,
    /// SIGHUP: the session hung up.
    HangUp,
}

impl Signal {
    /// The signal's usual name, for the log.
    pub fn name(self) -> &'static str {
        match self {
            Signal::Terminate => "SIGTERM",
            Signal::HangUp => "SIGHUP",
        }
    }

    /// The signal of this number, if it is one of these.
    #[cfg(unix)]
    pub fn from_raw(raw: i32) -> Option<Signal> {
        match raw {
            signal_hook::consts::SIGTERM => Some(Signal::Terminate),
            signal_hook::consts::SIGHUP => Some(Signal::HangUp),
            _ => None,
        }
    }
}

/// What the signal thread tells the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// A signal arrived.
    Signal(Signal),
    /// `deadline` has passed since the first signal.
    Overdue(Signal),
}

/// Hears SIGTERM and SIGHUP for the rest of the process and posts each to the core behind `post`,
/// with an `Overdue` once `deadline` has passed since the first. The runner calls this once,
/// before its event loop runs, with [`SIGNAL_DEADLINE`]; a test that raises a signal at its own
/// process may call it over its app's post. Until then, and on a platform without signals, the
/// signals keep their default.
#[cfg(unix)]
pub fn listen<A: App>(post: Post<Inbound<A>>, deadline: Duration) -> std::io::Result<()> {
    use signal_hook::consts::{SIGHUP, SIGTERM};
    let mut signals = signal_hook::iterator::Signals::new([SIGTERM, SIGHUP])?;
    std::thread::Builder::new()
        .name(format!("{}-signals", A::ID))
        .spawn(move || {
            let heard = signals.forever().filter_map(Signal::from_raw);
            forward(heard, deadline, move |heard| {
                post.post_control(match heard {
                    Heard::Signal(signal) => Inbound::Signal(signal),
                    Heard::Overdue(signal) => Inbound::Overdue(signal),
                })
                .is_ok()
            });
        })?;
    Ok(())
}

#[cfg(not(unix))]
pub fn listen<A: App>(post: Post<Inbound<A>>, deadline: Duration) -> std::io::Result<()> {
    let _ = (post, deadline);
    Ok(())
}

/// Every signal heard, passed on in order; the first also arms the deadline, once, on a thread
/// of its own, so the signals go on being heard while it waits. `post` says whether the core
/// still takes posts: once it does not (the app is shutting down), the rest are not passed on.
/// A signal then is ignored, so the shutdown's bounded writes are not cut short.
pub(crate) fn forward<P>(heard: impl IntoIterator<Item = Signal>, deadline: Duration, post: P)
where
    P: Fn(Heard) -> bool + Clone + Send + 'static,
{
    let mut armed = false;
    for signal in heard {
        if !post(Heard::Signal(signal)) {
            return;
        }
        if !std::mem::replace(&mut armed, true) {
            let overdue = post.clone();
            let spawned = std::thread::Builder::new()
                .name("quit-deadline".into())
                .spawn(move || {
                    std::thread::sleep(deadline);
                    overdue(Heard::Overdue(signal));
                });
            if let Err(e) = spawned {
                // Without the backstop the app still hears the signal; the system's kill is the
                // end then, as it always was.
                tracing::warn!("no thread for the quit deadline: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn recording() -> (impl Fn(Heard) -> bool + Clone + Send + 'static, mpsc::Receiver<Heard>) {
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Arc::new(std::sync::Mutex::new(tx));
        (move |h| tx.lock().unwrap().send(h).is_ok(), rx)
    }

    #[test]
    fn every_signal_is_passed_on_and_the_deadline_is_armed_once() {
        let (post, rx) = recording();
        let signals = [Signal::Terminate, Signal::Terminate, Signal::HangUp];
        forward(signals, Duration::from_millis(200), post);
        let wait = Duration::from_secs(5);
        let mut got: Vec<Heard> = (0..3).map(|_| rx.recv_timeout(wait).unwrap()).collect();
        got.push(rx.recv_timeout(wait).expect("the deadline came"));
        assert_eq!(
            got,
            vec![
                Heard::Signal(Signal::Terminate),
                Heard::Signal(Signal::Terminate),
                Heard::Signal(Signal::HangUp),
                Heard::Overdue(Signal::Terminate),
            ]
        );
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "one deadline only");
    }

    #[test]
    fn nothing_more_is_passed_on_once_the_core_takes_no_posts() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        forward([Signal::HangUp, Signal::Terminate], Duration::from_secs(60), move |h| {
            s.lock().unwrap().push(h);
            false
        });
        assert_eq!(*seen.lock().unwrap(), vec![Heard::Signal(Signal::HangUp)]);
    }

    #[test]
    fn after_a_signal_the_writes_may_use_what_is_left_of_the_deadline() {
        let grace = Duration::from_secs(2);
        assert_eq!(lane_grace(grace, None), grace, "no signal: the usual grace");
        assert_eq!(lane_grace(grace, Some(Duration::from_secs(5))), SIGNAL_DEADLINE - Duration::from_secs(5));
        assert_eq!(lane_grace(grace, Some(SIGNAL_DEADLINE)), grace, "never less than the grace");
    }

    #[cfg(unix)]
    #[test]
    fn only_sigterm_and_sighup_are_heard() {
        assert_eq!(Signal::from_raw(signal_hook::consts::SIGTERM), Some(Signal::Terminate));
        assert_eq!(Signal::from_raw(signal_hook::consts::SIGHUP), Some(Signal::HangUp));
        assert_eq!(Signal::from_raw(signal_hook::consts::SIGINT), None);
        assert_eq!(Signal::Terminate.name(), "SIGTERM");
    }
}
