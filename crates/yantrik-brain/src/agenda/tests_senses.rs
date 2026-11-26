//! The calendar and notes senses, and the tick that runs them.

use std::fs;

use super::senses::{gather_senses, CalEvent, NoteInfo};
use super::store::find;
use super::test_support::*;
use super::tick::{tick, Switches};
use super::{describe, Kind};

const ON: Switches = Switches { private: false, enabled: true };

fn ev(id: &str, title: &str, start: f64) -> CalEvent {
    CalEvent { id: id.into(), title: title.into(), start }
}
fn note(id: &str, title: &str, edited: f64, items: &[&str]) -> NoteInfo {
    let items: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    NoteInfo::new(id, title, edited, &items)
}

#[test]
fn a_future_event_is_mirrored_and_a_past_or_far_one_is_not() {
    let c = mem();
    let events = [
        ev("a", "Launch on Show HN", T0 + 5.0 * DAY),
        ev("past", "Yesterday's standup", T0 - HOUR),
        ev("far", "Next month", T0 + 20.0 * DAY),
    ];
    gather_senses(&c, T0, Some(&events), None).unwrap();
    let e = find(&c, "calendar", "a").unwrap().expect("mirrored");
    assert_eq!(e.kind, Kind::Event);
    assert_eq!(e.due_at, Some(T0 + 5.0 * DAY));
    assert!(find(&c, "calendar", "past").unwrap().is_none());
    assert!(find(&c, "calendar", "far").unwrap().is_none());
}

#[test]
fn an_event_that_goes_away_or_passes_is_closed_and_an_unreadable_calendar_closes_nothing() {
    let c = mem();
    gather_senses(&c, T0, Some(&[ev("a", "Dentist", T0 + DAY)]), None).unwrap();
    gather_senses(&c, T0 + 1.0, None, None).unwrap();
    assert_eq!(describe(&c, T0 + 2.0).unwrap()["count"], 1, "calendar down: left as it was");
    gather_senses(&c, T0 + 2.0, Some(&[]), None).unwrap();
    assert_eq!(describe(&c, T0 + 3.0).unwrap()["count"], 0);
}

#[test]
fn a_second_pass_changes_nothing() {
    let c = mem();
    let events = [ev("a", "Dentist", T0 + DAY)];
    let notes = [note("n.md", "Prep", T0 - HOUR, &["call dentist"])];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    let before = describe(&c, T0 + 1.0).unwrap();
    gather_senses(&c, T0 + 1.0, Some(&events), Some(&notes)).unwrap();
    assert_eq!(describe(&c, T0 + 1.0).unwrap()["count"], before["count"]);
}

#[test]
fn a_note_without_open_items_or_not_edited_lately_is_not_mirrored() {
    let c = mem();
    let notes = [
        note("done.md", "All ticked", T0 - HOUR, &[]),
        note("old.md", "Old list", T0 - 10.0 * DAY, &["something"]),
        note("live.md", "Live list", T0 - HOUR, &["one", "two"]),
    ];
    gather_senses(&c, T0, None, Some(&notes)).unwrap();
    assert!(find(&c, "notes", "done.md").unwrap().is_none());
    assert!(find(&c, "notes", "old.md").unwrap().is_none());
    let e = find(&c, "notes", "live.md").unwrap().expect("mirrored");
    assert_eq!(e.kind, Kind::NoteTodo);
}

#[test]
fn a_note_that_shares_a_word_with_a_near_event_is_linked_and_says_so() {
    let c = mem();
    let events = [
        ev("rec", "Record the 60 s demo video", T0 + DAY),
        ev("hn", "Launch Yantrik OS on Show HN", T0 + 5.0 * DAY),
    ];
    let notes = [
        note("lp.md", "Launch prep", T0 - HOUR, &["Script", "Demo video (not started)", "Post text"]),
        note("groceries.md", "Groceries", T0 - HOUR, &["milk"]),
    ];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    let d = describe(&c, T0 + 1.0).unwrap();
    let top = d["top"].as_array().unwrap();
    let lp = top.iter().find(|t| t["title"] == "Launch prep").expect("in the list");
    let reason = lp["reason"].as_str().unwrap();
    assert!(reason.contains("demo video") && reason.contains("Record the 60 s demo video"), "{reason}");
    assert!(reason.contains("3 unticked items") && reason.contains("1 day"), "{reason}");
    assert!(!reason.contains("not started"), "no note text in the reason: {reason}");
    let groceries = top.iter().find(|t| t["title"] == "Groceries").unwrap();
    assert!(!groceries["reason"].as_str().unwrap().contains("matches"));
    let rank = |t: &str| top.iter().position(|x| x["title"] == t).unwrap();
    assert!(rank("Launch prep") < rank("Groceries"), "the linked note outranks a bare list");
}

#[test]
fn an_event_beyond_three_days_does_not_link() {
    let c = mem();
    let events = [ev("hn", "Launch Yantrik OS on Show HN", T0 + 5.0 * DAY)];
    let notes = [note("lp.md", "Launch prep", T0 - HOUR, &["Script"])];
    gather_senses(&c, T0, Some(&events), Some(&notes)).unwrap();
    let d = describe(&c, T0).unwrap();
    let r = d["top"].as_array().unwrap().iter().find(|t| t["title"] == "Launch prep").unwrap()["reason"].as_str().unwrap().to_string();
    assert!(!r.contains("matches"), "{r}");
}

#[test]
fn the_notes_directory_is_read_for_titles_and_counts_only() {
    let dir = std::env::temp_dir().join(format!("agenda-notes-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("prep.md"), "# Launch prep\n- [x] done\n- [ ] Demo video (not started)\n- [ ] Script\n").unwrap();
    fs::write(dir.join("sub/nothing.md"), "# Nothing\n- [x] all done\n").unwrap();
    fs::write(dir.join("plain.txt"), "- [ ] not markdown\n").unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let c = mem();
    let report = tick(&c, now, ON, || Some(vec![]), Some(&dir)).unwrap().expect("ran");
    assert_eq!(report.senses.len(), 2);
    let e = find(&c, "notes", "prep.md").unwrap().expect("mirrored");
    assert_eq!(e.title, "Launch prep");
    assert!(find(&c, "notes", "sub/nothing.md").unwrap().is_none());
    let text: String = c
        .query_row("SELECT group_concat(title || coalesce(reason, ''), '|') FROM agenda", [], |r| r.get(0))
        .unwrap();
    assert!(!text.contains("Demo video") && !text.contains("not started"), "{text}");
    assert!(read_missing_dir_is_none(&dir));
    let _ = fs::remove_dir_all(&dir);
}

fn read_missing_dir_is_none(dir: &std::path::Path) -> bool {
    super::notes_dir::read_notes(&dir.join("absent"), 0.0, 1.0).is_none()
}

#[test]
fn private_mode_and_the_switch_gather_nothing_and_read_nothing() {
    let c = mem();
    make_promises(&c);
    add_promise(&c, "send the invoice", T0 + DAY);
    for sw in [Switches { private: true, enabled: true }, Switches { private: false, enabled: false }] {
        let r = tick(&c, T0, sw, || panic!("the calendar must not be read"), Some(std::path::Path::new("/nonexistent"))).unwrap();
        assert!(r.is_none());
    }
    let has_table: i64 =
        c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'agenda'", [], |r| r.get(0)).unwrap();
    assert_eq!(has_table, 0, "not even the table was made");
    tick(&c, T0, ON, || Some(vec![]), None).unwrap().expect("on: it runs");
    assert_eq!(describe(&c, T0).unwrap()["count"], 1, "the promise was gathered once on");
}

#[test]
fn the_think_arm_calls_the_tick_once() {
    let bridge = include_str!("../../../yantrik-ui/src/bridge.rs");
    assert_eq!(bridge.matches("agenda_tick::tick_all(").count(), 1, "one call, in the Think arm");
    let think = bridge.find("Ok(CompanionCommand::Think {").expect("the Think arm");
    let call = bridge.find("agenda_tick::tick_all(").unwrap();
    assert!(call > think && call - think < 2500, "the call is at the top of the Think arm");
    let module = include_str!("../../../yantrik-ui/src/agenda_tick.rs");
    assert_eq!(module.matches("agenda::tick(conn").count(), 1, "the shell goes through tick, which calls gather once");
    assert_eq!(include_str!("tick.rs").matches("gather(conn, now)").count(), 1);
}

#[test]
fn prepare_for_me_is_called_once_from_the_same_arm_and_only_through_its_switch() {
    let module = include_str!("../../../yantrik-ui/src/agenda_tick.rs");
    assert_eq!(module.matches("prepare_work::tick(").count(), 1, "one call, after the agenda's own pass");
    let work = include_str!("../../../yantrik-ui/src/prepare_work.rs");
    let gate = work.find("prepare_for_me_on").expect("the switch is read");
    let run = work.find("prepare_for_me(&item").expect("the job is started");
    assert!(gate < run, "the switch is read before anything runs");
    assert_eq!(work.matches("prepare_for_me(&item").count(), 1);
}
