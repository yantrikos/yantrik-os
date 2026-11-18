//! The window closed, a logout, and the display going: with a command on the lane, with text
//! unsaved, and with neither. Nothing is lost, nothing is half written, and nothing waits on a
//! person who is not there.

use std::time::{Duration, Instant};

use serde_json::json;
use yantrik_app::app::{ExitReason, WindowId};
use yantrik_app::cx::Inbound;
use yantrik_app::quit::Signal;

use super::{open, PIP};
use crate::update::life::CLOSE_BACKSTOP;

#[test]
fn closing_with_nothing_to_wait_for_ends_at_once() {
    let f = open();
    f.app.close_requested(WindowId::PRIMARY);
    assert_eq!(f.app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert!(!f.app.lingering());
}

/// A save on the lane when the window closes: the window goes at once and the app runs on without
/// it until the save has landed, whole.
#[test]
fn closing_with_a_save_on_the_lane_waits_for_it_to_land() {
    let f = open();
    let pending = f.app.act("new_character", json!({ "spec": PIP })).unwrap();
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.windows().is_empty(), "the window is closed at once");
    assert!(f.app.lingering(), "and the app runs on while the save lands");
    assert_eq!(f.app.exit_reason(), None);
    f.quiet();
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App));
    assert!(pending.settle().is_ok());
    assert_eq!(f.library().load_character("pip").unwrap().name, "Pip", "the character is on disk, whole");
}

#[test]
fn a_logout_by_a_signal_waits_for_the_lane_and_asks_nothing() {
    for signal in [Signal::Terminate, Signal::HangUp] {
        let f = open();
        f.type_spec("{ \"half\": \"a spec");
        f.app.act("new_character", json!({ "spec": PIP })).unwrap();
        let post = f.app.post();
        std::thread::spawn(move || post.post(Inbound::Signal(signal)).is_ok()).join().unwrap();
        f.quiet();
        assert!(f.app.exit_reason().is_some(), "{signal:?}");
        f.app.run_writes();
        // The unsaved text went to the draft lane, without a question.
        let kept = std::fs::read_to_string(yantrik_app::drafts::path_in(f.paths.state_dir(), "spec")).unwrap();
        assert!(kept.contains("half"), "{kept}");
        assert_eq!(f.library().load_character("pip").unwrap().name, "Pip");
    }
}

#[test]
fn the_display_going_is_a_logout_too() {
    let f = open();
    f.type_spec("{ \"typed\": 1 }");
    f.app.display_gone();
    f.quiet();
    assert!(f.app.exit_reason().is_some());
    f.app.run_writes();
    assert!(yantrik_app::drafts::path_in(f.paths.state_dir(), "spec").exists());
}

#[test]
fn a_logout_with_nothing_to_wait_for_ends_at_once() {
    let f = open();
    f.app.display_gone();
    assert!(f.app.exit_reason().is_some());
}

/// A second launch during the wait cancels a person's close, and a logout is never cancelled.
#[test]
fn a_second_launch_during_the_wait_cancels_a_close_but_not_a_logout() {
    let f = open();
    let pending = f.app.act("new_character", json!({ "spec": PIP })).unwrap();
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.lingering());
    f.app.launch_again().unwrap();
    assert!(!f.app.windows().is_empty(), "the window is back");
    assert!(!f.app.model(|m| m.closing), "and the close is over");
    f.quiet();
    assert_eq!(f.app.exit_reason(), None, "the app stays");
    assert!(pending.settle().is_ok());

    let f = open();
    let _pending = f.app.act("new_character", json!({ "spec": PIP })).unwrap();
    f.app.display_gone();
    f.app.present().ok();
    assert!(f.app.model(|m| m.ending), "the window coming back does not cancel a logout");
}

#[test]
fn a_lane_that_never_answers_does_not_hold_the_app_past_its_backstop() {
    let f = open();
    f.app.act("new_character", json!({ "spec": PIP })).unwrap();
    f.app.close_requested(WindowId::PRIMARY);
    let started = Instant::now();
    f.app.advance(CLOSE_BACKSTOP + Duration::from_secs(1));
    assert!(f.app.exit_reason().is_some() && !f.app.lingering(), "the framework ended the app at the backstop");
    assert!(started.elapsed() < Duration::from_secs(5), "the backstop is the virtual clock's, not a real wait");
}

#[test]
fn nothing_more_is_started_while_the_app_is_closing() {
    let f = open();
    f.app.act("new_character", json!({ "spec": PIP })).unwrap();
    f.app.close_requested(WindowId::PRIMARY);
    let refused = f.act("new_game", json!({ "spec": "{}" })).unwrap_err();
    assert_eq!(refused, crate::update::CLOSING);
}

/// Review M1 (probe): a browser action asked, the window closed before the lane lands. The job
/// must not start after the close (nothing is left to wait for it, and its browser would be
/// orphaned), and the caller is told why instead of "started".
#[test]
fn a_browser_job_is_not_started_once_the_app_is_closing() {
    let f = open();
    super::seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let pending = f.app.act("verify", json!({ "game": "meadow-run" })).unwrap();
    f.app.close_requested(WindowId::PRIMARY);
    f.quiet();
    assert!(f.browser.calls().is_empty(), "no worker started after the close: {:?}", f.browser.calls());
    assert_eq!(pending.settle().unwrap_err(), crate::update::CLOSING);
    assert!(f.app.exit_reason().is_some());
}

#[test]
fn a_browser_job_is_not_started_once_the_app_is_ending() {
    let f = open();
    super::seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let pending = f.app.act("screenshot", json!({ "game": "meadow-run" })).unwrap();
    f.app.display_gone();
    f.quiet();
    assert!(f.browser.calls().is_empty());
    assert_eq!(pending.settle().unwrap_err(), crate::update::CLOSING);
}

/// Review M1: a logout stops the headless browsers still running, because each has a debugging
/// port open that any local process could drive.
#[cfg(unix)]
#[test]
fn a_logout_stops_a_browser_that_is_still_running() {
    let f = open();
    let mut browser = std::process::Command::new("sleep").arg("60").spawn().unwrap();
    let nowhere = std::env::temp_dir().join("arcade-nonexistent");
    crate::browser::register(browser.id(), &nowhere, &nowhere, Default::default());
    // Only this child is checked: the list of live browsers is shared by every test that ends an app.
    f.app.display_gone();
    f.quiet();
    let status = browser.wait().unwrap();
    assert!(!status.success(), "it was killed, not left to finish");
}

/// A window closed while a browser job is under way: the app ends at once, the browser is dead, and
/// neither its profile nor its temporary folder is left behind. (The app used to close the window,
/// wait out the framework's grace period for the job, and exit without cleaning up.)
#[cfg(unix)]
#[test]
fn closing_the_window_mid_job_stops_the_browser_and_clears_its_folders() {
    let f = open();
    super::seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let base = std::env::temp_dir().join(format!("arcade-close-{}", std::process::id()));
    let profile = crate::browser::profile_in(&base).unwrap();
    let browser = crate::browser::tests::stand_in(&profile).unwrap();
    let tmp = browser.tmp.clone();
    let job = f.start_slow(vec![("verify", json!({ "game": "meadow-run" }))]).remove(0);
    job.settle().unwrap();
    assert_eq!(f.state()["jobs"][0]["status"], json!("running"));
    f.app.close_requested(WindowId::PRIMARY);
    f.app.tick();
    assert!(f.app.exit_reason().is_some(), "the app ends at once");
    assert!(!profile.exists() && !tmp.exists(), "both folders are gone: {} {}", profile.display(), tmp.display());
    assert_eq!(f.state()["jobs"][0]["status"], json!("failed"));
    assert_eq!(f.state()["jobs"][0]["detail"], json!(crate::update::CLOSING));
    drop(browser);
    let _ = std::fs::remove_dir_all(&base);
}
