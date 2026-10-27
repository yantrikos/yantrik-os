//! An app of the framework that is not running is started for the built-in companion, with no
//! window, before it describes or acts on it (design #978 step 1). `yos` does the same for every
//! other Mind (`deploy/yantrik-os/yos`, `start_framework_app`); the rules are the same:
//!
//! - the shell is asked (`start_app`) and starts the app's one instance on the person's display;
//! - an act WAITS for it, at most [`BOUND`], and then runs once; one that cannot run in time is
//!   dropped, never queued, so it cannot say "nothing was done" and run later;
//! - a describe during the start says it is starting; only a start that failed says the app is
//!   not running;
//! - the sentences are the shell's (`say` in its answer), written once from its list of apps.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_ipc_transport::SyncRpcClient;

/// The longest a describe or an act waits for a start.
pub const BOUND: Duration = Duration::from_secs(15);

/// Between two looks at whether the app answers.
const STEP: Duration = Duration::from_millis(100);

/// How often the shell is asked again while a start is waited for, to hear of one that failed.
const RECHECK: Duration = Duration::from_secs(1);

fn address(app: &str) -> String {
    yantrik_ipc_transport::server::RpcServer::default_address(&format!("app-{app}"))
}

/// Whether the app's surface answers: a connection is made.
#[cfg(unix)]
fn answers(app: &str) -> bool {
    std::os::unix::net::UnixStream::connect(address(app)).is_ok()
}

#[cfg(not(unix))]
fn answers(_app: &str) -> bool {
    false
}

/// The shell's answer to `start_app` for `app`, if it gave one.
fn ask(app: &str) -> Option<Value> {
    let client = SyncRpcClient::new(&yantrik_ipc_transport::server::RpcServer::default_address("app-shell"))
        .with_timeout(Duration::from_secs(10));
    let reply = client
        .call("app.act", json!({ "action": "start_app", "args": { "name": app } }))
        .ok()?;
    let said = reply.get("result").filter(|r| r.is_object()).unwrap_or(&reply);
    said.get("state").is_some().then(|| said.clone())
}

/// Make sure `app` answers before the call is made: `Ok` when it does within [`BOUND`] or is not
/// an app that starts on demand; `Err` with the sentence to give, and the call is not made.
pub fn ensure(app: &str, acting: bool) -> Result<(), String> {
    ensure_with(app, acting, BOUND, || answers(app), || ask(app), std::thread::sleep)
}

/// [`ensure`] with the world given: whether the app answers, what the shell says, and how to wait.
pub fn ensure_with(
    app: &str,
    acting: bool,
    bound: Duration,
    answers: impl Fn() -> bool,
    mut ask: impl FnMut() -> Option<Value>,
    pause: impl Fn(Duration),
) -> Result<(), String> {
    if answers() {
        return Ok(());
    }
    let began = Instant::now();
    let Some(mut said) = ask() else { return Ok(()) };
    if said["state"] == "not_on_demand" {
        return Ok(());
    }
    let mut last_asked = Instant::now();
    loop {
        if said["state"] == "failed" {
            return Err(sentence(&said, app, acting, true));
        }
        if said["state"] == "running" && answers() {
            return Ok(());
        }
        if began.elapsed() >= bound {
            break;
        }
        pause(STEP);
        if answers() && began.elapsed() < bound {
            return Ok(());
        }
        if last_asked.elapsed() >= RECHECK {
            last_asked = Instant::now();
            said = ask().unwrap_or(said);
        }
    }
    // The bound passed: one last look at the shell's account, so a start that failed meanwhile is
    // told as failed. The call is dropped either way.
    let said = ask().unwrap_or(said);
    let failed = said["state"] == "failed";
    Err(sentence(&said, app, acting, failed))
}

/// The shell's sentence for this case, or a plain one if it sent none.
fn sentence(said: &Value, app: &str, acting: bool, failed: bool) -> String {
    let kind = format!("{}_{}", if acting { "act" } else { "describe" }, if failed { "failed" } else { "starting" });
    if let Some(told) = said["say"][kind.as_str()].as_str() {
        return told.to_string();
    }
    let name = said["name"].as_str().unwrap_or(app);
    match (acting, failed) {
        (true, true) => format!("{name} could not be started; nothing was done."),
        (true, false) => format!("{name} is starting; nothing was done \u{2014} send it again in a moment."),
        (false, true) => format!("{name} is not running: the desktop could not start it."),
        (false, false) => format!("{name} is starting; ask again in a moment."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn starting() -> Value {
        json!({ "app": "notes", "name": "Notes", "state": "starting", "say": {
            "act_starting": "Notes is starting; nothing was done \u{2014} send it again in a moment.",
            "describe_starting": "Notes is starting; ask again in a moment.",
            "act_failed": "Notes could not be started (it ended); nothing was done.",
            "describe_failed": "Notes is not running: the desktop could not start it (it ended).",
        }})
    }

    #[test]
    fn an_app_that_answers_is_not_started() {
        let asked = Cell::new(0);
        let ok = ensure_with("notes", true, BOUND, || true, || { asked.set(asked.get() + 1); None }, |_| {});
        assert_eq!(ok, Ok(()));
        assert_eq!(asked.get(), 0);
    }

    #[test]
    fn an_act_waits_for_the_start_and_then_goes_ahead() {
        let polls = Cell::new(0);
        let up = || polls.get() >= 3;
        let ok = ensure_with("notes", true, BOUND, up, || Some(starting()), |_| polls.set(polls.get() + 1));
        assert_eq!(ok, Ok(()));
    }

    #[test]
    fn a_start_that_is_not_done_by_the_bound_drops_the_act_and_says_so_word_for_word() {
        let err = ensure_with("notes", true, Duration::from_millis(30), || false, || Some(starting()), std::thread::sleep)
            .unwrap_err();
        assert_eq!(err, "Notes is starting; nothing was done \u{2014} send it again in a moment.");
        let err = ensure_with("notes", false, Duration::from_millis(30), || false, || Some(starting()), std::thread::sleep)
            .unwrap_err();
        assert_eq!(err, "Notes is starting; ask again in a moment.");
        assert!(!err.contains("not running"));
    }

    /// An instance that becomes ready just after the bound is not called: the act was dropped.
    #[test]
    fn an_app_that_comes_up_just_after_the_bound_is_still_not_called() {
        let began = Instant::now();
        let bound = Duration::from_millis(40);
        let up = || began.elapsed() > bound + Duration::from_millis(5);
        let err = ensure_with("notes", true, bound, up, || Some(starting()), std::thread::sleep).unwrap_err();
        assert!(err.contains("nothing was done"), "{err}");
    }

    #[test]
    fn a_failed_start_is_told_at_once_and_the_describe_says_not_running() {
        let failed = json!({ "app": "notes", "name": "Notes", "state": "failed", "say": starting()["say"].clone() });
        let err = ensure_with("notes", true, BOUND, || false, || Some(failed.clone()), |_| {}).unwrap_err();
        assert!(err.contains("nothing was done"), "{err}");
        let err = ensure_with("notes", false, BOUND, || false, || Some(failed.clone()), |_| {}).unwrap_err();
        assert!(err.contains("is not running"), "{err}");
    }

    #[test]
    fn an_app_that_does_not_start_on_demand_is_left_to_the_caller() {
        let said = json!({ "state": "not_on_demand" });
        assert_eq!(ensure_with("terminal", true, BOUND, || false, || Some(said.clone()), |_| {}), Ok(()));
        assert_eq!(ensure_with("notes", true, BOUND, || false, || None, |_| {}), Ok(()), "no desktop to ask");
    }
}
