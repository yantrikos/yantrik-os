//! The rules around preparing a promised thing (#1184, review of 377b29c5): what is not a promise,
//! asking once before acting, which file is the one, where a window lands, what is retried, what is
//! kept, and what the worker's hook does.

mod common;

use common::*;
use serde_json::json;
use std::time::Duration;
use yantrik_companion::anticipation::AnticipationEngine;
use yantrik_companion::commitment_extractor::extract_commitments_at;
use yantrik_companion::commitment_hook::{note_commitments_in, run_urge};
use yantrik_companion::commitment_prep::{commitment_to_prepare, Outcome, CONFIDENCE_FLOOR};
use yantrik_companion::{commitment_consent, commitment_slot, commitment_store};
use yantrik_companion::world_model::{CommitmentSource, WorldModel};
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::ToolContext;
use yantrik_companion_core::types::UrgeSpec;

fn found_by_extractor(text: &str) -> Vec<yantrik_companion::commitment_extractor::ExtractedCommitment> {
    extract_commitments_at(text, "Pranab", &CommitmentSource::Conversation { turn_id: None }, MONDAY)
}

// ── MEDIUM-4: what is not a promise ─────────────────────────────────

/// The review's probe rows, each as a negative test.
#[test]
fn sarcasm_is_not_a_commitment() {
    assert!(found_by_extractor("Oh sure, I'll send Maya the Q4 deck on Thursday, right after pigs fly.").is_empty());
}

#[test]
fn someone_elses_quoted_promise_is_not_the_persons() {
    assert!(found_by_extractor("Priya wrote: I'm sending you the contract on Friday.").is_empty());
}

#[test]
fn a_conditional_is_not_a_commitment() {
    assert!(found_by_extractor("If Maya asks, I'll send her the Q4 deck on Thursday.").is_empty());
}

#[test]
fn a_weekday_inside_a_past_clause_is_not_the_deadline() {
    let found = found_by_extractor("I'll tell Sam about the report I finished on Monday.");
    assert!(found.iter().all(|c| c.deadline_ts == 0.0), "{found:?}");
}

#[test]
fn another_sentences_file_does_not_follow_a_commitment_that_comes_after_it() {
    let found = found_by_extractor("I'm sending Maya the Q4 deck on Thursday. I'll also call Bob about the budget spreadsheet tomorrow.");
    let bob = found.iter().find(|c| c.promisee.as_deref() == Some("Bob")).expect("Bob's commitment");
    assert_eq!(bob.files, vec!["file:budget spreadsheet"], "no Q4 deck from the sentence before");
}

#[test]
fn promises_in_the_past_or_the_negative_still_make_nothing() {
    for text in ["I sent the Q4 deck to Maya on Thursday.", "I'd send her the Q4 deck.", "She'll send the Q4 deck on Thursday.", "I'm not sending Maya the Q4 deck on Thursday."] {
        assert!(found_by_extractor(text).is_empty(), "{text}");
    }
}

#[test]
fn a_guess_below_the_confidence_floor_is_not_prepared() {
    let rig = Rig::new(&[DECK, SHEET]);
    // A request is a weaker signal than a promise: 0.60, under the floor.
    let found = found_by_extractor("Can you remind me to send Maya the Q4 deck by Thursday?");
    assert!(found.iter().all(|c| c.confidence < CONFIDENCE_FLOOR) || found.is_empty());
    rig.say(SAID, MONDAY);
    let id = rig.only_commitment().id;
    rig.db.conn().execute("UPDATE wm_commitments SET confidence = 0.6 WHERE id = ?1", [id]).unwrap();
    assert!(matches!(rig.prepare(id, window()), Outcome::Nothing(_)));
    assert!(rig.calls().is_empty());
}

// ── MEDIUM-4: asked once, earned ────────────────────────────────────

#[test]
fn the_first_promise_asks_once_and_only_records_what_would_be_prepared() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    let item = the_item(&outcomes);
    assert_eq!(item.data["state"], "proposed");
    assert!(item.description.ends_with("Shall I get these ready for promises like this?"), "{}", item.description);
    assert!(!commitment_store::asked(&rig.db.conn()), "stored is not shown: nobody has been asked yet");
    assert!(item.description.contains("Nothing would be sent."));
    assert!(rig.calls().is_empty(), "nothing was searched, opened or drafted: {:?}", rig.calls());
    // Not asked again, and still nothing done, on the next cycles.
    assert!(matches!(rig.cycle(window() + HOUR).as_slice(), [Outcome::AlreadyPrepared]));
    assert!(rig.calls().is_empty());

    // A second promise before the answer is recorded too, and nothing is done for it.
    rig.say("I'll send Sam the budget spreadsheet on Friday.", MONDAY);
    let outcomes = rig.cycle(friday_window());
    let second = outcomes.iter().find_map(|o| match o { Outcome::Prepared(i) if i.data["promisee"] == "Sam" => Some(i), _ => None }).expect("Sam's item");
    assert_eq!(second.data["state"], "proposed");
    assert!(rig.calls().is_empty());
}

#[test]
fn yes_makes_the_next_run_prepare_it() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    assert_eq!(the_item(&rig.cycle(window())).data["state"], "proposed");
    assert!(rig.person_answers(commitment_consent::YES, window() + HOUR).is_some());
    let outcomes = rig.cycle(window() + HOUR);
    assert_eq!(the_item(&outcomes).data["state"], "prepared");
    assert_eq!(rig.items(), 1, "the record was updated, not duplicated");
    assert!(rig.app_calls().contains(&("email".into(), "compose".into())));
}

#[test]
fn the_switch_stops_everything() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    commitment_store::refuse(&rig.db.conn());
    assert!(matches!(rig.cycle(window()).as_slice(), [Outcome::Nothing(_)]));
    assert!(rig.calls().is_empty());
}

// ── MEDIUM-7: privacy and housekeeping ──────────────────────────────

#[test]
fn incognito_extracts_and_prepares_nothing() {
    let rig = Rig::new(&[DECK, SHEET]);
    assert_eq!(note_commitments_in(&rig.db.conn(), true, "Pranab", SAID, 1, MONDAY), 0);
    assert!(WorldModel::active_commitments(&rig.db.conn()).is_empty());
    rig.say(SAID, MONDAY);
    let id = rig.only_commitment().id;
    let mut incognito = Rig::new(&[DECK, SHEET]);
    incognito.incognito = true;
    incognito.say(SAID, MONDAY);
    assert!(matches!(incognito.prepare(incognito.only_commitment().id, window()), Outcome::Nothing(_)));
    assert!(incognito.calls().is_empty());
    assert_eq!(note_commitments_in(&rig.db.conn(), false, "Pranab", "I'll send Sam the budget spreadsheet on Friday.", 2, MONDAY), 1);
    let _ = id;
}

#[test]
fn forgetting_a_commitment_deletes_its_item() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let id = rig.only_commitment().id;
    rig.cycle(window());
    assert_eq!(rig.items(), 1);
    assert!(commitment_store::forget(&rig.db.conn(), id));
    assert_eq!(rig.items(), 0);
    assert!(WorldModel::active_commitments(&rig.db.conn()).is_empty());
}

#[test]
fn the_item_keeps_paths_and_words_and_no_tool_output() {
    let rig = Rig::new(&[DECK, SHEET, "/home/p/Documents/other-secret.csv"]);
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    let stored = the_item(&outcomes).data.to_string();
    assert!(!stored.contains("other-secret"), "a file that was listed and not chosen is not kept: {stored}");
    assert!(!stored.contains("file(s) found") && !stored.contains('\u{2192}'), "{stored}");
}

#[test]
fn expired_items_are_closed_by_the_housekeeping_the_worker_calls() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let id = rig.only_commitment().id;
    rig.cycle(window());
    rig.db.conn().execute("UPDATE prepared_items SET expires_at = 1.0, created_at = 1.0", []).unwrap();
    AnticipationEngine::cleanup_expired(&rig.db.conn());
    assert!(AnticipationEngine::for_commitment(&rig.db.conn(), &id.to_string()).is_none(), "very old items go");
}

// ── HIGH-2: which file ──────────────────────────────────────────────

/// Friday's deadline for a promise said at `MONDAY`, and a moment inside its prep window.
fn friday_window() -> f64 {
    yantrik_companion::commitment_when::next_weekday_at_five(MONDAY, 4, &chrono::Local).unwrap() - 23.0 * HOUR
}

fn said_with(files: &[&str], text: &str) -> (Rig, Vec<Outcome>) {
    let rig = Rig::new(files);
    rig.say(text, MONDAY);
    let outcomes = rig.cycle(if text.contains("Friday") { friday_window() } else { window() });
    (rig, outcomes)
}

#[test]
fn a_bare_noun_names_no_file_and_the_newest_is_never_taken() {
    let (rig, outcomes) = said_with(&["/home/p/Documents/journal.md", "/home/p/Documents/contract-old.md"], "I'm forwarding you the contract on Friday.");
    let item = the_item(&outcomes);
    assert_eq!(item.data["attachment"], serde_json::Value::Null);
    assert_eq!(item.data["opened"], json!([]));
    assert!(item.data["missing"][0].as_str().unwrap().contains("couldn't tell which file you mean by the contract"), "{}", item.data["missing"]);
    assert!(rig.app_calls().is_empty(), "nothing was opened or drafted");
}

#[test]
fn more_than_one_candidate_chooses_none_and_says_how_many() {
    let (rig, outcomes) = said_with(
        &["/home/p/Documents/Q4-deck.pptx", "/home/p/Documents/Q4-deck.pdf", "/home/p/Desktop/q4-deck-final.odp", SHEET],
        "I'm sending Maya the Q4 deck on Thursday.",
    );
    let item = the_item(&outcomes);
    assert_eq!(item.data["missing"][0], "found 3 files that could be the Q4 deck");
    assert!(rig.app_calls().iter().all(|(app, _)| app != "email"), "no draft over a guess");
}

#[test]
fn a_file_that_merely_contains_the_word_is_not_a_candidate() {
    let (rig, outcomes) = said_with(&["/home/p/Documents/q4-tax-return.pdf", "/home/p/Documents/Q40-deck.pptx", "/home/p/Documents/q4-notes.md"], "I'm sending Maya the Q4 deck on Thursday.");
    assert_eq!(the_item(&outcomes).data["missing"][0], "couldn't find the Q4 deck");
    assert!(rig.app_calls().is_empty());
}

#[test]
fn only_the_persons_document_folders_are_searched() {
    let (rig, outcomes) = said_with(&["/home/p/projects/archive/Q4-deck.ydeck"], "I'm sending Maya the Q4 deck on Thursday.");
    assert_eq!(the_item(&outcomes).data["missing"][0], "couldn't find the Q4 deck");
    for (tool, args) in rig.calls() {
        if tool == "glob" {
            let path = args["path"].as_str().unwrap();
            assert!(["/home/p/Documents", "/home/p/Desktop", "/home/p/Downloads"].contains(&path), "{path}");
            assert_ne!(path, "~");
        }
    }
}

#[test]
fn too_many_files_to_list_is_said_and_not_guessed() {
    let many: Vec<String> = (0..60).map(|i| format!("/home/p/Documents/report-{i}.pdf")).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let (_, outcomes) = said_with(&refs, "I'm sending Maya the Acme report on Thursday.");
    assert!(the_item(&outcomes).data["missing"][0].as_str().unwrap().contains("too many files"), "{:?}", the_item(&outcomes).data["missing"]);
}

#[test]
fn a_cut_listing_with_one_visible_candidate_chooses_none() {
    // `q4-deck-v1.pdf` is hidden past the tool's cap; `q4-deck-final.pdf` is listed. One candidate
    // seen is not "the one" when the list was cut.
    let mut files: Vec<String> = vec!["/home/p/Documents/q4-deck-final.pdf".into()];
    files.extend((0..60).map(|i| format!("/home/p/Documents/report-{i}.pdf")));
    files.push("/home/p/Documents/q4-deck-v1.pdf".into());
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let (rig, outcomes) = said_with(&refs, "I'm sending Maya the Q4 deck on Thursday.");
    let item = the_item(&outcomes);
    assert!(item.data["missing"][0].as_str().unwrap().contains("too many files"), "{:?}", item.data["missing"]);
    assert!(rig.app_calls().iter().all(|(_, a)| a != "open" && a != "compose"), "nothing opened or attached: {:?}", rig.app_calls());
}

// ── MEDIUM-5: where a window landed ─────────────────────────────────

#[test]
fn a_launch_that_lands_on_the_desktop_does_not_get_the_file() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.world.lock().unwrap().lands_in = "the person's desktop".into();
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    let item = the_item(&outcomes);
    assert_eq!(item.data["opened"], json!([]));
    assert!(item.data["missing"][0].as_str().unwrap().contains("apps open on your desktop with your current setting"), "{}", item.data["missing"]);
    assert!(rig.app_calls().iter().all(|(_, action)| action != "open"), "no file was opened: {:?}", rig.app_calls());
    assert_eq!(item.data["draft"]["state"], "draft", "the draft does not need Mind View");
}

#[test]
fn an_app_already_open_on_the_desktop_is_left_alone() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.world.lock().unwrap().refused.push((
        "shell.open_app".into(),
        "shell.open_app \u{2192} {\"launching\":\"presentation\",\"note\":\"already open on the person's desktop: not launched again\"}".into(),
        false,
    ));
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    assert!(the_item(&outcomes).data["missing"][0].as_str().unwrap().contains("already open on your desktop"));
    assert!(rig.app_calls().iter().all(|(_, a)| a != "open"));
}

// ── MEDIUM-6: transient and final failures ──────────────────────────

#[test]
fn a_slow_email_is_retried_with_backoff_and_never_shows_the_tool_sentence() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.world.lock().unwrap().refused.push((
        "email.compose".into(),
        "Email is starting; nothing was done \u{2014} send it again in a moment.".into(),
        true,
    ));
    rig.say(SAID, MONDAY);
    let first = rig.cycle(window());
    let item = the_item(&first);
    assert_eq!(item.data["state"], "retry");
    assert!(item.data["draft"].is_null());
    let text = format!("{} {}", item.description, item.data);
    assert!(!text.contains("nothing was done") && !text.contains("send it again"), "{text}");
    assert!(item.description.contains("Email wasn't ready, so I'll try again"), "{}", item.description);
    let calls_then = rig.calls().len();

    // Too soon: it waits.
    assert!(matches!(rig.cycle(window() + 60.0).as_slice(), [Outcome::Nothing("waiting to try again")]));
    assert_eq!(rig.calls().len(), calls_then);

    // After the pause: the draft is made, and the files already opened are not opened again.
    let later = rig.cycle(window() + 10.0 * 60.0);
    let item = the_item(&later);
    assert_eq!(item.data["state"], "prepared");
    assert_eq!(item.data["draft"]["attached"], DECK);
    let opens = rig.app_calls().iter().filter(|(_, a)| a == "open").count();
    assert_eq!(opens, 2, "each file was opened once");
    assert_eq!(rig.items(), 1);
}

#[test]
fn a_final_refusal_is_said_in_words_and_not_retried() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.world.lock().unwrap().refused.push(("email.compose".into(), "Refused: `app_action` reaches the internet.".into(), false));
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    let item = the_item(&outcomes);
    assert_eq!(item.data["state"], "prepared");
    assert_eq!(item.data["missing"][0], "couldn't draft the email: I'm not allowed to do that in Email while you're away");
    assert!(!item.data.to_string().contains("reaches the internet"));
}

#[test]
fn a_person_with_an_unsent_email_of_their_own_is_waited_on() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.world.lock().unwrap().refused.push((
        "email.compose".into(),
        "You have an unsent message of your own, and a draft from Yantrik would not replace it. Send or discard yours first.".into(),
        true,
    ));
    rig.say(SAID, MONDAY);
    let item_state = the_item(&rig.cycle(window())).data["state"].clone();
    assert_eq!(item_state, "retry");
}

#[test]
fn when_the_window_closes_a_retry_is_final_and_says_what_is_missing() {
    let rig = Rig::new(&[SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let id = rig.only_commitment().id;
    let Outcome::Prepared(done) = rig.prepare(id, thursday() + 3.0 * HOUR) else { panic!("expected the final item") };
    assert_eq!(done.data["state"], "prepared");
    assert_eq!(done.data["missing"][0], "couldn't find the Q4 deck");
}

// ── LOW-10: the hook is capped ──────────────────────────────────────

#[test]
fn a_preparation_out_of_time_stops_calling_and_is_retried_later() {
    let mut rig = Rig::new(&[DECK, SHEET]);
    rig.settings.budget = Duration::ZERO;
    rig.say(SAID, MONDAY);
    let outcomes = rig.cycle(window());
    let item = the_item(&outcomes);
    assert_eq!(item.data["state"], "retry");
    assert!(rig.calls().is_empty(), "no call was started once the budget was spent");
    assert!(item.description.contains("ran out of time"), "{}", item.description);
}

// ── MEDIUM-8: the worker's hook ─────────────────────────────────────

fn urge_for(id: i64) -> UrgeSpec {
    UrgeSpec::new("commitment_prep", "EXECUTE Prepare what you promised", 0.8)
        .with_context(json!({ "executor": "commitment_prep", "commitment_id": id }))
}

#[test]
fn the_hook_takes_a_prep_urge_and_leaves_every_other_to_the_model() {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let id = rig.only_commitment().id;
    let ctx = ToolContext { db: &rig.db, max_permission: PermissionLevel::Standard, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    assert!(run_urge(&rig.registry, &ctx, &urge_for(id), window(), &rig.settings), "taken");
    assert_eq!(rig.items(), 1);
    let other = UrgeSpec::new("check_in", "Say hello", 0.5);
    assert!(!run_urge(&rig.registry, &ctx, &other, window(), &rig.settings), "left to the model");
    assert_eq!(commitment_to_prepare(&other), None);
}

#[test]
fn the_hook_notes_promises_and_skips_what_is_not_one() {
    let rig = Rig::new(&[]);
    assert_eq!(note_commitments_in(&rig.db.conn(), false, "Pranab", SAID, 1, MONDAY), 1);
    assert_eq!(note_commitments_in(&rig.db.conn(), false, "Pranab", "Oh sure, I'll send Maya the Q4 deck on Thursday, right after pigs fly.", 2, MONDAY), 0);
}
