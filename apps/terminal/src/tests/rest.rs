//! Nothing redraws at rest (the kit's rule, #944 and #949), measured the way the display pays for
//! it. A shell at its prompt arms no timer and costs no frame; a job running in one is looked at
//! now and then without drawing anything, because nothing on the screen has changed.

use std::time::Duration;

use yantrik_app::app::WindowId;
use yantrik_testing::AtRest;

use super::open;

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_shell_at_its_prompt_arms_no_timer_and_draws_nothing() {
    let f = open();
    f.until("the probe", |f| f.summary().contains("idle at a prompt"));
    f.app.advance(Duration::from_secs(10));
    f.quiet();
    assert_eq!(f.app.timers_armed(), 0);
    let rest = f.app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    assert_eq!(rest, AtRest::default(), "ten minutes alone: no frame, no timer, no job");
}

#[test]
fn a_job_that_prints_nothing_is_looked_at_without_a_frame() {
    let f = open();
    f.start_job("sleep 300");
    f.until("the probe", |f| f.summary().contains("running: sleep"));
    f.app.settle_frames();
    let rest = f.app.at_rest(Duration::from_secs(30), Duration::from_secs(1));
    assert_eq!(rest.frames, 0, "the same facts are not a new frame");
    assert!(rest.jobs > 0, "a running job is probed: that is how its end is seen");
}

#[test]
fn bringing_the_window_forward_over_unchanged_shells_draws_nothing() {
    let f = open();
    f.until("the probe", |f| f.summary().contains("idle at a prompt"));
    f.app.settle_frames();
    f.app.activate(WindowId::PRIMARY);
    f.app.advance(Duration::from_millis(500));
    f.quiet();
    assert_eq!(f.app.draw(), 0);
}

#[test]
fn typing_draws_the_screen_and_then_rests_again() {
    let f = open();
    f.until("the probe", |f| f.summary().contains("idle at a prompt"));
    f.app.settle_frames();
    f.type_until("echo hello", "echo hello");
    assert!(f.app.ui(|ui| ui.get_screen_text().contains("echo hello")), "a key moves the screen");
    f.app.draw_until_still();
    f.app.advance(Duration::from_secs(2));
    f.quiet();
    f.app.settle_frames();
    assert_eq!(f.app.at_rest(Duration::from_secs(30), Duration::from_secs(1)), AtRest::default());
}
