//! The cards the shell raises for preparing promised things (#1184, round 5, MEDIUM-1): the question
//! when a proposal is stored, and what was done when something was prepared, one per item.

mod common;

use common::*;
use yantrik_companion::commitment_cards::cards;
use yantrik_companion::commitment_consent::{self, TURN_OFF};

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
    assert_eq!((card.yes_label, card.yes_id.as_str(), card.no_id.as_str()), ("Turn off", TURN_OFF, ""));
    assert!(card.text.contains("opened in Mind View") && card.text.contains("Nothing sent"), "{}", card.text);
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
