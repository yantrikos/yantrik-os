//! Naming an event by title and date, and the name an approval card asks permission with.

use crate::views::{name_event, named_on, naming_index, EventRef, Named, NAMING_CAP};

// ── Naming an event a caller did not store ───────────────────────
//
// `delete_event` and `update_event` take the store's id, which a mind has after `add_event`
// or after reading `describe`. A person's instruction does not carry one — "drop the dentist
// on Thursday" — so a title and a date are the other way in, and the whole risk of that way
// is picking the wrong one silently.

fn stored(id: &str, title: &str, start: &str, end: &str) -> EventRef {
    EventRef {
        id: id.into(),
        title: title.into(),
        start: start.into(),
        end: end.into(),
        is_all_day: false,
    }
}

fn a_day() -> Vec<EventRef> {
    vec![
        stored("a", "Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00"),
        stored("b", "Dentist", "2026-09-22T11:00:00", "2026-09-22T12:00:00"),
        stored("c", "Dentist", "2026-09-23T11:00:00", "2026-09-23T12:00:00"),
    ]
}

#[test]
fn a_title_and_a_date_that_name_one_event_name_that_one() {
    match named_on(&a_day(), "Dentist", "2026-09-22") {
        Named::One(event) => assert_eq!(event.id, "b", "not the one on the next day"),
        other => panic!("expected exactly one, got {other:?}"),
    }
}

#[test]
fn the_title_is_matched_without_case_or_surrounding_space() {
    // What a person types, and what a model passes on from what a person typed.
    match named_on(&a_day(), "  dentist ", "2026-09-22") {
        Named::One(event) => assert_eq!(event.id, "b"),
        other => panic!("expected exactly one, got {other:?}"),
    }
}

#[test]
fn a_title_that_is_only_part_of_one_names_nothing() {
    // Deliberately not a substring match. "Stand" removing "Standup" is the beginning of
    // "standup" removing "standup with the platform team" on a day that has both.
    assert_eq!(named_on(&a_day(), "Stand", "2026-09-22"), Named::None);
    assert_eq!(named_on(&a_day(), "Standup", "2026-09-24"), Named::None);
}

#[test]
fn two_events_of_one_name_on_one_day_are_ambiguous_and_both_are_handed_back() {
    // The refusal that matters. Picking either would remove the wrong appointment half the
    // time and report success — which is the trash-icon bug 617dac9 fixed, rebuilt on the
    // control surface. The candidates come back so the caller can say which by id.
    let mut day = a_day();
    day.push(stored("d", "Dentist", "2026-09-22T16:00:00", "2026-09-22T17:00:00"));
    match named_on(&day, "Dentist", "2026-09-22") {
        Named::Ambiguous(candidates) => {
            let ids: Vec<&str> = candidates.iter().map(|e| e.id.as_str()).collect();
            assert_eq!(ids, vec!["b", "d"]);
            // And the times, because two ids alone do not tell a caller which is which.
            assert_eq!(candidates[0].start, "2026-09-22T11:00:00");
            assert_eq!(candidates[1].start, "2026-09-22T16:00:00");
        }
        other => panic!("expected an ambiguity, got {other:?}"),
    }
}

// ── The name an approval card asks permission with (#54) ─────────
//
// `delete_event` recommends the store's id and the grant is bound to it byte for byte —
// the reliable way in, and the one a person cannot read. `named_on` goes title and date
// to id; these go id back, which is what the describe's naming index is built from.

#[test]
fn a_timed_event_is_named_by_title_day_and_start() {
    // The event from the issue: a Friday afternoon appointment, asked away by its uuid.
    assert_eq!(
        name_event(&stored(
            "01a0c718-3931-7342-b9c7-8de36140ddb0",
            "Dentist",
            "2026-09-25T13:00:00",
            "2026-09-25T13:45:00"
        )),
        "Dentist, Fri 25 Sep 13:00"
    );
    assert_eq!(
        name_event(&stored("b", "Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00")),
        "Standup, Tue 22 Sep 09:00"
    );
}

#[test]
fn an_all_day_event_says_it_is_all_day() {
    // The grids leave these to the day header; reading out the midnight an all-day row is
    // stored at would be inventing a time nobody gave.
    let mut holiday = stored("d", "Company holiday", "2026-09-23T00:00:00", "2026-09-23T23:59:00");
    holiday.is_all_day = true;
    assert_eq!(name_event(&holiday), "Company holiday, Wed 23 Sep, all day");
}

#[test]
fn a_start_that_will_not_parse_adds_no_time() {
    // A file edited by hand can hold anything. The title alone is short; a made-up time
    // inside the sentence a person decides on would be worse than that.
    assert_eq!(name_event(&stored("m", "Mystery", "sometime", "whenever")), "Mystery");
}

#[test]
fn the_index_keeps_every_name_until_the_cap_then_drops_the_oldest() {
    let day = a_day();
    let whole = naming_index(&day);
    assert_eq!(whole.len(), day.len(), "an ordinary range keeps every name");
    assert!(
        whole.contains(&(day[0].id.clone(), name_event(&day[0]))),
        "and the values are `name_event`'s, not a second sentence for the same thing"
    );

    // A range far past the cap: one event an hour, oldest first, plus a hand-edited row
    // whose start will not parse — which counts as oldest of all (see `naming_index`).
    let mut events: Vec<EventRef> = (0..NAMING_CAP + 10)
        .map(|i| {
            stored(
                &format!("e{i}"),
                &format!("Event {i}"),
                &format!("2026-01-{:02}T{:02}:00:00", 1 + i / 24, i % 24),
                &format!("2026-01-{:02}T{:02}:00:00", 1 + i / 24, i % 24),
            )
        })
        .collect();
    events.push(stored("junk", "Hand-edited", "sometime", "whenever"));
    let index = naming_index(&events);
    assert_eq!(index.len(), NAMING_CAP, "the cap holds whatever the range");
    for gone in (0..10).map(|i| format!("e{i}")).chain(["junk".to_string()]) {
        assert!(
            !index.iter().any(|(id, _)| *id == gone),
            "{gone} is at the old end and was dropped"
        );
    }
    let newest = &events[NAMING_CAP + 9];
    assert!(
        index.contains(&(newest.id.clone(), name_event(newest))),
        "what an action taken now was read from keeps its name"
    );
}

// The index is worth only what the app actually publishes: dropping the key is exactly the quiet
// half of #54, where the card keeps working and stops saying what the thing is. That is read off
// the running app's describe: `tests::describe::the_describe_publishes_what_its_ids_stand_for`.
