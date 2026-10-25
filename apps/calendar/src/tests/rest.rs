//! Nothing redraws at rest (the kit's rule), measured as Weather's is (apps/weather/src/
//! rest_tests.rs): the window drawn whenever it asks to be, at the size it opens at, over ten
//! minutes of the virtual clock with nobody touching it; what its timers start runs and lands.

use std::time::Duration;

use yantrik_app::app::Event;
use yantrik_app::mind::Reach;
use yantrik_testing::AtRest;

use super::{a_week, open};

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_left_alone_draws_nothing() {
    let f = open(a_week());
    // The rail on screen, with the Mind able to answer, as on a desktop with the shell up.
    f.app.event(Event::ReachChanged(Reach::Ready));
    f.app.resize(1100, 720);
    f.app.advance(Duration::from_secs(1));
    f.quiet();
    let rest = f.app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    // The minute clock (10) and the store looked at every 20 s (30 looks, each a job, and the
    // rail's refresh after each landing, 30): all off the screen, and none of it draws.
    assert_eq!(rest, AtRest { frames: 0, fired: 70, jobs: 30 });
}
