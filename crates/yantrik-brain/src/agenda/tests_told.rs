use super::senses::{gather_senses, CalEvent, NoteInfo};
use super::test_support::*;
use super::tick::Switches;
use super::told::{told, MAX_CHARS, MAX_LINES};

const ON: Switches = Switches { private: false, enabled: true };

fn when(t: f64) -> String {
    format!("at {}", (t - T0) / HOUR)
}
fn ev(id: &str, title: &str, start: f64) -> CalEvent {
    CalEvent { id: id.into(), title: title.into(), start }
}
fn note(id: &str, title: &str, items: &[&str]) -> NoteInfo {
    let items: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    NoteInfo::new(id, title, T0 - HOUR, &items)
}

#[test]
fn the_note_names_the_event_and_the_note_with_counts_and_no_note_text() {
    let c = mem();
    let events = [ev("e1", "Record the 60 s demo video", T0 + 20.0 * HOUR)];
    let notes = [note(
        "n1",
        "Launch prep",
        &["pick a thumbnail", "script the demo video", "email Sam secret-plan-x", "book venue"],
    )];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    let t = told(&c, T0, ON, when).expect("a note");
    assert_eq!(t.items, 2);
    assert!(t.text.starts_with("What's going on for the person"), "{}", t.text);
    assert!(t.text.contains("- Record the 60 s demo video, at 20"), "{}", t.text);
    assert!(t.text.contains("- Launch prep: 4 open items, one about the demo video"), "{}", t.text);
    assert!(!t.text.contains("secret-plan-x") && !t.text.contains("thumbnail"), "no note text: {}", t.text);
    assert_eq!(t.shown(), "Told: what's going on (2 items)");
}

#[test]
fn an_empty_agenda_private_mode_and_the_switch_off_give_no_note() {
    let c = mem();
    gather_senses(&c, T0, Some(&[]), Some(&[])).unwrap();
    assert_eq!(told(&c, T0, ON, when), None);
    let events = [ev("e1", "Dentist", T0 + HOUR)];
    gather_senses(&c, T0, Some(&events), None).unwrap();
    assert!(told(&c, T0, ON, when).is_some());
    assert_eq!(told(&c, T0, Switches { private: true, enabled: true }, when), None);
    assert_eq!(told(&c, T0, Switches { private: false, enabled: false }, when), None);
}

#[test]
fn the_note_holds_five_lines_and_six_hundred_characters() {
    let c = mem();
    let events: Vec<CalEvent> = (0..9).map(|i| ev(&format!("e{i}"), &format!("Event number {i}"), T0 + (i + 1) as f64 * HOUR)).collect();
    gather_senses(&c, T0, Some(&events), None).unwrap();
    let t = told(&c, T0, ON, when).unwrap();
    assert_eq!(t.items, MAX_LINES);
    assert_eq!(t.text.lines().count(), MAX_LINES + 1);
    let long: Vec<CalEvent> = (0..5).map(|i| ev(&format!("l{i}"), &"w".repeat(100), T0 + (i + 1) as f64 * HOUR)).collect();
    let c2 = mem();
    gather_senses(&c2, T0, Some(&long), None).unwrap();
    let t2 = told(&c2, T0, ON, when).unwrap();
    assert!(t2.text.chars().count() <= MAX_CHARS && t2.items < 5, "{}", t2.text.len());
}
