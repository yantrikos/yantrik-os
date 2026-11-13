//! The cards the shell raises for preparing promised things (#1184, round 5, MEDIUM-1): the question
//! when a proposal is stored, and what was done when something was prepared, one per item.

mod common;

use common::*;
use yantrik_companion::anticipation::AnticipationEngine;
use yantrik_companion::commitment_cards::cards;
use yantrik_companion::commitment_consent::{self, SEEN, TURN_OFF};
use yantrik_companion::commitment_hook::reply_to_press;
use yantrik_companion::commitment_reply::{shown, Shown};
use yantrik_companion::commitment_store;

#[test]
fn storing_a_proposal_raises_one_card_with_yes_and_no_wired_to_the_answer() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let at = window() + 60.0;
    let shown = cards(&rig.db.conn(), at);
    assert_eq!(shown.len(), 1, "{shown:?}");
    let card = &shown[0];
    assert_eq!((card.yes_label, card.no_label), ("Yes", "No"));
    assert!(card.yes_id.starts_with(commitment_consent::YES) && card.no_id.starts_with(commitment_consent::NO));
    assert!(!card.text.contains("Shall I"), "the question is the heading, said once: {}", card.text);
    assert!(card.text.contains("Nothing would be sent"), "{}", card.text);
    // Asked again, on every tick, it is the same one card.
    for tick in 1..5 {
        assert_eq!(cards(&rig.db.conn(), at + f64::from(tick) * 5.0), shown);
    }
    // Its buttons answer, with the same ids as the brief card's, and the card goes.
    assert!(commitment_consent::person_answer(&rig.db.conn(), &card.yes_id).is_some());
    assert!(cards(&rig.db.conn(), at + 30.0).iter().all(|c| c.yes_id != card.yes_id), "answered, it is gone");
}

#[test]
fn a_no_takes_the_card_away_and_an_expired_proposal_has_none() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let shown = cards(&rig.db.conn(), window() + 60.0);
    assert!(commitment_consent::person_answer(&rig.db.conn(), &shown[0].no_id).is_some());
    assert!(cards(&rig.db.conn(), window() + 90.0).is_empty(), "off: no card");

    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    assert!(cards(&rig.db.conn(), thursday() + 10.0 * DAY).is_empty(), "expired: no card");
}

#[test]
fn a_prepared_item_raises_a_card_that_says_what_was_done_with_turn_off() {
    let (rig, _) = prepared_rig_for_cards();
    let at = window() + 60.0;
    let shown = cards(&rig.db.conn(), at);
    assert_eq!(shown.len(), 1, "{shown:?}");
    let card = &shown[0];
    assert_eq!((card.yes_label, card.no_label, card.no_id.as_str()), ("Got it", "Stop getting things ready", TURN_OFF));
    assert!(card.text.contains("2 files found") && card.text.contains("draft is in Email") && card.text.contains("Nothing sent"), "{}", card.text);
    assert!(!card.text.to_lowercase().contains("mind view"), "no promise of Mind View: {}", card.text);
    assert_eq!(cards(&rig.db.conn(), at + 5.0), shown, "one card per item, not per tick");
    assert!(commitment_consent::person_answer(&rig.db.conn(), TURN_OFF).is_some());
    assert!(cards(&rig.db.conn(), at + 10.0).is_empty(), "turned off: the card goes");
}

#[test]
fn a_prepared_card_goes_when_its_item_expires() {
    let (rig, _) = prepared_rig_for_cards();
    assert_eq!(cards(&rig.db.conn(), window() + 60.0).len(), 1);
    assert!(cards(&rig.db.conn(), thursday() + 10.0 * DAY).is_empty());
}

fn prepared_rig_for_cards() -> (Rig, i64) {
    let rig = Rig::new(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    let id = rig.only_commitment().id;
    rig.cycle(window());
    (rig, id)
}

fn person_says(rig: &Rig, id: &str, now: f64) -> Option<String> {
    commitment_consent::person_answer_at(&rig.db.conn(), id, now)
}

#[test]
fn got_it_puts_that_card_away_and_leaves_preparing_on() {
    let (rig, _) = prepared_rig_for_cards();
    let at = window() + 60.0;
    let card = cards(&rig.db.conn(), at).remove(0);
    assert_eq!((card.yes_label, card.no_label, card.no_id.as_str()), ("Got it", "Stop getting things ready", TURN_OFF));
    assert!(card.yes_id.starts_with(SEEN) && card.yes_id.ends_with(&card.item));
    assert!(person_says(&rig, &card.yes_id, at).is_some());
    assert!(cards(&rig.db.conn(), at + 5.0).is_empty(), "that card is gone");
    assert!(commitment_store::enabled(&rig.db.conn()) && commitment_store::consented(&rig.db.conn()), "consent is as it was");
    assert!(commitment_consent::status(&rig.db.conn(), at).on);
    assert!(person_says(&rig, &card.yes_id, at + 6.0).is_none(), "a second press has nothing to put away");
}

#[test]
fn got_it_puts_away_one_card_only() {
    let (rig, _) = prepared_rig_for_cards();
    let at = window() + 60.0;
    let other = commitment_prep_item(&rig, "commitment-prep-other", at);
    let shown = cards(&rig.db.conn(), at);
    assert_eq!(shown.len(), 2, "{shown:?}");
    let mine = shown.iter().find(|c| c.item == other).unwrap().yes_id.clone();
    assert!(person_says(&rig, &mine, at).is_some());
    let left = cards(&rig.db.conn(), at + 1.0);
    assert_eq!(left.len(), 1);
    assert_ne!(left[0].item, other);
}

#[test]
fn the_seen_id_answers_no_question_turns_nothing_on_and_needs_a_card_on_screen() {
    // A question outstanding: the seen id for its item is not a yes.
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let at = window() + 60.0;
    let q = cards(&rig.db.conn(), at).remove(0);
    let item = q.item.clone();
    assert!(person_says(&rig, &format!("{SEEN}{item}"), at).is_none(), "a proposal has no prepared card");
    assert!(!commitment_store::consented(&rig.db.conn()), "no yes was given");
    assert!(commitment_consent::status(&rig.db.conn(), at).question.is_some(), "the question still waits");
    // Off: the seen id does not turn it on.
    commitment_store::refuse(&rig.db.conn());
    assert!(person_says(&rig, &format!("{SEEN}{item}"), at).is_none());
    assert!(!commitment_store::enabled(&rig.db.conn()));
    // An item that is not on screen is refused.
    let (rig, _) = prepared_rig_for_cards();
    assert!(person_says(&rig, &format!("{SEEN}commitment-prep-nope"), window() + 60.0).is_none(), "not a card shown");
    assert!(person_says(&rig, SEEN, window() + 60.0).is_none());
}

#[test]
fn at_most_three_prepared_cards_show_newest_first() {
    let (rig, _) = prepared_rig_for_cards();
    let at = window() + 60.0;
    for n in 0..4 {
        commitment_prep_item(&rig, &format!("commitment-prep-x{n}"), at + f64::from(n));
    }
    let many = cards(&rig.db.conn(), at + 10.0);
    assert_eq!(many.len(), 3, "at most three prepared cards: {many:?}");
    assert!(many[0].item > many[2].item, "newest first: {many:?}");
}

fn commitment_prep_item(rig: &Rig, id: &str, created: f64) -> String {
    store_item(rig, id, created, serde_json::json!({ "state": "prepared", "files": [{ "label": "a", "path": "/home/p/Documents/a.csv" }] }));
    id.to_string()
}

fn store_item(rig: &Rig, id: &str, created: f64, data: serde_json::Value) {
    use yantrik_companion::anticipation::{AnticipationEngine, PreparationTrigger, PreparedItem, PreparedKind};
    let item = PreparedItem {
        id: id.into(),
        trigger: PreparationTrigger::CommitmentDeadline { commitment_id: "999".into() },
        kind: PreparedKind::CommitmentPrep,
        description: format!("Opened {id}. Nothing sent."),
        data,
        relevant_at: created,
        expires_at: created + 3.0 * DAY,
        surfaced: false,
        outcome: None,
        created_at: created,
    };
    AnticipationEngine::store(&rig.db.conn(), &item);
}

#[test]
fn in_incognito_settings_shows_the_switch_and_no_question() {
    let rig = Rig::asked(&[DECK, SHEET]);
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let at = window() + 60.0;
    let (said, status, shown) = reply_to_press(&rig.db.conn(), true, "", at);
    assert!(said.is_none() && shown.is_empty());
    assert!(status.on && status.question.is_none(), "{status:?}");
    // Nothing was marked asked in incognito, so a press of Yes there answers nothing.
    let yes = commitment_consent::status(&rig.db.conn(), at).question.unwrap().yes;
    assert!(reply_to_press(&rig.db.conn(), true, &yes, at).0.is_none());
    // Not incognito: the same state has the question.
    assert!(reply_to_press(&rig.db.conn(), false, "", at).1.question.is_some());
}

#[test]
fn each_press_that_answers_nothing_says_its_own_true_sentence() {
    let rig = Rig::asked(&[DECK, SHEET]);
    let none = || (None, commitment_consent::Status { on: true, question: None }, Vec::new());
    let toast = |id: &str| shown(id, &none()).toast;
    assert_eq!(toast(commitment_consent::TURN_OFF), "It is already off.");
    assert_eq!(toast(commitment_consent::TURN_ON), "It is already on.");
    assert_eq!(toast(&format!("{SEEN}x")), "That card is already gone.");
    assert!(toast(&format!("{}x", commitment_consent::YES)).starts_with("That question has changed"));
    assert!(toast(&format!("{}x", commitment_consent::NO)).contains("Settings"));
    // An answered press says what it did and puts no question back.
    let said = (Some("Okay.".to_string()), commitment_consent::Status { on: true, question: None }, Vec::new());
    assert_eq!(shown(TURN_OFF, &said), Shown { toast: "Okay.".into(), brief_question: None });
    // A stale yes hands the brief the question outstanding now.
    rig.say(SAID, MONDAY);
    rig.cycle(window());
    let q = commitment_consent::status(&rig.db.conn(), window() + 1.0).question;
    let stale = (None, commitment_consent::Status { on: true, question: q.clone() }, Vec::new());
    assert_eq!(shown(&format!("{}x", commitment_consent::YES), &stale).brief_question, q);
}

#[test]
fn every_prepared_card_says_its_own_draft_is_in_email() {
    // 520: three promises each made a draft and Email kept the last. Email now keeps one draft
    // per promise, so no card has to take back what it said.
    let q3 = "/home/p/Documents/Q3-deck.ydeck";
    let rig = Rig::new(&[DECK, q3]);
    rig.say("I'm sending Maya the Q4 deck on Thursday.", MONDAY);
    rig.say("I'm sending Ravi the Q3 deck on Thursday.", MONDAY);
    let outcomes = rig.cycle(window());
    assert_eq!(outcomes.len(), 2, "{outcomes:?}");
    let shown = cards(&rig.db.conn(), window() + 60.0);
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(shown.iter().all(|c| c.text.contains("The draft is in Email") && !c.text.contains("replaced")), "{shown:?}");
    let mut ids: Vec<String> = rig
        .calls()
        .into_iter()
        .filter(|(_, a)| a["action"] == "compose")
        .map(|(_, a)| a["args"]["commitment"].to_string())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 2, "each draft carries its own promise's id: {ids:?}");
}

#[test]
fn a_prepared_card_carries_the_located_files_and_the_commitment_whose_draft_waits() {
    let (rig, _) = prepared_rig_for_cards();
    let shown = cards(&rig.db.conn(), window() + 60.0);
    let card = &shown[0];
    let paths: Vec<&str> = card.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, [DECK, SHEET], "the files the promise named, as located");
    let id = AnticipationEngine::get_relevant_at(&rig.db.conn(), window() + 60.0)[0].data["commitment_id"].to_string();
    assert_eq!(card.draft_for.as_deref(), Some(id.as_str()), "the draft is opened by its commitment");
}
