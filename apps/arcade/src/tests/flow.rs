//! Every action's answer, the window's buttons, and the three browser jobs: each pressed or asked
//! as the person or a mind asks, and read back from the library on disk.

use serde_json::json;
use slint::Model as _;

use super::{meadow_run, open, seeded, Fixture, PIP};
use crate::command::Verb;

fn game_rows(f: &Fixture) -> Vec<String> {
    f.app.ui(|ui| ui.get_games().iter().map(|r| r.title.to_string()).collect())
}

#[test]
fn a_character_is_saved_and_answered_in_the_words_it_always_was() {
    let f = open();
    let answer = f.act("new_character", json!({ "spec": PIP })).unwrap();
    assert_eq!(answer["summary"], json!("saved character \"Pip\""));
    assert_eq!(answer["saved"], json!("saved character \"Pip\""));
    assert_eq!((answer["slug"].clone(), answer["archetype"].clone()), (json!("pip"), json!("critter")));
    assert_eq!(f.library().load_character("pip").unwrap().name, "Pip", "and it is on disk");
    assert_eq!(f.notice(), "saved character \"Pip\"");
    let again = f.act("new_character", json!({ "spec": PIP })).unwrap_err();
    assert!(again.contains("already saved"), "{again}");
}

#[test]
fn a_game_is_saved_built_and_deleted_to_the_trash() {
    let f = open();
    seeded(&f);
    let built = f.act("build", json!({ "game": "Meadow Run" })).unwrap();
    assert_eq!(built["summary"], json!("built meadow-run"));
    let html = f.library().game_html("meadow-run");
    assert!(html.exists(), "the build is on disk");
    assert_eq!(f.state()["games"][0]["built"], json!(true));
    let gone = f.act("delete", json!({ "game": "meadow-run" })).unwrap();
    assert_eq!(gone["deleted"], json!("\"Meadow Run\" moved to the Trash"));
    assert!(!html.exists());
    assert_eq!(f.state()["games"], json!([]));
    assert_eq!(f.state()["selected"], json!(""), "and nothing stays selected");
    let trashed = f.paths.state_dir().join("trash/Trash/files/yantrik-arcade-meadow-run");
    assert!(trashed.join("spec.json").exists(), "the game is in the Trash, whole: {}", trashed.display());
}

#[test]
fn an_update_keeps_the_spec_it_replaces_and_drops_the_build() {
    let f = open();
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let answer = f.act("update_game", json!({ "spec": meadow_run("Meadow Run").replace("\"count\": 5", "\"count\": 7") })).unwrap();
    assert_eq!(answer["saved"], json!("updated game \"Meadow Run\""));
    assert_eq!(answer["built"], json!(false));
    assert!(!f.library().game_html("meadow-run").exists());
    assert!(f.library().game_dir("meadow-run").join("spec.previous.json").exists());
    let stale = f.act("update_character", json!({ "spec": PIP.replace("curl", "none") })).unwrap();
    assert_eq!(stale["needs_build"], json!([]), "nothing was built to go stale");
}

#[test]
fn a_missing_argument_or_an_unknown_game_is_a_sentence() {
    let f = open();
    assert_eq!(
        f.act("new_character", json!({ "spec": "   " })).unwrap_err(),
        "`spec` is required: the character JSON to save"
    );
    assert_eq!(
        f.act("build", json!({ "game": " " })).unwrap_err(),
        "`game` is required: the title or slug of the game, as `describe` lists under `games`"
    );
    let unknown = f.act("build", json!({ "game": "nope" })).unwrap_err();
    assert!(unknown.contains("No game called \"nope\""), "{unknown}");
    assert_eq!(f.notice(), unknown, "and the window says it too");
    let bad = f.act("new_game", json!({ "spec": "{ not json" })).unwrap_err();
    assert!(!bad.is_empty());
}

#[test]
fn the_browser_actions_answer_started_and_the_verdict_lands_in_describe() {
    let f = open();
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let answer = f.start_slow(vec![("verify", json!({ "game": "meadow-run" }))]).remove(0).settle().unwrap();
    assert_eq!(answer["started"], json!(true));
    assert_eq!((answer["action"].clone(), answer["game"].clone()), (json!("verify"), json!("meadow-run")));
    let job = answer["job"].as_i64().unwrap();
    assert_eq!(f.app.jobs_waiting(), 1, "the worker has not answered");
    assert!(f.browser.calls().is_empty());
    assert_eq!(f.state()["jobs"][0]["status"], json!("running"));
    assert_eq!(f.summary(), "Arcade \u{2014} verify of meadow-run running");
    assert_eq!(f.app.ui(|ui| ui.get_status().to_string()), "verify meadow-run running\u{2026}");
    assert!(f.app.ui(|ui| ui.get_busy()), "the buttons are held while it runs");

    f.quiet();
    assert_eq!(f.state()["jobs"][0], json!({ "id": job, "action": "verify", "subject": "meadow-run", "status": "done", "detail": "passed all 7 gates" }));
    assert_eq!(f.notice(), "passed all 7 gates");
    assert!(!f.app.ui(|ui| ui.get_busy()));
    assert_eq!(f.browser.calls(), vec![(Verb::Verify, "meadow-run".to_string())]);
}

#[test]
fn a_failed_verdict_is_a_failed_job() {
    let f = open();
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    *f.browser.verdict.lock().unwrap() = Err("the Browser did not answer".into());
    f.act("play", json!({ "game": "meadow-run" })).unwrap();
    assert_eq!(f.state()["jobs"][0]["status"], json!("failed"));
    assert_eq!(f.state()["jobs"][0]["detail"], json!("the Browser did not answer"));
    assert_eq!(f.notice(), "the Browser did not answer");
}

#[test]
fn a_browser_action_on_an_unbuilt_game_is_refused_before_any_worker_starts() {
    let f = open();
    seeded(&f);
    for action in ["play", "verify", "screenshot"] {
        let refused = f.act(action, json!({ "game": "meadow-run" })).unwrap_err();
        assert_eq!(refused, "meadow-run has not been built yet; `build game=meadow-run` first");
    }
    assert!(f.browser.calls().is_empty());
    assert_eq!(f.state()["jobs"], json!([]));
}

#[test]
fn no_more_than_two_browser_jobs_run_at_once() {
    let f = open();
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let mut both = f.start_slow(vec![("verify", json!({ "game": "meadow-run" })), ("screenshot", json!({ "game": "meadow-run" }))]);
    let (second, first) = (both.pop().unwrap(), both.pop().unwrap());
    // Asked without running the two workers first, as `act` would.
    let refused = f.app.act("play", json!({ "game": "meadow-run" })).expect_err("the third is refused");
    assert!(refused.contains("already running 2 browser jobs"), "{refused}");
    first.settle().unwrap();
    second.settle().unwrap();
    f.quiet();
    f.act("play", json!({ "game": "meadow-run" })).unwrap();
}

#[test]
fn the_buttons_make_the_same_requests_the_mind_does() {
    let f = open();
    f.type_spec(PIP);
    f.press("new-character");
    assert_eq!(f.notice(), "saved character \"Pip\"");
    assert_eq!(f.app.ui(|ui| ui.get_characters().row_count()), 1);
    f.type_spec(&meadow_run("Meadow Run"));
    f.press("new-game");
    assert_eq!(game_rows(&f), vec!["Meadow Run"]);
    assert_eq!(f.app.ui(|ui| ui.get_game_name().to_string()), "meadow-run", "the new game is selected");
    f.press("build");
    assert!(f.library().game_html("meadow-run").exists());
    assert!(f.app.ui(|ui| ui.get_games().row_data(0).unwrap().detail.to_string()).starts_with("built "));
    f.press("delete");
    assert_eq!(game_rows(&f), Vec::<String>::new());
}

#[test]
fn a_button_with_nothing_to_act_on_says_what_is_missing_and_starts_nothing() {
    let f = open();
    f.press("new-game");
    assert_eq!(f.notice(), "`spec` is required: the game JSON to save");
    f.press("build");
    assert_eq!(f.notice(), "`game` is required: the title or slug of the game, as `describe` lists under `games`");
    assert_eq!(f.app.jobs_waiting(), 0);
}

#[test]
fn clicking_a_character_loads_its_spec_into_the_editor() {
    let f = open();
    seeded(&f);
    f.app.ui(|ui| ui.invoke_pick_character("pip".into()));
    f.quiet();
    let text = f.app.ui(|ui| ui.get_spec_text().to_string());
    assert!(text.contains("\"name\": \"Pip\""), "{text}");
    assert_eq!(f.notice(), "");
    f.app.ui(|ui| ui.invoke_pick_character("ghost".into()));
    f.quiet();
    assert!(f.notice().contains("ghost"), "{}", f.notice());
}

#[test]
fn clicking_a_game_selects_it_and_typing_its_title_does_too() {
    let f = open();
    seeded(&f);
    f.act("new_game", json!({ "spec": meadow_run("Second Run") })).unwrap();
    f.app.ui(|ui| ui.invoke_pick_game("meadow-run".into()));
    f.quiet();
    let selected = |f: &Fixture| f.app.ui(|ui| ui.get_games().iter().filter(|r| r.selected).map(|r| r.title.to_string()).collect::<Vec<_>>());
    assert_eq!(selected(&f), vec!["Meadow Run"]);
    assert_eq!(f.state()["selected"], json!("meadow-run"));
    f.app.ui(|ui| {
        ui.set_game_name("Second Run".into());
        ui.invoke_game_edited("Second Run".into());
    });
    f.quiet();
    assert_eq!(selected(&f), vec!["Second Run"]);
}

/// A mind's token reaches the same nine actions and is answered the same way; a delete is the one
/// that asks for a grant first, as its grade says.
#[test]
fn an_agent_is_answered_as_the_person_is() {
    let f = open();
    f.agent("new_character", json!({ "spec": PIP })).unwrap();
    let saved = f.agent("new_game", json!({ "spec": meadow_run("Agent Run") })).unwrap();
    assert_eq!(saved["slug"], json!("agent-run"));
    assert_eq!(f.agent("build", json!({ "game": "agent-run" })).unwrap()["summary"], json!("built agent-run"));
    assert!(f.library().game_html("agent-run").exists());
    let pending = f.app.act_as(yantrik_testing::Call::new("delete", json!({ "game": "agent-run" })).as_agent(super::AGENT).granted()).unwrap();
    let gone = f.settled(pending).unwrap();
    assert_eq!(gone["deleted"], json!("\"Agent Run\" moved to the Trash"));
}

/// Review M2: a game with a browser job under way is not built again, replaced or deleted under
/// it, with a sentence that says why. Done, the same call works.
#[test]
fn a_game_with_a_job_running_is_not_deleted_replaced_or_built_under_it() {
    let f = open();
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let job = f.start_slow(vec![("verify", json!({ "game": "meadow-run" }))]).remove(0);
    // All three asked while the job is under way, so each is judged against it.
    let asked: Vec<_> = [
        ("delete", json!({ "game": "meadow-run" })),
        ("build", json!({ "game": "Meadow Run" })),
        ("update_game", json!({ "spec": meadow_run("Meadow Run") })),
    ]
    .into_iter()
    .map(|(name, args)| (name, f.app.act(name, args).unwrap()))
    .collect();
    f.quiet();
    for (name, pending) in asked {
        let refused = pending.settle().unwrap_err();
        assert_eq!(refused, "verify of meadow-run is running; wait for it to land (`describe` lists it under jobs), then try again", "{name}");
    }
    assert!(f.library().game_html("meadow-run").exists(), "the build is as it was");
    job.settle().unwrap();
    f.act("delete", json!({ "game": "meadow-run" })).unwrap();
}
