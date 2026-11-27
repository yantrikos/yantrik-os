//! What "Prepare for me" starts from: linked events within 36 h, other items within 48 h, the
//! switches.

use super::senses::{gather_senses, CalEvent, NoteInfo};
use super::test_support::*;
use super::tick::Switches;
use super::triggers::{triggers, Why};

const ON: Switches = Switches { private: false, enabled: true };

fn ev(id: &str, title: &str, start: f64) -> CalEvent {
    CalEvent { id: id.into(), title: title.into(), start }
}

#[test]
fn the_trials_event_is_a_trigger_with_its_linked_note() {
    let c = mem();
    let events = [ev("e1", "Record the 60 s demo video", T0 + 20.0 * HOUR), ev("e2", "Launch Yantrik OS on Show HN", T0 + 40.0 * HOUR)];
    let notes = [NoteInfo::new("launch-prep.md", "Launch prep", T0 - HOUR, &["Write the demo video script".to_string(), "Post on Show HN".to_string()])];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    let got = triggers(&c, T0, ON, 5).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].title.as_str(), got[0].note_title.as_deref(), got[0].why), ("Record the 60 s demo video", Some("Launch prep"), Why::EventWithOpenNote));
    assert_eq!(got[0].note_id.as_deref(), Some("launch-prep.md"));
    assert_eq!(got[1].why, Why::DueSoon, "40 h away: due soon, but the 36 h rule does not apply");
    assert!(got.iter().all(|t| t.id.starts_with("calendar:")));
}

#[test]
fn nothing_past_48_hours_and_nothing_that_is_a_note() {
    let c = mem();
    let events = [ev("e1", "Dentist", T0 + 60.0 * HOUR)];
    let notes = [NoteInfo::new("n.md", "Groceries", T0 - HOUR, &["milk".to_string()])];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    assert!(triggers(&c, T0, ON, 5).unwrap().is_empty());
}

#[test]
fn private_mode_and_the_brain_switch_give_nothing() {
    let c = mem();
    gather_senses(&c, T0, Some(&[ev("e1", "Dentist", T0 + 5.0 * HOUR)]), Some(&[])).unwrap();
    assert_eq!(triggers(&c, T0, ON, 5).unwrap().len(), 1);
    assert!(triggers(&c, T0, Switches { private: true, enabled: true }, 5).unwrap().is_empty());
    assert!(triggers(&c, T0, Switches { private: false, enabled: false }, 5).unwrap().is_empty());
}
