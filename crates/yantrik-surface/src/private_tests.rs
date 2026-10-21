//! Private mode's view (#924), with Private mode passed in or pinned per registry: no test here
//! reads the person's file, so none depends on whether the machine running it is private.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_ipc_contracts::control_surface::{Action, Param, View};
use yantrik_ipc_transport::gate::{Authority, Mode};

use crate::private::{private_view, show_problem, Hold, PRIVATE_SUMMARY, RESERVED_KEYS, WITHHELD, WITHHELD_REFUSAL};
use crate::registry::LocalRegistry;

/// In every place an app could put the person's words: the summary, a key, a nested value.
const SENTINEL: &str = "Dentist with Priya at 4pm SENTINEL-924";

fn open() -> Authority {
    Authority { ceiling: "dangerous".into(), mode: Mode::named("bypass_all"), granted: false, asks_above: None }
}

/// A calendar whose every word is the person's, and how many times its describe ran.
fn calendar(reads: Rc<Cell<u32>>) -> LocalRegistry {
    let mut reg = LocalRegistry::new("calendar");
    reg.set_describe(Box::new(move || {
        reads.set(reads.get() + 1);
        View::new(format!("Calendar — next: {SENTINEL}"))
            .with("next_event", SENTINEL)
            .with("events", json!([{ "title": SENTINEL, "at": "16:00" }]))
            .with("units", "24h")
    }));
    reg.add(
        Action::new("rename", "Rename the selected event").arg(Param::text("to")),
        Box::new(|args| Ok(json!({ "renamed_to": args["to"].clone() }))),
    );
    reg.add(Action::new("today", "Answer with today's date").stateless(), Box::new(|_| Ok(json!("2026-10-21"))));
    // What a careless handler answers and refuses with: the person's words.
    reg.add(Action::new("read_next", "Read the next event").stateless(), Box::new(|_| Ok(json!({ "title": SENTINEL }))));
    reg.add(
        Action::new("open_event", "Open an event").arg(Param::text("id")),
        Box::new(|_| Err(format!("no event like that; the open one is {SENTINEL}"))),
    );
    reg
}

fn has_no_sentinel(answer: &Value) {
    let text = answer.to_string();
    assert!(!text.contains("SENTINEL-924") && !text.contains("Priya"), "the person's words leaked: {text}");
}

#[test]
fn with_private_mode_off_describe_is_what_it_always_was() {
    let reg = calendar(Rc::new(Cell::new(0)));
    let answer = reg.describe_in(false);
    assert_eq!(answer["summary"], format!("Calendar — next: {SENTINEL}"));
    assert_eq!(answer["state"]["next_event"], SENTINEL);
    assert!(answer.get("private").is_none(), "no marker while the person is not private");
    assert_eq!(answer["revision"], reg.snapshot().revision());
}

#[test]
fn in_private_mode_describe_answers_with_the_id_the_marker_and_nothing_the_app_holds() {
    let reads = Rc::new(Cell::new(0));
    let reg = calendar(reads.clone());
    let answer = reg.describe_in(true);
    assert_eq!(answer["app"], "calendar");
    assert_eq!(answer["protocol"], 1);
    assert_eq!(answer["private"], true);
    assert_eq!(answer["summary"], PRIVATE_SUMMARY);
    assert_eq!(answer["state"], json!({ "private": true }), "none of the app's own keys");
    has_no_sentinel(&answer);
    // Still there, still drivable by the person: the actions and their grades are the author's.
    let actions: Vec<&str> = answer["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(actions, ["rename", "today", "read_next", "open_event"]);
    // The revision is the private view's, so it says nothing about what changed underneath.
    assert_eq!(answer["revision"], private_view(None, &[], None).revision());
    assert_ne!(answer["revision"], reg.snapshot().revision());
    assert_eq!(reads.get(), 1, "only the snapshot above read the app; the private answer did not");
}

#[test]
fn an_app_shows_only_the_keys_it_listed_and_only_by_name() {
    let mut reg = calendar(Rc::new(Cell::new(0)));
    reg.show_in_private("units");
    reg.show_in_private("units");
    reg.show_in_private("not_a_key_it_has");
    assert_eq!(reg.shown_in_private(), ["units", "not_a_key_it_has"], "listed once each");
    let answer = reg.describe_in(true);
    assert_eq!(answer["state"], json!({ "private": true, "units": "24h" }));
    has_no_sentinel(&answer);
}

#[test]
fn the_private_views_own_keys_cannot_be_listed_by_an_app() {
    let mut reg = calendar(Rc::new(Cell::new(0)));
    for key in RESERVED_KEYS.iter().copied().chain(["", "  "]) {
        assert!(show_problem(key).is_some(), "{key:?}");
        reg.show_in_private(key);
    }
    assert!(reg.shown_in_private().is_empty());
    assert!(show_problem("units").is_none());
}

#[test]
fn the_window_facts_come_from_presence_and_nothing_else_of_it_does() {
    let mut reg = calendar(Rc::new(Cell::new(0)));
    reg.set_presence(Box::new(|| {
        View::new(SENTINEL).with("open", true).with("windows_open", 2).with("title", SENTINEL)
    }));
    let answer = reg.describe_in(true);
    assert_eq!(answer["state"], json!({ "private": true, "open": true, "windows_open": 2 }));
    assert_eq!(answer["summary"], PRIVATE_SUMMARY);
    has_no_sentinel(&answer);
    // And while the person is not private, presence adds nothing to the app's own answer.
    assert!(reg.describe_in(false)["state"].get("windows_open").is_none());
}

#[test]
fn an_act_in_private_mode_replies_with_the_private_view() {
    let reg = calendar(Rc::new(Cell::new(0)));
    let reply = reg.act_in("rename", &json!({ "to": "x" }), None, "t#1", &open(), Hold::Withheld).unwrap();
    assert_eq!(reply["accepted"], true);
    assert_eq!(reply["private"], true);
    assert_eq!(reply["summary"], PRIVATE_SUMMARY);
    assert_eq!(reply["state"], json!({ "private": true }));
    assert_eq!(reply["revision"], reg.describe_in(true)["revision"]);
    has_no_sentinel(&reply);
    // A stateless reply carries no state either way, and is still marked.
    let today = reg.act_in("today", &json!({}), None, "t#2", &open(), Hold::Withheld).unwrap();
    assert!(today.get("state").is_none() && today.get("summary").is_none());
    assert_eq!(today["private"], true);
    // Off, the reply is the app's own, unmarked.
    let off = reg.act_in("rename", &json!({ "to": "x" }), None, "t#3", &open(), Hold::Open).unwrap();
    assert_eq!(off["state"]["next_event"], SENTINEL);
    assert!(off.get("private").is_none());
}

#[test]
fn the_guard_in_private_mode_holds_a_call_to_what_its_caller_could_see_and_says_no_more() {
    let reg = calendar(Rc::new(Cell::new(0)));
    let real = reg.snapshot().revision();
    let shown = reg.describe_in(true)["revision"].as_str().unwrap().to_string();
    // The real revision is not something a private caller could have read, and taking it would
    // make the guard an oracle on the app's contents.
    let stale = reg.act_in("rename", &json!({ "to": "x" }), Some(&real), "t#1", &open(), Hold::Withheld).unwrap_err();
    assert!(stale.starts_with("STALE:"), "{stale}");
    assert!(stale.contains(PRIVATE_SUMMARY), "{stale}");
    assert!(!stale.contains("SENTINEL-924"), "the refusal leaked the summary: {stale}");
    assert!(reg.act_in("rename", &json!({ "to": "x" }), Some(&shown), "t#2", &open(), Hold::Withheld).is_ok());
}

/// #926 review, M1: in Private mode a caller nobody vouches for is told the action ran, or was
/// refused, and nothing it said; the shell, the app's own program and the listed desktop programs
/// (`Hold::Vouched`) get its own answer and refusal, beside the private view.
#[test]
fn an_unvouched_caller_gets_no_result_and_no_refusal_text_and_a_vouched_one_gets_both() {
    let reg = calendar(Rc::new(Cell::new(0)));
    let withheld = reg.act_in("read_next", &json!({}), None, "t#1", &open(), Hold::Withheld).unwrap();
    assert_eq!(withheld["accepted"], true);
    assert_eq!(withheld["result"], Value::Null);
    assert_eq!(withheld["withheld"], WITHHELD);
    assert_eq!(withheld["private"], true);
    has_no_sentinel(&withheld);
    let refused = reg.act_in("open_event", &json!({ "id": "e1" }), None, "t#2", &open(), Hold::Withheld).unwrap_err();
    assert_eq!(refused, WITHHELD_REFUSAL);
    // The dispatch's own refusals are its words, not the app's, and stay as they were.
    let unknown = reg.act_in("nope", &json!({}), None, "t#3", &open(), Hold::Withheld).unwrap_err();
    assert!(unknown.starts_with("unknown action `nope`"), "{unknown}");

    let vouched = reg.act_in("read_next", &json!({}), None, "t#4", &open(), Hold::Vouched).unwrap();
    assert_eq!(vouched["result"]["title"], SENTINEL, "the shell, the app's own program, yantrik-term");
    assert_eq!(vouched["private"], true);
    assert!(vouched.get("withheld").is_none());
    let why = reg.act_in("open_event", &json!({ "id": "e1" }), None, "t#5", &open(), Hold::Vouched).unwrap_err();
    assert!(why.contains(SENTINEL), "{why}");

    assert_eq!((Hold::of(false, true), Hold::of(true, true), Hold::of(true, false)), (Hold::Open, Hold::Vouched, Hold::Withheld));
    assert_eq!(Hold::for_caller(false, None), Hold::Open);
    assert_eq!(Hold::for_caller(true, None), Hold::Withheld, "no pid is nobody's");
    assert_eq!(Hold::for_caller(true, Some(std::process::id() as i32)), Hold::Withheld, "a test binary is not installed");
}

#[test]
fn an_in_process_read_asks_the_registrys_source_on_every_call() {
    let private = Arc::new(AtomicBool::new(false));
    let mut reg = calendar(Rc::new(Cell::new(0)));
    reg.set_privacy_source(Box::new({
        let private = private.clone();
        move || private.load(Ordering::SeqCst)
    }));
    assert_eq!(reg.describe()["state"]["next_event"], SENTINEL);
    private.store(true, Ordering::SeqCst);
    let now = reg.describe();
    assert_eq!(now["private"], true, "the next call, with nothing to restart or refresh");
    has_no_sentinel(&now);
    has_no_sentinel(&reg.act("rename", &json!({ "to": "x" }), None, "t#1", &open()).unwrap());
    private.store(false, Ordering::SeqCst);
    assert_eq!(reg.describe()["state"]["next_event"], SENTINEL, "and back, on the next call");
}
