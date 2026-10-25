//! Nothing redraws at rest (the kit's rule), measured the way the display pays for it: the
//! window is drawn whenever it asks to be, over ten minutes of the virtual clock with nobody
//! touching it, at the size it opens at, with the reading in and the rail on screen.
//!
//! On 520 the window at rest cost 42–61 ticks per 30 s, about half of them in the software
//! rasteriser. Measured here on main (d2d5074d): 170 frames and 190 timer fires in ten minutes.
//! The rail's four-second fallback tick set its rows to new models each time, which is a new
//! frame even when every row is the same (150 fires, 150 frames); the 30 s "updated" tick (20
//! fires, its own words unchanged) asked the rail to refresh 300 ms after it (20 fires, 20
//! frames). The words that say the age change once a minute at most.

use std::time::Duration;

use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_services::weather::FakeWeather;
use yantrik_testing::AtRest;

use crate::tests::{forecast_at, start};
use crate::UPDATED_TICK;

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_left_alone_draws_once_a_minute_and_only_when_its_words_change() {
    let (app, _, mind) = start(FakeWeather::new().forecasting(Ok(forecast_at(12.0))));
    mind.set_reach(Reach::Ready);
    app.seen(Seen::Reach(Reach::Ready));
    app.run_jobs();
    app.resize(1000, 720);
    assert!(app.ui(|ui| ui.get_rail_shown()), "the rail is on screen at the size the window opens at");
    // The refresh a landed reading asks of the rail, then nothing more is owed.
    app.advance(Duration::from_secs(1));
    assert_eq!(app.model(|m| m.rail.suggestions().len()), 1, "the rail has its question");

    // Ten minutes: the rail's "Updated N min ago" changes on each of the reading's minutes, and
    // that is all that changes. The corner says nothing under fifteen minutes.
    let rest = app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    assert_eq!(rest, AtRest { frames: 10, fired: 10, jobs: 0 }, "one look and one frame a minute, nothing else");
    assert_eq!(app.model(|m| m.rail.context()[1].label.clone()), "Updated 10 min ago");
    // Between two of its minutes the window asks for nothing at all.
    assert_eq!(app.at_rest(Duration::from_secs(58), Duration::from_secs(1)), AtRest::default());
}

#[test]
fn the_age_is_looked_at_when_its_words_change_and_not_between() {
    use crate::model::next_age_change;
    let s = Duration::from_secs;
    // "Updated just now" until the first minute, then each whole minute of the reading's age.
    assert_eq!(next_age_change(s(0)), s(60));
    assert_eq!(next_age_change(s(59)), s(1));
    assert_eq!(next_age_change(s(60)), s(60));
    assert_eq!(next_age_change(s(61)), s(59));
    // From the hour the words change by the hour.
    assert_eq!(next_age_change(s(59 * 60 + 30)), s(30));
    assert_eq!(next_age_change(s(70 * 60)), s(50 * 60));
    // The timer is armed under the app's own name, and only one of it.
    assert_eq!(UPDATED_TICK.0, "weather.updated");
}

/// A look at the age redraws only the words that changed, in one command, and arms itself once
/// for the next change: the rail's row from the first minute, the corner's line from fifteen.
#[test]
fn a_look_at_the_age_draws_only_the_words_that_changed() {
    use std::time::Instant;
    use yantrik_app::effects::Effects;
    use yantrik_app::rail;

    use crate::model::{Model, PART_READING, PART_UPDATED};
    use crate::update::update;
    use crate::{Command, Weather};

    let s = Duration::from_secs;
    let look = |m: &mut Model| {
        let mut fx = Effects::<Weather>::recording();
        update(m, Command::TickUpdated, &mut fx).unwrap();
        let armed: Vec<&str> = fx.requests().iter().map(|r| r.name()).collect();
        (fx.changed_parts().clone(), armed)
    };
    let t0 = Instant::now();
    let mut m = Model::for_tests();
    // Two minutes old, last looked at a minute ago.
    m.last_fetch = Some(t0 - s(125));
    m.now = t0 - s(60);
    let (changed, armed) = look(&mut m);
    assert!(changed.has(rail::PART), "{changed:?}");
    assert!(!changed.has(PART_UPDATED) && !changed.has(PART_READING), "the corner says nothing yet: {changed:?}");
    assert_eq!(armed, ["after"], "one shot for the next change, not a period");
    assert_eq!(m.rail.context()[0].label, "Updated 2 min ago");
    // Looked at again straight away: the same words, so nothing redraws.
    let (changed, armed) = look(&mut m);
    assert!(changed.is_empty(), "{changed:?}");
    assert_eq!(armed, ["after"]);
    // Fifteen minutes: the corner begins to say the age, with the rail's row, in one command.
    m.last_fetch = Some(t0 - s(15 * 60) - Duration::from_millis(500));
    m.now = t0 - s(2);
    let (changed, _) = look(&mut m);
    assert!(changed.has(PART_UPDATED) && changed.has(rail::PART), "{changed:?}");
    assert_eq!(m.freshness().line, "Updated 15 min ago");
}
