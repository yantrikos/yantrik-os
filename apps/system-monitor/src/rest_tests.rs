//! Nothing redraws at rest (the kit's rule), measured as Weather's is (apps/weather/src/
//! rest_tests.rs): the window drawn whenever it asks to be, at the size it opens at, over ten
//! minutes of the virtual clock with nobody touching it; what its timers start runs and lands.

use std::time::Duration;

use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_testing::AtRest;

use crate::app_tests::{normal, start};
use crate::machine::fake::FakeMachine;

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_left_alone_draws_nothing_while_the_numbers_hold() {
    let (app, _) = start(FakeMachine::new().reads(normal()));
    app.run_jobs();
    // The rail on screen, with the Mind able to answer, as on a desktop with the shell up.
    app.seen(Seen::Reach(Reach::Ready));
    app.resize(1200, 800);
    app.advance(Duration::from_secs(1));
    app.run_jobs();
    let rest = app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    // A live monitor: the machine is sampled every two seconds off the window's thread (300
    // samples, each a job, and the rail's refresh after each landing, 300), and a sample the same
    // as the last draws nothing.
    assert_eq!(rest, AtRest { frames: 0, fired: 600, jobs: 300 });
}
