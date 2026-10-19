//! A second launch shows the window of the first, or says why it cannot.
//!
//! Our apps are single-instance (`instance`): a second launch finds the slot held and exits. That
//! used to be all it did, on the belief that the shell would raise the window already open. The
//! shell can only raise a window its compositor lists, and on a fresh install (13 Oct) Calendar
//! and Notes were running with no window the person's desktop could see: a mind had opened them,
//! and a mind's apps are drawn in Mind View, a nested display of their own. The dock, the
//! workspace tile and `open_app` all answered "accepted", the second copy exited, nothing was
//! raised because nothing was listed, and nothing was said. Killing the first copy was the only
//! way back to the app.
//!
//! So the second launch asks the first, over its control surface, to present itself: map its
//! window if it is not mapped, ask for it to come back if it is minimised, and answer where it is.
//! The first copy owns the window and is the only process that can show it. The second still
//! exits, so there is never a second copy; it only stops exiting in silence.
//!
//! `present` is an action every surface built on this runtime publishes ([`crate::control::App::serve`]
//! adds it), graded `safe` because it changes nothing a person keeps. It is the desktop's own
//! handover and not something a mind may use: a mind raising a window over the person's work is
//! what Mind View exists to stop (#239), so an agent's call is refused.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::control::{self, Action, Param};

/// The action's name on every surface.
pub const ACTION: &str = "present";

/// How long the second launch waits for the first to bind its socket.
///
/// A launch can land while the first copy is still starting: it has claimed the slot and not yet
/// served its surface. Binding takes tens of milliseconds on a release build; two seconds is a
/// bound on a copy that is stuck, and stays inside the shell's launch grace, so the dock still
/// reads this exit as a handover.
const BIND_WAIT: Duration = Duration::from_secs(2);

/// How long the first copy is given to answer once it is reachable.
const ANSWER_WAIT: Duration = Duration::from_secs(3);

/// Shows this process's window. Returns whether it did, with the reason when it did not.
type Presenter = Box<dyn Fn() -> Result<(), String>>;

thread_local! {
    /// Installed on the thread that owns the window, by [`crate::run_until_closed`], before the
    /// loop starts. A process that never runs a window has none, and says so when asked.
    static WINDOW: RefCell<Option<Presenter>> = const { RefCell::new(None) };
}

/// Install how this process shows its window. Called on the UI thread.
pub(crate) fn install(presenter: impl Fn() -> Result<(), String> + 'static) {
    WINDOW.with(|cell| *cell.borrow_mut() = Some(Box::new(presenter)));
}

/// The action as every surface publishes it.
pub(crate) fn action() -> Action {
    Action::new(ACTION, "Show this app's window and bring it forward; for the desktop's own launches")
        .risk("safe")
        .arg(Param::text("display").optional())
}

/// The handler [`crate::control::App::serve`] registers. Runs on the UI thread.
pub(crate) fn handle(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    if control::agent_is_calling() {
        return Err("present is the desktop's own handover between two launches of an app; an agent \
                    drives the app through its other actions and does not bring its window forward"
            .into());
    }
    let here = std::env::var("WAYLAND_DISPLAY").ok();
    WINDOW.with(|cell| answer(cell.borrow().as_deref(), here.as_deref(), args["display"].as_str()))
}

/// What the first copy answers, given how it shows its window (if it has one), the display it
/// draws on, and the display the asking launch is on.
fn answer(
    window: Option<&dyn Fn() -> Result<(), String>>,
    here: Option<&str>,
    asked_from: Option<&str>,
) -> Result<serde_json::Value, String> {
    let Some(show) = window else {
        return Ok(serde_json::json!({ "window": false }));
    };
    show()?;
    // Said, not decided here: this process cannot move its window to another display, and the
    // launch that asked is the one that knows which display the person is looking at.
    let elsewhere = matches!((here, asked_from), (Some(h), Some(a)) if h != a);
    Ok(serde_json::json!({ "window": true, "presented": true, "display": here, "elsewhere": elsewhere }))
}

/// What the running copy made of the request, as the second launch reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presented {
    /// Its window is shown on this display.
    Shown,
    /// Its window is shown, on a display other than this launch's: Mind View, where a mind's apps
    /// are drawn.
    Elsewhere(String),
    /// It runs without a window, so there is nothing to show.
    NoWindow,
    /// It answered and would not, with its reason.
    Refused(String),
    /// It did not answer: no surface, or not in time.
    Unanswered(String),
}

impl Presented {
    /// The one sentence a launch says about it, or `None` when the window is in front of the
    /// person and there is nothing to add.
    pub fn sentence(&self, app: &str) -> Option<String> {
        match self {
            Presented::Shown => None,
            Presented::Elsewhere(display) => Some(format!(
                "{app} is already open on another display ({display}), where a mind's apps are drawn; \
                 its window was shown there."
            )),
            Presented::NoWindow => Some(format!(
                "{app} is already running without a window, so there is no window to show."
            )),
            Presented::Refused(why) => Some(format!("{app} is already running and would not show its window: {why}")),
            Presented::Unanswered(why) => Some(format!(
                "{app} is already running and did not answer a request to show its window ({why})."
            )),
        }
    }
}

/// Read the reply to one `app.act present`.
fn read(reply: Result<serde_json::Value, yantrik_ipc_transport::RpcError>) -> Presented {
    match reply {
        Err(e) if e.code == yantrik_surface::REFUSED => Presented::Refused(e.message),
        Err(e) => Presented::Unanswered(e.message),
        Ok(reply) => {
            let result = &reply["result"];
            if result["window"] != true {
                Presented::NoWindow
            } else if result["elsewhere"] == true {
                Presented::Elsewhere(result["display"].as_str().unwrap_or("unknown").to_string())
            } else {
                Presented::Shown
            }
        }
    }
}

/// Ask the running copy of `app_id` to present itself, and say what came of it.
///
/// Called by a launch that found the single-instance slot held, before it exits.
pub fn ask(app_id: &str) -> Presented {
    let service = control::service_id_for(app_id);
    let address = yantrik_ipc_transport::mind_door::client_address(&service)
        .unwrap_or_else(|| yantrik_ipc_transport::RpcServer::default_address(&service));
    ask_at(&address)
}

/// [`ask`], at a socket named outright. Split out so a test can serve the first copy in a
/// directory of its own.
pub(crate) fn ask_at(address: &str) -> Presented {
    // Waited for before the one call, rather than calling in a loop: a failed call trips the
    // client's breaker for five seconds, and every retry after it fails without being sent.
    let deadline = Instant::now() + BIND_WAIT;
    while !reachable(address) {
        if Instant::now() >= deadline {
            return Presented::Unanswered(format!("nothing answers at {address}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let here = std::env::var("WAYLAND_DISPLAY").ok();
    let request = serde_json::json!({ "action": ACTION, "args": { "display": here } });
    read(yantrik_ipc_transport::SyncRpcClient::new(address).with_timeout(ANSWER_WAIT).call("app.act", request))
}

#[cfg(unix)]
fn reachable(address: &str) -> bool {
    std::os::unix::net::UnixStream::connect(address).is_ok()
}

#[cfg(not(unix))]
fn reachable(_address: &str) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// The case the bug turned on: a copy with no window, asked to present. It answers that it has
    /// none, and the launch says so in a sentence instead of nothing.
    #[test]
    fn a_copy_without_a_window_answers_that_it_has_none_and_the_launch_says_so() {
        let reply = answer(None, Some("wayland-0"), Some("wayland-0")).unwrap();
        assert_eq!(reply["window"], false);
        let read = read(Ok(serde_json::json!({ "accepted": true, "result": reply })));
        assert_eq!(read, Presented::NoWindow);
        let said = read.sentence("Calendar").expect("a windowless copy is not passed over in silence");
        assert!(said.contains("without a window"), "{said}");
    }

    /// The handler as the dispatch calls it, in a process that never ran a window: nothing was
    /// installed, so it answers that it has none rather than claiming a window it does not have.
    #[test]
    fn the_action_in_a_process_that_never_ran_a_window_says_it_has_none() {
        let answered = std::thread::spawn(|| handle(&serde_json::json!({}))).join().unwrap().unwrap();
        assert_eq!(answered, serde_json::json!({ "window": false }));
        assert_eq!(action().permission, "safe");
    }

    #[test]
    fn a_copy_with_a_window_shows_it_and_the_launch_has_nothing_to_add() {
        let shown = Cell::new(0);
        let show = || {
            shown.set(shown.get() + 1);
            Ok(())
        };
        let reply = answer(Some(&show), Some("wayland-0"), Some("wayland-0")).unwrap();
        assert_eq!(shown.get(), 1, "the window was not shown");
        let read = read(Ok(serde_json::json!({ "result": reply })));
        assert_eq!(read, Presented::Shown);
        assert_eq!(read.sentence("Calendar"), None);
    }

    /// A mind opened the app, so it is drawn in Mind View. It is shown there, and the person's
    /// launch is told where, since that copy cannot move its window to their display.
    #[test]
    fn a_window_on_another_display_is_shown_there_and_the_launch_is_told_where() {
        let show = || Ok(());
        let reply = answer(Some(&show), Some("wayland-1"), Some("wayland-0")).unwrap();
        let read = read(Ok(serde_json::json!({ "result": reply })));
        assert_eq!(read, Presented::Elsewhere("wayland-1".into()));
        assert!(read.sentence("Notes").unwrap().contains("wayland-1"));
    }

    #[test]
    fn a_window_that_cannot_be_shown_is_a_refusal_with_its_reason() {
        let show = || Err("the window is closing".to_string());
        assert_eq!(answer(Some(&show), None, None).unwrap_err(), "the window is closing");
        let refused = read(Err(yantrik_ipc_transport::RpcError {
            code: yantrik_surface::REFUSED,
            message: "the window is closing".into(),
            data: None,
        }));
        assert_eq!(refused, Presented::Refused("the window is closing".into()));
    }

    #[test]
    fn a_launch_with_nobody_to_ask_says_it_was_not_answered() {
        let dir = std::env::temp_dir().join(format!("yantrik-present-none-{}", std::process::id()));
        let started = Instant::now();
        let read = ask_at(&dir.join("app-nobody.sock").display().to_string());
        assert!(matches!(read, Presented::Unanswered(_)), "{read:?}");
        assert!(started.elapsed() < BIND_WAIT + Duration::from_secs(1), "waited past the bind budget");
    }
}
