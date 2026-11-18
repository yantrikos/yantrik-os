//! The system monitor's two honesty rules, tested without a window or a service (moved here
//! from `tests/sysmon-core`, which compiled `outcome.rs` by path, when the app became a library).
//!
//! The faults these cover were both invisible from the screen: `kill_process` answered
//! `{"killed": pid}` whether the service, the local fallback, or neither had managed it, and a
//! dead service moved every reading on the window to this process's own `sysinfo` with nothing
//! saying so — which is where an audit's `cpu_model: ""` and `interfaces[0].ip: ""` came from.
//!
//! The kill tests below spawn their own children and end them, so "the process is gone" is read
//! out of the real process table rather than asserted about a mock. The zombie case is the one
//! that matters most: a killed child whose parent has not waited on it is still in the table,
//! and a verifier that calls that "still running" turns every successful kill into a reported
//! failure. They read `/proc`, so they run on Linux, as the old crate's did.

use crate::outcome::*;
use std::process::{Child, Command};
use std::time::Duration;

/// A child of our own to end. `sleep` is the smallest thing that stays alive long enough to
/// be killed on purpose and short enough to disappear if a test ever leaks one.
fn sleeper() -> Child {
    Command::new("sleep").arg("30").spawn().expect("sleep must be on PATH")
}

fn signal(pid: u32, number: i32) {
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, number) }, 0, "signalling {pid}");
}

// ── The guard ───────────────────────────────────────────────────

#[test]
fn init_and_nonsense_pids_are_refused() {
    for pid in [-1, 0, 1] {
        assert!(checked_pid(pid).is_err(), "{pid} must not be killable");
    }
    assert_eq!(checked_pid(4242).unwrap(), 4242);
}

// ── Reading the process table ───────────────────────────────────

#[test]
fn a_zombie_is_gone_and_a_stopped_process_is_not() {
    // The second field is the executable's own name, in parentheses, and it may contain
    // both spaces and parentheses: splitting the line on whitespace reads the wrong field.
    assert_eq!(liveness_from_stat("42 (sleep) S 1 42 42 0 -1"), Liveness::Running);
    assert_eq!(liveness_from_stat("42 (my (odd) name) R 1 42"), Liveness::Running);
    assert_eq!(liveness_from_stat("42 (a b c) Z 1 42"), Liveness::Gone);
    assert_eq!(liveness_from_stat("42 (sleep) X 1 42"), Liveness::Gone);
    // Stopped under a debugger, and asleep in a syscall, are both still running.
    assert_eq!(liveness_from_stat("42 (sleep) T 1 42"), Liveness::Running);
    assert_eq!(liveness_from_stat("42 (sleep) D 1 42"), Liveness::Running);
}

#[test]
fn a_killed_child_reads_as_gone_before_it_is_reaped() {
    let mut child = sleeper();
    let pid = child.id();
    assert_eq!(liveness(pid), Liveness::Running);

    signal(pid, libc::SIGKILL);
    let (exited, waited_ms) = wait_until_gone(pid, VERIFY_BUDGET, VERIFY_STEP, liveness);
    // Nothing has called `wait` yet, so the entry is still in /proc — as a zombie. It has
    // exited, and that is what the answer has to say.
    assert!(exited, "a SIGKILLed child must read as gone within {VERIFY_BUDGET:?}");
    assert!(waited_ms <= 1000, "took {waited_ms}ms");

    child.wait().unwrap();
    assert_eq!(liveness(pid), Liveness::Gone, "and gone for good once reaped");
}

#[test]
fn sigterm_is_confirmed_against_the_table_not_the_syscall() {
    let mut child = sleeper();
    let pid = child.id();
    signal(pid, libc::SIGTERM);
    let (exited, waited_ms) = wait_until_gone(pid, VERIFY_BUDGET, VERIFY_STEP, liveness);
    assert!(exited);
    let observed = classify(pid, "sleep", Signal::Term, Source::Local, exited, waited_ms)
        .expect("a process that is gone is a successful kill");
    let json = observed.json();
    assert_eq!(json["pid"], pid);
    assert_eq!(json["signal"], "SIGTERM");
    assert_eq!(json["via"], "local");
    assert_eq!(json["exited"], true);
    assert!(json["confirmed_gone_after_ms"].is_number());
    child.wait().unwrap();
}

#[test]
fn a_pid_that_never_existed_is_gone_and_cannot_be_signalled() {
    // A pid of our own that we have already reaped: nothing else can be holding it, and it
    // is the case the old local fallback swallowed — `sys.process(pid)` returned `None` and
    // the kill simply did nothing, while the action reported the process ended.
    let mut child = sleeper();
    let pid = child.id();
    signal(pid, libc::SIGKILL);
    child.wait().unwrap();
    assert_eq!(liveness(pid), Liveness::Gone);

    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) }, -1);
    let reason = signal_failure(pid, &std::io::Error::last_os_error());
    assert!(reason.contains("no process"), "{reason}");
}

#[test]
fn the_reason_a_signal_failed_is_the_errno_not_a_bool() {
    let denied = signal_failure(9, &std::io::Error::from_raw_os_error(libc::EPERM));
    assert!(denied.contains("not permitted"), "{denied}");
    let missing = signal_failure(9, &std::io::Error::from_raw_os_error(libc::ESRCH));
    assert!(missing.contains("no process"), "{missing}");
}

// ── Waiting, and giving up ──────────────────────────────────────

#[test]
fn waiting_stops_at_the_budget_and_reports_how_long_it_waited() {
    let mut looks = 0;
    let (exited, waited) = wait_until_gone(
        7,
        Duration::from_millis(80),
        Duration::from_millis(10),
        |_| {
            looks += 1;
            Liveness::Running
        },
    );
    assert!(!exited, "a process that never goes must not be reported as killed");
    assert!(waited >= 80, "gave up after {waited}ms, before the budget");
    assert!(looks > 1, "looked once and gave up");

    // And it returns the moment the process goes, rather than sitting out the budget.
    let mut looks = 0;
    let (exited, _) = wait_until_gone(
        7,
        Duration::from_secs(5),
        Duration::from_millis(5),
        |_| {
            looks += 1;
            if looks > 2 { Liveness::Gone } else { Liveness::Running }
        },
    );
    assert!(exited);
    assert_eq!(looks, 3);
}

// ── What the answer says ────────────────────────────────────────

#[test]
fn a_process_still_running_is_never_a_successful_kill() {
    let declined = classify(500, "stubborn", Signal::Term, Source::Service, false, 1000)
        .expect_err("still running is not killed");
    assert!(declined.contains("SIGTERM"), "{declined}");
    assert!(declined.contains("force"), "the next step has to be in the message: {declined}");

    let stuck = classify(500, "stubborn", Signal::Kill, Source::Local, false, 1000)
        .expect_err("still there after SIGKILL is not killed either");
    assert!(stuck.contains("SIGKILL"), "{stuck}");
    assert!(stuck.contains("uninterruptible"), "{stuck}");
}

#[test]
fn the_answer_names_the_path_that_did_it() {
    let by_service = classify(500, "sleep", Signal::Term, Source::Service, true, 12).unwrap();
    assert_eq!(by_service.json()["via"], "service");
    assert!(by_service.sentence().contains("service"));

    let by_app = classify(500, "sleep", Signal::Kill, Source::Local, true, 3).unwrap();
    assert_eq!(by_app.json()["via"], "local");
    assert_eq!(by_app.json()["signal"], "SIGKILL");

    // A pid below the top of the process list has no name on screen, and an empty one in
    // the answer would read as a process called nothing.
    let unnamed = classify(500, "  ", Signal::Term, Source::Local, true, 1).unwrap();
    assert_eq!(unnamed.json()["name"], "unknown");
}

// ── Where a reading came from ───────────────────────────────────

#[test]
fn a_fallback_reading_cannot_be_taken_without_the_note_that_it_is_one() {
    let (value, from) = reading::<i32>(Ok(7), || 0);
    assert_eq!(value, 7);
    assert!(!from.degraded());
    assert_eq!(from.source, Source::Service);
    assert_eq!(from.notice(), None);

    let (value, from) = reading::<i32>(Err("connection refused".into()), || 3);
    assert_eq!(value, 3, "the fallback still answers; that is the point of it");
    assert!(from.degraded());
    assert_eq!(from.source, Source::Local);
    let notice = from.notice().expect("a degraded reading has to say so");
    assert!(notice.contains("connection refused"), "{notice}");
}

#[test]
fn one_degraded_half_of_a_poll_degrades_the_whole_window() {
    let both = worse(Provenance::service(), Provenance::service());
    assert!(!both.degraded());
    assert!(worse(Provenance::service(), Provenance::local("no socket")).degraded());
    assert!(worse(Provenance::local("no socket"), Provenance::service()).degraded());
}

#[test]
fn a_field_nobody_measured_is_null_and_not_an_empty_measurement() {
    assert_eq!(measured(""), serde_json::Value::Null);
    assert_eq!(measured("   "), serde_json::Value::Null);
    assert_eq!(measured("AMD Ryzen 9"), serde_json::json!("AMD Ryzen 9"));
}

#[test]
fn a_degraded_reading_does_not_wipe_the_reason_a_kill_failed() {
    assert_eq!(compose_notice(None, None), "");
    assert_eq!(compose_notice(Some("could not end 900"), None), "could not end 900");
    assert_eq!(compose_notice(None, Some("readings are local")), "readings are local");
    let both = compose_notice(Some("could not end 900"), Some("readings are local"));
    assert!(both.contains("could not end 900") && both.contains("readings are local"));
}
