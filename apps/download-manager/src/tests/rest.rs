//! Nothing redraws at rest (the kit's rule, #944 and #949), measured the way the display pays for
//! it. The look at the engine runs only while a download does, and draws only when the engine's
//! revision moved; the list is set only when a row differs.

use std::time::Duration;

use serde_json::json;
use yantrik_app::effects::Effects;
use yantrik_app::rail::Rail;
use yantrik_testing::AtRest;

use super::{body, open, server::File};
use crate::model::Model;
use crate::update::update;
use crate::{Command, Downloads};

const TEN_MINUTES: Duration = Duration::from_secs(10 * 60);

#[test]
fn a_window_with_nothing_running_arms_no_timer_and_draws_nothing() {
    let f = open(vec![("/a.bin", File::plain(body(6_000)))]);
    f.download("/a.bin");
    f.quiet();
    // Past the deadline of the add's own answer (the framework's, not the app's): what is left
    // armed after that is the app's, and there is none.
    f.app.advance(crate::intents::WITHIN + Duration::from_secs(1));
    assert_eq!(f.app.timers_armed(), 0, "no look is owed while nothing moves");
    let rest = f.app.at_rest(TEN_MINUTES, Duration::from_secs(1));
    assert_eq!(rest, AtRest::default(), "ten minutes alone: no frame, no timer, no job");
}

#[test]
fn a_download_that_is_waiting_on_the_network_looks_but_draws_nothing() {
    let f = open(vec![("/held.bin", File::held_at(body(400_000), 60_000))]);
    let id = f.act("add", json!({ "url": f.server.url("/held.bin") })).unwrap()["id"].as_i64().unwrap();
    f.until("it is on disk and waiting", |f| {
        std::fs::metadata(f.default_dir().join("held.bin")).map(|m| m.len() >= 60_000).unwrap_or(false) && f.row(id)["status"] == "downloading"
    });
    // Let the speed window pass once so the numbers have settled to a stalled transfer.
    std::thread::sleep(Duration::from_millis(1_200));
    f.app.advance(crate::update::TICK * 4);
    assert_eq!(f.app.timers_armed(), 1, "the look runs while a download is running");
    let rest = f.app.at_rest(Duration::from_secs(20), Duration::from_millis(250));
    assert_eq!(rest.frames, 0, "a stalled transfer is no new frame: {rest:?}");
    assert_eq!(rest.jobs, 0);
    assert!(rest.fired >= 70, "and the look does fire: {rest:?}");
}

#[test]
fn a_look_at_an_engine_that_has_not_moved_asks_for_nothing() {
    let dir = std::env::temp_dir().join(format!("yantrik-dl-rest-{}", std::process::id()));
    let engine = crate::engine::Engine::open_in(dir.join("list"), dir.join("files"));
    let mut m = Model::new(engine, Rail::default());
    m.seen = m.engine.revision();
    let mut fx = Effects::<Downloads>::recording();
    update(&mut m, Command::Tick, &mut fx).unwrap();
    assert!(fx.changed_parts().is_empty(), "no part redraws: {:?}", fx.changed_parts());
    assert!(fx.requests().is_empty(), "and no timer or job is asked for: {}", fx.requests().len());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_list_set_again_to_the_rows_it_holds_is_not_drawn() {
    let f = open(vec![("/s.bin", File::plain(body(6_000)))]);
    f.download("/s.bin");
    f.app.run(Command::Filter(0));
    f.app.settle_frames();
    f.app.run(Command::Filter(0));
    assert_eq!(f.app.draw(), 0, "the same rows are not a new frame");
}
