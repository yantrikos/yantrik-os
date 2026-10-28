//! Nothing redraws at rest (the kit's rule, #944 and #949), measured the way the display pays for
//! it. Arcade runs no clock at all: a window with nothing to show draws nothing and arms nothing,
//! and bringing it forward over an unchanged library is no frame.

use std::time::Duration;

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_testing::AtRest;

use super::{meadow_run, open, seeded};

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_with_nothing_running_arms_no_timer_and_draws_nothing() {
    let f = open();
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    f.quiet();
    // Past the deadline of the last answer (the framework's, not the app's): what is left armed
    // after that is the app's, and there is none.
    f.app.advance(crate::intents::WITHIN + Duration::from_secs(1));
    assert_eq!(f.app.timers_armed(), 0);
    let rest = f.app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    assert_eq!(rest, AtRest::default(), "ten minutes alone: no frame, no timer, no job");
}

#[test]
fn a_browser_job_in_flight_arms_no_timer_either() {
    let f = open();
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    f.start_slow(vec![("verify", json!({ "game": "meadow-run" }))]).remove(0).settle().unwrap();
    f.app.advance(crate::intents::WITHIN + Duration::from_secs(1));
    assert_eq!(f.state()["jobs"][0]["status"], json!("running"));
    assert_eq!(f.app.timers_armed(), 0, "a worker is waited for, not polled");
    f.quiet();
    assert_eq!(f.state()["jobs"][0]["status"], json!("done"));
}

#[test]
fn bringing_the_window_forward_over_an_unchanged_library_draws_nothing() {
    let f = open();
    seeded(&f);
    f.app.settle_frames();
    f.app.activate(WindowId::PRIMARY);
    f.quiet();
    assert_eq!(f.app.draw(), 0, "the same lists are not a new frame");
}

#[test]
fn typing_in_the_editor_draws_nothing_the_editor_has_not_already() {
    let f = open();
    f.app.settle_frames();
    f.type_spec(&meadow_run("Typing"));
    f.app.settle_frames();
    f.type_spec(&meadow_run("Typing"));
    assert_eq!(f.app.draw(), 0);
}
