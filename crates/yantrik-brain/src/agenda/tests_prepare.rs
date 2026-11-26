//! The first anticipate rule: linked event and note, once per event, inside 36 hours, and the
//! switches.

use super::describe;
use super::prepare::{candidates, offered, record, settle_ignored, waiting, Outcome, RULE_EVENT_NOTE};
use super::senses::{gather_senses, CalEvent, NoteInfo};
use super::test_support::*;
use super::tick::Switches;

const ON: Switches = Switches { private: false, enabled: true };

fn ev(id: &str, title: &str, start: f64) -> CalEvent {
    CalEvent { id: id.into(), title: title.into(), start }
}
fn note(id: &str, title: &str, items: &[&str]) -> NoteInfo {
    let items: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    NoteInfo::new(id, title, T0 - HOUR, &items)
}

/// The trial's data: tomorrow 15:00's demo recording, and "Launch prep" with four open items.
fn trial(c: &rusqlite::Connection) {
    let events = [ev("e1", "Record the 60 s demo video", T0 + 20.0 * HOUR)];
    let notes = [note(
        "launch-prep.md",
        "Launch prep",
        &["Write the demo video script", "Post on Show HN", "Reply to testers", "Update the README"],
    )];
    gather_senses(c, T0, Some(&events), Some(&notes)).unwrap();
}

#[test]
fn a_linked_note_with_open_items_makes_one_candidate_in_the_events_and_notes_words() {
    let c = mem();
    trial(&c);
    let got = candidates(&c, T0, ON, true).unwrap();
    assert_eq!(got.len(), 1);
    let m = &got[0];
    assert_eq!((m.event_title.as_str(), m.note_title.as_str(), m.open_items), ("Record the 60 s demo video", "Launch prep", 4));
    assert_eq!(m.start, T0 + 20.0 * HOUR);
    assert!(m.reason.contains("Launch prep") && m.reason.contains("4 open items") && m.reason.contains("demo video"), "{}", m.reason);
    assert!(m.ask_text().contains("Record the 60 s demo video") && m.ask_text().contains("Launch prep"));
    assert!(m.ask_text_at("Fri 27 Nov, 15:00").contains("\u{2019} (Fri 27 Nov, 15:00) from my note"));
}

#[test]
fn an_unlinked_note_or_one_with_nothing_open_makes_none() {
    let c = mem();
    let events = [ev("e1", "Dentist", T0 + 5.0 * HOUR)];
    let notes = [note("n.md", "Groceries", &["milk", "bread"])];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    assert!(candidates(&c, T0, ON, true).unwrap().is_empty(), "no shared word: no link");

    let c = mem();
    let events = [ev("e1", "Record the demo video", T0 + 5.0 * HOUR)];
    let notes = [note("n.md", "Launch prep", &[])];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    assert!(candidates(&c, T0, ON, true).unwrap().is_empty(), "nothing open: no note entry at all");
}

#[test]
fn an_event_outside_thirty_six_hours_or_already_started_makes_none() {
    let c = mem();
    let events = [ev("far", "Record the demo video", T0 + 40.0 * HOUR)];
    // Within the link window of three days, but past 36 hours.
    gather_senses(&c, T0, Some(&events), Some(&[note("n.md", "Launch prep", &["demo video script"])])).unwrap();
    assert!(candidates(&c, T0, ON, true).unwrap().is_empty());
    assert_eq!(candidates(&c, T0 + 5.0 * HOUR, ON, true).unwrap().len(), 1, "inside 36 hours it is a candidate");
    assert!(candidates(&c, T0 + 41.0 * HOUR, ON, true).unwrap().is_empty(), "started");
}

#[test]
fn one_candidate_per_event_ever() {
    let c = mem();
    trial(&c);
    let m = candidates(&c, T0, ON, true).unwrap().remove(0);
    assert!(offered(&c, &m, T0).unwrap());
    assert!(!offered(&c, &m, T0 + 1.0).unwrap(), "the ledger holds it once");
    assert!(candidates(&c, T0 + 60.0, ON, true).unwrap().is_empty(), "not again");
    // Whatever the person did, and after the notes are read again, it stays once.
    record(&c, RULE_EVENT_NOTE, "e1", Outcome::Dismissed, T0 + 70.0).unwrap();
    trial(&c);
    assert!(candidates(&c, T0 + 120.0, ON, true).unwrap().is_empty());
}

#[test]
fn two_notes_for_one_event_make_one_candidate_with_the_fuller_note() {
    let c = mem();
    let events = [ev("e1", "Record the demo video", T0 + 5.0 * HOUR)];
    let notes = [note("a.md", "Short", &["demo video"]), note("b.md", "Launch prep", &["demo video a", "demo video b", "demo video c"])];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    let got = candidates(&c, T0, ON, true).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].note_title, "Launch prep");
}

#[test]
fn private_mode_the_brain_switch_and_the_moments_own_switch_each_say_nothing() {
    let c = mem();
    trial(&c);
    assert!(candidates(&c, T0, Switches { private: true, enabled: true }, true).unwrap().is_empty());
    assert!(candidates(&c, T0, Switches { private: false, enabled: false }, true).unwrap().is_empty());
    assert!(candidates(&c, T0, ON, false).unwrap().is_empty());
    assert_eq!(candidates(&c, T0, ON, true).unwrap().len(), 1);
}

#[test]
fn the_outcome_is_recorded_once_and_an_unanswered_moment_is_ignored_after_the_event_starts() {
    let c = mem();
    trial(&c);
    let m = candidates(&c, T0, ON, true).unwrap().remove(0);
    offered(&c, &m, T0).unwrap();
    assert_eq!(waiting(&c).unwrap().len(), 1);
    assert_eq!(settle_ignored(&c, T0 + HOUR).unwrap(), 0, "the event has not started");
    record(&c, RULE_EVENT_NOTE, "e1", Outcome::Opened, T0 + 2.0 * HOUR).unwrap();
    record(&c, RULE_EVENT_NOTE, "e1", Outcome::Dismissed, T0 + 3.0 * HOUR).unwrap();
    assert!(waiting(&c).unwrap().is_empty());
    let d = describe(&c, T0 + 4.0 * HOUR).unwrap();
    assert_eq!(d["moments"][0]["outcome"], "opened", "the first answer stays");
    assert_eq!(d["moments"][0]["key"], "prepare");
    assert_eq!(d["moments"][0]["title"], "Record the 60 s demo video");

    let c = mem();
    trial(&c);
    let m = candidates(&c, T0, ON, true).unwrap().remove(0);
    offered(&c, &m, T0).unwrap();
    assert_eq!(settle_ignored(&c, T0 + 21.0 * HOUR).unwrap(), 1);
    assert_eq!(describe(&c, T0 + 22.0 * HOUR).unwrap()["moments"][0]["outcome"], "ignored");
}
