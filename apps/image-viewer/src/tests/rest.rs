//! Nothing redraws at rest (the kit's rule), measured as Weather's is (apps/weather/src/
//! rest_tests.rs): the window drawn whenever it asks to be, at the size it opens at, over ten
//! minutes of the virtual clock with nobody touching it; what its timers start runs and lands.

use std::time::Duration;

use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_testing::AtRest;

use super::fixture::{showing, Folder};

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_left_alone_draws_nothing() {
    let folder = Folder::new("rest");
    let app = showing(&folder, "a.png");
    // The rail on screen, with the Mind able to answer, as on a desktop with the shell up.
    app.seen(Seen::Reach(Reach::Ready));
    app.resize(1100, 800);
    app.advance(Duration::from_secs(1));
    app.run_jobs();
    let rest = app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    assert_eq!(rest, AtRest::default(), "nothing runs and nothing draws");
}
