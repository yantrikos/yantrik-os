//! Nothing redraws at rest (the kit's rule), measured as Weather's is (apps/weather/src/
//! rest_tests.rs): the window drawn whenever it asks to be, at the size it opens at, over ten
//! minutes of the virtual clock with nobody touching it; what its timers start runs and lands.

use std::time::Duration;

use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_testing::AtRest;

use crate::tests::{on_wifi, read, run_until_quiet};

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_left_alone_draws_nothing() {
    let app = read(&on_wifi());
    // The rail on screen, with the Mind able to answer, as on a desktop with the shell up.
    app.seen(Seen::Reach(Reach::Ready));
    app.resize(1100, 750);
    app.advance(Duration::from_secs(1));
    run_until_quiet(&app);
    let rest = app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    // The network is read every three seconds off the window's thread (200 reads, each a job,
    // and the rail's refresh after each landing, 200), and the same network read again draws
    // nothing.
    assert_eq!(rest, AtRest { frames: 0, fired: 400, jobs: 200 });
}
