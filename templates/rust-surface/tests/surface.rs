//! The surface in-process: what it publishes, what each action does, and what the dispatch refuses
//! before a handler runs. No socket and no files: each call is made under an `Authority` the test
//! pins — a ceiling and a mode — so the machine running the tests lends it neither.
//!
//! Keep these when you rename the template, and add one for each action you add.

use std::sync::{Arc, Mutex};

use my_surface::{surface_over, Tasks, APP_ID};
use yantrik_surface::gate::{Authority, Mode};
use yantrik_surface::serde_json::{json, Value};
use yantrik_surface::Surface;

/// The machine this OS ships: a `sensitive` ceiling, in `mode`.
fn under(mode: &str) -> Authority {
    Authority { ceiling: "sensitive".into(), mode: Mode::named(mode), granted: false, asks_above: None }
}

fn fresh() -> (Surface, Arc<Mutex<Tasks>>) {
    let tasks = Arc::new(Mutex::new(Tasks::default()));
    (surface_over(tasks.clone()), tasks)
}

fn act(surface: &Surface, mode: &str, action: &str, args: Value) -> Result<Value, String> {
    surface
        .act(&json!({ "action": action, "args": args }), None, under(mode))
        .map_err(|e| {
            assert_eq!(e.code, -32602, "every refusal of an act is -32602: {}", e.message);
            e.message
        })
}

#[test]
fn every_declaration_is_one_the_dispatch_can_enforce() {
    let (surface, _) = fresh();
    assert_eq!(surface.registry().problems(), Vec::<String>::new());
}

#[test]
fn describe_publishes_the_list_and_every_action_graded() {
    let (surface, _) = fresh();
    let described = surface.describe_json();
    assert_eq!(described["app"], APP_ID);
    assert_eq!(described["protocol"], 1);
    assert_eq!(described["summary"], "My Surface — 0 tasks, 0 done");
    let grades: Vec<(String, String)> = described["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["name"].as_str().unwrap().to_string(), a["permission"].as_str().unwrap().to_string()))
        .collect();
    let expected = [("add", "standard"), ("complete", "standard"), ("find", "safe"), ("remove", "sensitive")];
    assert_eq!(grades, expected.map(|(a, g)| (a.to_string(), g.to_string())));
    let add = &described["actions"][0]["parameters"];
    assert_eq!(add["required"], json!(["title"]));
    assert_eq!(add["properties"]["priority"]["enum"], json!(["low", "normal", "high"]));
    assert_eq!(add["properties"]["priority"]["default"], "normal");
}

#[test]
fn add_complete_and_find_do_what_they_say() {
    let (surface, tasks) = fresh();
    let reply = act(&surface, "ask", "add", json!({ "title": "Water the plants", "priority": "high" })).unwrap();
    assert_eq!(reply["accepted"], true);
    assert_eq!(reply["settled"], true);
    assert_eq!(reply["summary"], "My Surface — 1 task, 0 done", "the answer carries the view after the act");

    act(&surface, "ask", "add", json!({ "title": "Call the plumber" })).unwrap();
    assert_eq!(lock_items(&tasks)[1].priority, "normal", "the declared default reached the handler");

    act(&surface, "ask", "complete", json!({ "index": 0 })).unwrap();
    let found = act(&surface, "plan", "find", json!({ "query": "PLANTS" })).unwrap();
    assert_eq!(found["result"]["matches"], json!([{ "index": 0, "title": "Water the plants", "done": true }]));

    // `done=false` takes it back: that is why `complete` is `standard`, not more.
    act(&surface, "ask", "complete", json!({ "index": 0, "done": false })).unwrap();
    assert!(!lock_items(&tasks)[0].done);
}

#[test]
fn a_call_its_own_arguments_refuse_never_reaches_the_handler() {
    let (surface, tasks) = fresh();
    assert_eq!(act(&surface, "ask", "add", json!({})), Err("`add` needs argument `title`".into()));
    assert_eq!(
        act(&surface, "ask", "add", json!({ "title": "x", "priority": "urgent" })),
        Err("`add` argument `priority` must be one of `low`, `normal`, `high`, and another string arrived".into())
    );
    assert_eq!(
        act(&surface, "ask", "complete", json!({ "index": "first" })),
        Err("`complete` argument `index` must be an integer, and a string arrived".into())
    );
    assert_eq!(
        act(&surface, "ask", "add", json!({ "title": "x", "due": "friday" })),
        Err("`add` has no argument `due`; it takes: title, priority".into())
    );
    assert!(lock_items(&tasks).is_empty());

    // What converts without loss is converted: `"0"` for an integer is 0.
    act(&surface, "ask", "add", json!({ "title": "x" })).unwrap();
    act(&surface, "ask", "complete", json!({ "index": "0" })).unwrap();
    assert!(lock_items(&tasks)[0].done);

    // The handler's own refusal, in its own words.
    assert_eq!(
        act(&surface, "ask", "complete", json!({ "index": 5 })),
        Err("there is no task 5; the list has 1, indexed 0 to 0".into())
    );
}

#[test]
fn remove_waits_for_the_person_in_ask_and_in_auto() {
    let (surface, tasks) = fresh();
    act(&surface, "ask", "add", json!({ "title": "Water the plants" })).unwrap();

    let refused = act(&surface, "ask", "remove", json!({ "index": 0 })).unwrap_err();
    assert!(refused.starts_with("GRANT: my-surface.remove is graded `sensitive`"), "{refused}");
    // Auto runs `sensitive` unasked — but not what its own description says cannot be undone.
    let refused = act(&surface, "auto", "remove", json!({ "index": 0 })).unwrap_err();
    assert!(refused.contains("its own description says it cannot be undone"), "{refused}");
    assert_eq!(lock_items(&tasks).len(), 1, "nothing was removed");

    // Bypass still asks before what cannot be undone; full bypass runs everything under the
    // ceiling.
    let refused = act(&surface, "bypass", "remove", json!({ "index": 0 })).unwrap_err();
    assert!(refused.contains("its own description says it cannot be undone"), "{refused}");
    let reply = act(&surface, "bypass_all", "remove", json!({ "index": 0 })).unwrap();
    assert_eq!(reply["result"], json!({ "removed": "Water the plants", "left": 0 }));
}

#[test]
fn an_act_decided_on_a_view_that_moved_is_refused_as_stale() {
    let (surface, _) = fresh();
    let read = surface.describe_json()["revision"].as_str().unwrap().to_string();
    act(&surface, "ask", "add", json!({ "title": "Something else happened first" })).unwrap();
    let refused = surface
        .act(&json!({ "action": "add", "args": { "title": "x" }, "expect_revision": read }), None, under("ask"))
        .unwrap_err()
        .message;
    assert!(refused.starts_with("STALE: this app is at revision "), "{refused}");
}

#[test]
fn the_desktop_file_declares_this_surface() {
    // Rename the template and forget the .desktop file, and this is the test that says so.
    let desktop = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/my-surface.desktop")).unwrap();
    let key = |k: &str| {
        desktop.lines().find_map(|l| l.strip_prefix(k).and_then(|rest| rest.strip_prefix('='))).map(str::trim)
    };
    assert_eq!(key("X-Yantrik-Surface"), Some(APP_ID));
    assert!(key("X-Yantrik-Purpose").is_some_and(|p| !p.is_empty()));
    assert_eq!(key("Exec"), Some("my-surface"), "the program Cargo builds, found on PATH or in /opt/yantrik/bin");
}

fn lock_items(tasks: &Mutex<Tasks>) -> Vec<my_surface::Task> {
    tasks.lock().unwrap().items.clone()
}
