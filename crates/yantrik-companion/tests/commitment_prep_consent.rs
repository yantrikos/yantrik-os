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

/// Every tool a model turn is offered, from the registry the shell builds, at the highest
/// permission any turn has: `(name, the words of its definition)`.
fn tools_a_model_is_offered() -> Vec<(String, String)> {
    let shell = yantrik_companion::tools::build_registry(&yantrik_companion::config::CompanionConfig::default());
    let offered: Vec<_> = shell
        .definitions(PermissionLevel::Dangerous)
        .iter()
        .map(|d| (d["function"]["name"].as_str().unwrap_or_default().to_string(), d.to_string().to_lowercase()))
        .collect();
    assert!(offered.len() > 20, "the real registry was enumerated: {}", offered.len());
    offered
}

#[test]
fn no_tool_a_model_turn_is_offered_can_change_the_commitment_prep_setting() {
    // Gate 8, bar 3: a companion turn, told to press "Got it" by calling `commitment_prep_setting`,
    // switched preparing off. Only the person's own pointer click (the card's "Stop getting things
    // ready", the Settings switch) may do that, so no tool offered to a model touches it.
    let offered = tools_a_model_is_offered();
    for (name, words) in &offered {
        assert!(!name.contains("commitment_prep"), "{name} is offered");
        let about_the_switch = words.contains("commitment_prep") || (words.contains("getting things ready") || words.contains("prepar") && (words.contains("setting") || words.contains("switch")));
        assert!(!about_the_switch, "{name} is offered and speaks of the commitment_prep switch: {words}");
    }
    // By behaviour: every tool named for commitments or preparing, called with every spelling
    // of an answer, leaves the switch, the consent and the answer as they were.
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.person_turns_it_on();
    rig.say(SAID, MONDAY);
    let shell = yantrik_companion::tools::build_registry(&yantrik_companion::config::CompanionConfig::default());
    let ctx = ToolContext { db: &rig.db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    let before = commitment_store::answered(&rig.db.conn());
    for (name, _) in offered.iter().filter(|(n, _)| n.contains("commitment") || n.contains("prepar") || n.contains("setting")) {
        for choice in ["yes", "on", "no", "off", "YES", "Off"] {
            shell.execute(&ctx, name, &json!({ "choice": choice, "enabled": choice, "value": choice, "key": "commitment_prep", "setting": "commitment_prep" }));
            assert!(commitment_store::enabled(&rig.db.conn()), "{name} {choice} switched it off");
            assert!(commitment_store::consented(&rig.db.conn()), "{name} {choice} took consent away");
            assert_eq!(commitment_store::answered(&rig.db.conn()), before, "{name} {choice} wrote an answer");
        }
    }
    // And the name itself is unknown to the registry, so calling it does nothing at all.
    shell.execute(&ctx, "commitment_prep_setting", &json!({ "choice": "off" }));
    assert!(commitment_store::enabled(&rig.db.conn()));
    assert!(matches!(rig.cycle(window()).as_slice(), [Outcome::Prepared(_)]), "it goes on preparing");
}

#[test]
fn the_cards_stop_action_still_switches_preparing_off() {
    // The person's pointer click on "Stop getting things ready" is the person-only path, and works.
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.person_turns_it_on();
    rig.say(SAID, MONDAY);
    assert!(commitment_store::enabled(&rig.db.conn()));
    assert!(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_OFF).is_some());
    assert!(!commitment_store::enabled(&rig.db.conn()));
    assert!(matches!(rig.cycle(window()).as_slice(), [Outcome::Nothing(_)]), "nothing more is prepared");
    assert_eq!(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_OFF), None, "already off says so, changes nothing");
}

#[test]
fn a_model_that_has_not_been_told_yes_cannot_make_it_consent() {
    let rig = rig_with_tools(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    assert!(matches!(rig.cycle(window()).as_slice(), [Outcome::Prepared(_)]));
    for choice in ["yes", "on"] {
        call(&rig, "commitment_prep_setting", json!({ "choice": choice }));
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
        for needle in ["commitment_consent", "person_answer", "record_answer", "commitment_store::refuse", "store::refuse"] {
            assert!(!src.contains(needle) || entry.file_name() == "commitment_prep_tools.rs" && !src.contains("person_answer(") && !src.contains("record_answer(") && !src.contains("::refuse("), "{} names {needle}", entry.path().display());
        }
    }
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
    assert_eq!(shown.len(), 1, "one row, with its own yes and no buttons: {shown:?}");
    let no = commitment_consent::no_for(&shown[0].action_id).expect("a no for the yes");
    assert!(commitment_consent::person_answer(&rig.db.conn(), &format!("{}commitment-prep-999", commitment_consent::YES)).is_none());
    assert!(!commitment_store::consented(&rig.db.conn()));
    // The button on the item answers it.
    assert!(commitment_consent::person_answer(&rig.db.conn(), &shown[0].action_id).is_some());
    assert!(commitment_store::consented(&rig.db.conn()));
    assert_eq!(commitment_store::answered(&rig.db.conn()), Some(("person".into(), item)));
    // And a repeated click, with no question outstanding, changes nothing.
    assert!(commitment_consent::person_answer(&rig.db.conn(), &no).is_none());
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
    assert_eq!(commitment_consent::brief_actions(&rig.db.conn(), window() + 60.0).len(), 1);
    assert!(commitment_store::asked(&rig.db.conn()));
    assert!(commitment_store::asking(&rig.db.conn()).is_some());
}

#[test]
fn off_then_on_by_the_person_prepares_again() {
    let rig = rig_with_tools(&[DECK, SHEET]);
    rig.person_turns_it_on();
    rig.say(SAID, MONDAY);
    // The person says "turn that off", and uses the switch in Settings.
    assert!(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_OFF).is_some());
    assert!(matches!(rig.cycle(window()).as_slice(), [Outcome::Nothing(_)]));
    // Saying "turn it back on" to the model does nothing.
    call(&rig, "commitment_prep_setting", json!({ "choice": "on" }));
    assert!(matches!(rig.cycle(window() + 60.0).as_slice(), [Outcome::Nothing(_)]));
    // The switch in Settings turns it back on: the person-only act, and is consent.
    assert_eq!(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_ON).as_deref(), Some("Preparing is on."));
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
    pay_with(rig, true)
}

fn pay_with(rig: &Rig, email_running: bool) -> usize {
    let ctx = ToolContext { db: &rig.db, max_permission: PermissionLevel::Standard, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    commitment_slot::pay(&rig.registry, &ctx, std::time::Duration::from_secs(60), email_running)
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

/// P3, on the companion's side: a clear never starts Email, so with Email not running nothing is
/// asked and the clear stays owed until it is.
#[test]
fn a_clear_never_starts_email_and_stays_owed_until_it_runs() {
    let (rig, id) = prepared_rig();
    commitment_store::forget(&rig.db.conn(), id);
    let before = rig.calls().len();
    assert_eq!(pay_with(&rig, false), 0);
    assert_eq!(rig.calls().len(), before, "no call was made, so nothing could start Email");
    assert_eq!(commitment_slot::owed(&rig.db.conn()), vec![id.to_string()]);
    assert_eq!(pay_with(&rig, true), 1);
}

/// HIGH-1: Email's answer while the person has the draft open keeps the clear owed.
#[test]
fn a_draft_the_person_has_open_keeps_the_clear_owed() {
    let (rig, id) = prepared_rig();
    let open = "The person has the prepared draft open, so it was left alone; try again when it is closed.";
    rig.world.lock().unwrap().refused.push(("email.forget_prepared".into(), open.into(), true));
    commitment_store::forget(&rig.db.conn(), id);
    assert_eq!(pay(&rig), 0);
    assert_eq!(commitment_slot::owed(&rig.db.conn()), vec![id.to_string()]);
    assert_eq!(pay(&rig), 1, "once it is closed");
}

/// LOW-4: after the person's own No the brief never asks again; the way back is the switch.
/// After a model-turn refuse the brief offers it.
#[test]
fn after_the_persons_own_no_the_brief_shows_no_turn_back_on_row_but_a_models_refuse_does() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let at = window() + 60.0;
    assert!(rig.person_answers(commitment_consent::NO, at).is_some());
    assert!(!commitment_store::enabled(&rig.db.conn()));
    assert!(commitment_consent::brief_actions(&rig.db.conn(), at + DAY).is_empty(), "no row after the person's No");
    let status = commitment_consent::status(&rig.db.conn(), at);
    assert!(!status.on, "the switch in Settings shows it off");
    assert!(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_ON).is_some(), "and turns it on");
    // A model turn that refuses is not the person's choice: the brief offers the way back.
    commitment_store::refuse(&rig.db.conn());
    let shown = commitment_consent::brief_actions(&rig.db.conn(), at + 2.0 * DAY);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].action_id, commitment_consent::TURN_ON);
}

/// MEDIUM-3: Settings can turn it off and on.
#[test]
fn the_settings_switch_turns_it_off_and_on() {
    let rig = rig_with_tools(&[DECK]);
    rig.person_turns_it_on();
    assert!(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_OFF).is_some());
    assert!(!commitment_store::enabled(&rig.db.conn()));
    assert!(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_OFF).is_none(), "off twice is nothing");
}

/// 520's toast cut every sentence past about 37 characters: "...and never send anything." and
/// "...ready is still on." were not readable. Every answer the person's press gives fits.
#[test]
fn every_answer_to_a_press_is_short_enough_to_read_whole_in_the_toast() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let at = window() + 60.0;
    let mut said = Vec::new();
    // Yes, then Got it on the card that comes of it, then off and on from Settings.
    said.extend(rig.person_answers(commitment_consent::YES, at));
    rig.cycle(window() + 120.0);
    let seen = yantrik_companion::commitment_cards::prepared(&rig.db.conn(), window() + 180.0);
    for item in seen.iter().take(1) {
        said.extend(commitment_consent::person_answer_at(&rig.db.conn(), &format!("{}{}", commitment_consent::SEEN, item.id), window() + 180.0));
    }
    said.extend(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_OFF));
    said.extend(commitment_consent::person_answer(&rig.db.conn(), commitment_consent::TURN_ON));
    // And the No.
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    said.extend(rig.person_answers(commitment_consent::NO, at));
    assert!(said.len() >= 4, "{said:?}");
    for line in &said {
        assert!(line.chars().count() <= commitment_consent::TOAST_CHARS, "cut in the toast: {line:?}");
    }
}
