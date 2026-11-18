//! Nothing redraws at rest (the kit's rule), measured as Weather's is (apps/weather/src/
//! rest_tests.rs): the window drawn whenever it asks to be, at the size it opens at, over ten
//! minutes of the virtual clock with nobody touching it; what its timers start runs and lands.

use std::time::Duration;

use yantrik_testing::AtRest;

use super::{quiet, started, with_inbox};

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_left_alone_draws_nothing() {
    let app = started(&with_inbox());
    app.resize(1200, 800);
    app.advance(Duration::from_secs(1));
    quiet(&app);
    let rest = app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    assert_eq!(rest, AtRest::default(), "nothing runs and nothing draws");
}
