//! "Say it once" (#1183, #1184): said on a Monday, prepared when Thursday's prep window opens.
//!
//! The clock is a number the test moves. The tools are fakes that stand in for the disk and for the
//! apps, registered in a real `ToolRegistry`, so every step goes through `ToolRegistry::execute`
//! and is written to the audit exactly as in the shell. The rules around it (what is not a promise,
//! consent, ambiguity, retries, where a window lands) are in `commitment_prep_rules.rs`.

mod common;

use common::*;
use serde_json::json;
use yantrik_companion::anticipation::{AnticipationEngine, PrepOutcome, PreparationTrigger};
use yantrik_companion::commitment_prep::Outcome;
use yantrik_companion::world_model::WorldModel;

#[test]
fn said_on_monday_prepared_when_the_window_opens_and_nothing_is_sent() {
    let rig = Rig::new(&[DECK, SHEET, "/home/p/Documents/notes.md"]);

    // Day 1: said once. Stored with who it is for, when, and the files it names.
    assert_eq!(rig.say(SAID, MONDAY), 1);
    let stored = rig.only_commitment();
    assert_eq!(stored.promisee, "Maya");
    assert_eq!(stored.deadline, thursday());
    assert_eq!(stored.related_entities, vec!["attach:Q4 deck", "file:budget spreadsheet"]);

    // Before the window: nothing is asked and nothing is done.
    for now in [MONDAY, MONDAY + DAY] {
        assert!(rig.cycle(now).is_empty(), "too early at {now}");
    }
    assert!(rig.calls().is_empty());

    // The window opens (Thursday 17:00 is 23 hours off).
    let outcomes = rig.cycle(window());
    let item = the_item(&outcomes);

    // Nothing is opened for the person (the shell refuses a Mind a window of a framework app):
    // the files are found, and the one app call is the draft.
    assert_eq!(rig.app_calls(), [("email".to_string(), "compose".to_string())]);
    let compose = rig.calls().into_iter().find(|(_, a)| a["action"] == "compose").unwrap().1;
    assert_eq!(compose["args"]["attach"], DECK, "the draft carries the deck");
    assert_eq!(compose["args"]["body"], "Hi Maya,\n\nHere is the Q4 deck.\n");

    // Nothing is sent, and nothing reaches the web: only the two local tools were called.
    assert!(rig.calls().iter().all(|(tool, _)| tool == "glob" || tool == "app_action"));
    assert!(rig.app_calls().iter().all(|(_, action)| !action.contains("send")));
    assert_eq!(rig.world.lock().unwrap().web_calls, 0);

    // The item keeps its result: paths and words, no tool output.
    assert_eq!(item.trigger, PreparationTrigger::CommitmentDeadline { commitment_id: stored.id.to_string() });
    assert_eq!(item.data["state"], "prepared");
    assert_eq!(item.data["sent"], false);
    assert_eq!(item.data["attachment"]["path"], DECK);
    assert_eq!(item.data["draft"]["attached"], DECK);
    assert_eq!(item.data["draft"]["can_be_sent"], false, "Email cannot send attachments yet");
    assert_eq!(item.data["files"][0]["path"], DECK);
    assert_eq!(item.data["files"][1]["path"], SHEET);
    assert!(item.data.get("opened").is_none());
    assert_eq!(item.data["missing"], json!([]));
    assert!(item.data.get("steps").is_none() && !item.data.to_string().contains("file(s) found"));
    // It does not claim a send card or a draft id.
    assert!(item.data.get("draft_id").is_none() && item.data.get("card").is_none());
    assert!(item.description.contains("Email can't send attachments yet, so it can't be sent from there"), "{}", item.description);
    assert!(item.description.ends_with("Nothing sent or opened for you."), "{}", item.description);
    assert!(item.description.starts_with("For your promise to Maya on Thursday: 2 files found"), "{}", item.description);

    // Every step is in the audit.
    assert_eq!(rig.audit_lines("app_action"), 1);
    assert!(rig.audit_lines("glob") >= 2);

    // The brief shows it, once, and the person's use of it is recorded.
    let thursday_9 = thursday() - 8.0 * HOUR;
    let relevant = AnticipationEngine::get_relevant_at(&rig.db.conn(), thursday_9);
    assert_eq!(relevant.len(), 1);
    AnticipationEngine::mark_surfaced(&rig.db.conn(), &relevant[0].id);
    AnticipationEngine::record_outcome(&rig.db.conn(), &relevant[0].id, PrepOutcome::Useful);
    assert!(AnticipationEngine::get_relevant_at(&rig.db.conn(), thursday_9).is_empty());
    let kept = rig.item_for(stored.id);
    assert!(kept.surfaced);
    assert_eq!(kept.outcome, Some(PrepOutcome::Useful));
    assert_eq!(kept.data["draft"]["attached"], DECK, "the result survives being surfaced");
}

#[test]
fn nothing_said_nothing_done() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say("It is a fine morning and the coffee is hot.", MONDAY);
    assert!(rig.cycle(window()).is_empty());
    assert!(rig.calls().is_empty());
    assert!(AnticipationEngine::get_relevant_at(&rig.db.conn(), thursday()).is_empty());
}

#[test]
fn a_file_that_cannot_be_found_is_said_in_the_persons_words() {
    // The deck is there; the spreadsheet is not.
    let rig = Rig::new(&[DECK]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    let item = the_item(&outcomes);
    assert_eq!(item.data["missing"], json!(["couldn't find the budget spreadsheet"]));
    assert!(item.description.contains("couldn't find the budget spreadsheet"), "{}", item.description);
    assert_eq!(item.data["files"].as_array().unwrap().len(), 1, "only what exists was found");
    assert_eq!(item.data["draft"]["attached"], DECK, "the deck was found, so the draft is real");
    assert_eq!(item.data["state"], "retry", "it may be saved later, while the window is open");

    // The deck is the thing being sent: without it there is no draft, which would pretend.
    let rig = Rig::new(&[SHEET]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    let item = the_item(&outcomes);
    assert_eq!(item.data["draft"], serde_json::Value::Null);
    assert_eq!(item.data["attachment"], serde_json::Value::Null);
    assert_eq!(item.data["missing"][0], "couldn't find the Q4 deck");
    assert_eq!(item.data["state"], "retry", "the deck may still be saved while the window is open");
    // Once the window has closed, the missing deck is final, and the draft is said not to exist.
    let id = rig.only_commitment().id;
    let closed = rig.prepare(id, thursday() + HOUR + 1e6);
    let Outcome::Prepared(done) = closed else { panic!("{closed:?}") };
    assert_eq!(done.data["state"], "prepared");
    assert_eq!(done.data["missing"], json!(["couldn't find the Q4 deck", "no email was drafted, because there is nothing to attach"]));
    assert!(rig.app_calls().iter().all(|(app, _)| app != "email"), "no draft was made");
}

#[test]
fn it_needs_no_research_and_calls_no_web_tool() {
    // A web tool is registered, and would answer; the preparation never reaches for it, and every
    // call it makes is one of the two local tools on a local app.
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    assert_eq!(the_item(&outcomes).data["draft"]["state"], "draft");
    assert_eq!(rig.world.lock().unwrap().web_calls, 0);
    let tools: std::collections::BTreeSet<_> = rig.calls().into_iter().map(|(t, _)| t).collect();
    assert_eq!(tools, ["app_action", "glob"].iter().map(|s| s.to_string()).collect());
    for (app, _) in rig.app_calls() {
        assert!(["shell", "presentation", "spreadsheet", "document-editor", "email"].contains(&app.as_str()), "{app}");
    }
}

#[test]
fn the_same_commitment_is_not_prepared_twice() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    assert_eq!(rig.cycle(window()).len(), 1);
    let calls_after_first = rig.calls().len();

    // The next think cycles, and a second run after the item was shown: the same answer, no work.
    for later in [window() + 60.0, window() + 12.0 * HOUR] {
        let outcomes = rig.cycle(later);
        assert!(matches!(outcomes.as_slice(), [Outcome::AlreadyPrepared]), "{outcomes:?}");
    }
    let id = WorldModel::active_commitments(&rig.db.conn())[0].id;
    AnticipationEngine::mark_surfaced(&rig.db.conn(), &format!("commitment-prep-{id}"));
    assert!(matches!(rig.prepare(id, window() + DAY), Outcome::AlreadyPrepared));
    assert_eq!(rig.calls().len(), calls_after_first, "nothing was opened or drafted again");
    assert_eq!(rig.items(), 1);
}
