//! Who may say yes to preparing promised things, and what a forgotten promise leaves behind
//! (#1184, review of 40fd3b3e: HIGH-B, MEDIUM-C and the clearing half of HIGH-A).

mod common;

use common::*;
use serde_json::json;
use yantrik_companion::commitment_prep::Outcome;
use yantrik_companion::tools::commitment_prep_tools;
use yantrik_companion::{commitment_consent, commitment_slot, commitment_store};
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::ToolContext;

/// A rig with the real tools registered beside the fakes, whose person has not been asked.
fn rig_with_tools(files: &[&str]) -> Rig {
    let mut rig = Rig::asked(files);
    commitment_prep_tools::register(&mut rig.registry);
    rig
}

fn call(rig: &Rig, tool: &str, args: serde_json::Value) -> String {
    let ctx = ToolContext {
        db: &rig.db,
        max_permission: PermissionLevel::Standard,
        registry_metadata: None,
        task_manager: None,
        incognito: false,
        agent_spawner: None,
    };
    rig.registry.execute(&ctx, tool, &args)
}

// ── HIGH-B: the yes is the person's ─────────────────────────────────

#[test]
fn a_tool_call_of_yes_or_on_leaves_consent_off() {
    let rig = rig_with_tools(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    assert!(matches!(rig.cycle(window()).as_slice(), [Outcome::Prepared(_)]));
    // An email body told a model to do this; a recipe step is the same registry call at Standard.
    for choice in ["yes", "on", "YES"] {
        let said = call(&rig, "commitment_prep_setting", json!({ "choice": choice }));
        assert!(said.starts_with("Error"), "{said}");
    }
    assert!(!commitment_store::consented(&rig.db.conn()));
    assert!(commitment_store::answered(&rig.db.conn()).is_none());
    // And nothing was opened or drafted on the next cycle.
    assert!(matches!(rig.cycle(window() + HOUR).as_slice(), [Outcome::AlreadyPrepared]));
    assert!(rig.calls().is_empty(), "{:?}", rig.calls());
}

#[test]
fn no_tool_names_the_person_only_answer() {
    // The tools directory must never reach `commitment_consent`: its answer is a click, wired by the
    // shell, and not something a model or a recipe step can call.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools");
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let src = std::fs::read_to_string(entry.path()).unwrap();
        for needle in ["commitment_consent", "person_answer", "record_answer"] {
            assert!(!src.contains(needle) || entry.file_name() == "commitment_prep_tools.rs" && !src.contains("person_answer(") && !src.contains("record_answer("), "{} names {needle}", entry.path().display());
        }
    }
    // Refusing is the one thing the tool does: it is safe for a model to say no.
    let rig = rig_with_tools(&[DECK]);
    rig.person_turns_it_on();
    assert!(call(&rig, "commitment_prep_setting", json!({ "choice": "off" })).starts_with("Okay"));
    assert!(!commitment_store::enabled(&rig.db.conn()));
}

#[test]
fn only_the_person_only_path_gives_the_yes_and_it_records_who_and_which_item() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let item = format!("commitment-prep-{}", rig.only_commitment().id);
    let at = window() + 60.0;
    // No question has been shown yet: a yes (a stale card, a guess at the id) is ignored.
    assert!(commitment_consent::person_answer(&rig.db.conn(), &format!("{}{item}", commitment_consent::YES)).is_none());
    assert!(!commitment_store::consented(&rig.db.conn()));
    // Shown, a yes for another item is ignored too.
    let shown = commitment_consent::brief_actions(&rig.db.conn(), at);
    assert_eq!(shown.len(), 2, "a yes and a no: {shown:?}");
    assert!(commitment_consent::person_answer(&rig.db.conn(), &format!("{}commitment-prep-999", commitment_consent::YES)).is_none());
    assert!(!commitment_store::consented(&rig.db.conn()));
    // The button on the item answers it.
    assert!(commitment_consent::person_answer(&rig.db.conn(), &shown[0].action_id).is_some());
    assert!(commitment_store::consented(&rig.db.conn()));
    assert_eq!(commitment_store::answered(&rig.db.conn()), Some(("person".into(), item)));
    // And a repeated click, with no question outstanding, changes nothing.
    assert!(commitment_consent::person_answer(&rig.db.conn(), &shown[1].action_id).is_none());
    assert!(commitment_store::consented(&rig.db.conn()));
}

// ── MEDIUM-C: asked when shown; off then on ─────────────────────────

#[test]
fn the_question_is_asked_when_it_is_shown_not_when_it_is_stored() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    assert!(!commitment_store::asked(&rig.db.conn()));
    // The item expires before the brief ever shows it: nobody was asked.
    assert!(commitment_consent::brief_actions(&rig.db.conn(), thursday() + 10.0 * DAY).is_empty());
    assert!(!commitment_store::asked(&rig.db.conn()));
    // Shown while it is live: asked, and the question is outstanding for that item.
    assert_eq!(commitment_consent::brief_actions(&rig.db.conn(), window() + 60.0).len(), 2);
    assert!(commitment_store::asked(&rig.db.conn()));
    assert!(commitment_store::asking(&rig.db.conn()).is_some());
}

#[test]
fn off_then_on_by_the_person_prepares_again() {
    let rig = rig_with_tools(&[DECK, SHEET]);
    rig.person_turns_it_on();
    rig.say(SAID, MONDAY);
    // The person says "turn that off".
    call(&rig, "commitment_prep_setting", json!({ "choice": "off" }));
    assert!(matches!(rig.cycle(window()).as_slice(), [Outcome::Nothing(_)]));
    // Saying "turn it back on" to the model does nothing, and says so.
    assert!(call(&rig, "commitment_prep_setting", json!({ "choice": "on" })).starts_with("Error"));
    assert!(matches!(rig.cycle(window() + 60.0).as_slice(), [Outcome::Nothing(_)]));
    // The brief offers the way back; pressing it is the person-only act, and is consent.
    let shown = commitment_consent::brief_actions(&rig.db.conn(), window());
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].action_id, commitment_consent::TURN_ON);
    assert_eq!(commitment_consent::person_answer(&rig.db.conn(), &shown[0].action_id).as_deref(), Some("Getting promised things ready is on."));
    let outcomes = rig.cycle(window() + 120.0);
    assert_eq!(the_item(&outcomes).data["state"], "prepared", "it really prepares again");
    assert!(rig.app_calls().contains(&("email".into(), "compose".into())));
}

// ── HIGH-A: forgetting a promise clears the draft it left in Email ──

fn prepared_rig() -> (Rig, i64) {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let id = rig.only_commitment().id;
    rig.cycle(window());
    let compose = rig.calls().into_iter().find(|(_, a)| a["action"] == "compose").unwrap().1;
    assert_eq!(compose["args"]["commitment"], id.to_string(), "Email is told which promise the draft is for");
    (rig, id)
}

fn pay(rig: &Rig) -> usize {
    let ctx = ToolContext { db: &rig.db, max_permission: PermissionLevel::Standard, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    commitment_slot::pay(&rig.registry, &ctx, std::time::Duration::from_secs(60))
}

fn forgets(rig: &Rig) -> Vec<serde_json::Value> {
    rig.calls().into_iter().filter(|(_, a)| a["action"] == "forget_prepared").map(|(_, a)| a["args"].clone()).collect()
}

#[test]
fn forgetting_a_promise_clears_its_draft_in_email() {
    let (rig, id) = prepared_rig();
    assert!(commitment_store::forget(&rig.db.conn(), id));
    assert_eq!(commitment_slot::owed(&rig.db.conn()), vec![id.to_string()]);
    assert_eq!(pay(&rig), 1);
    assert_eq!(forgets(&rig), vec![json!({ "commitment": id.to_string() })]);
    assert!(commitment_slot::owed(&rig.db.conn()).is_empty());
}

#[test]
fn an_expired_item_clears_its_draft_once() {
    let (rig, id) = prepared_rig();
    rig.db.conn().execute("UPDATE prepared_items SET expires_at = 1.0", []).unwrap();
    commitment_slot::owe_for_expired(&rig.db.conn(), 1000.0);
    assert_eq!(pay(&rig), 1);
    assert_eq!(forgets(&rig).len(), 1);
    // Paid: the item says so, and the next housekeeping owes nothing.
    commitment_slot::owe_for_expired(&rig.db.conn(), 1000.0);
    assert!(commitment_slot::owed(&rig.db.conn()).is_empty());
    assert_eq!(rig.item_for(id).data["slot_cleared"], true);
}

#[test]
fn a_promise_with_no_draft_owes_nothing_and_a_busy_email_keeps_the_clear_owed() {
    // Forgetting a promise that was never prepared owes Email nothing.
    let rig = Rig::new(&[SHEET]);
    rig.say("I'll send Sam the budget spreadsheet on Friday.", MONDAY);
    let id = rig.only_commitment().id;
    commitment_store::forget(&rig.db.conn(), id);
    assert!(commitment_slot::owed(&rig.db.conn()).is_empty());

    let (rig, id) = prepared_rig();
    rig.world.lock().unwrap().refused.push(("email.forget_prepared".into(), "Email is starting; nothing was done \u{2014} send it again in a moment.".into(), true));
    commitment_store::forget(&rig.db.conn(), id);
    assert_eq!(pay(&rig), 0, "not now");
    assert_eq!(commitment_slot::owed(&rig.db.conn()), vec![id.to_string()]);
    assert_eq!(pay(&rig), 1, "later");
}
