//! The times a saved, added or moved event gets, and the two lines the form and `describe` say
//! about time.

use super::date;
use crate::views::{
    added_clock, all_day_bounds, rescheduled, start_and_end, timezone_label, today_line, week_view,
    SourceEvent, WeekStart,
};

// -- Saving ------------------------------------------------------

#[test]
fn an_events_end_is_its_start_plus_how_long_it_runs() {
    let (start, end) = start_and_end("2026-09-22", "14:00", 90).unwrap();
    assert_eq!(start, "2026-09-22T14:00:00");
    assert_eq!(end, "2026-09-22T15:30:00");
}

#[test]
fn a_late_event_ends_at_the_end_of_its_day_rather_than_at_a_time_that_does_not_exist() {
    // "23:30 plus an hour" was written as hour + 1 and produced T24:30:00, which the store
    // refuses outright -- so the evening appointment was simply not kept.
    let (_, end) = start_and_end("2026-09-22", "23:30", 60).unwrap();
    assert_eq!(end, "2026-09-22T23:59:00");
    let (_, end) = start_and_end("2026-09-22", "22:30", 120).unwrap();
    assert_eq!(end, "2026-09-22T23:59:00");
}

#[test]
fn a_date_or_a_time_that_will_not_parse_is_refused_rather_than_sent_to_the_store() {
    assert!(start_and_end("next friday", "14:00", 60).is_none());
    assert!(start_and_end("2026-09-22", "half past two", 60).is_none());
}

#[test]
fn the_timezone_strip_says_the_offset_it_can_actually_know() {
    assert_eq!(timezone_label(5 * 3600 + 1800), "Times are local, UTC+05:30");
    assert_eq!(timezone_label(0), "Times are local, UTC+00:00");
    assert_eq!(timezone_label(-(7 * 3600 + 1800)), "Times are local, UTC-07:30");
}

/// "What is on my calendar today" is the question a calendar exists to answer, and the
/// mind that asked it used to run `date` through the shell to learn which day today even
/// is — sensitive, so knowing the day raised an approval card (#207). The describe now
/// says it: the date and its weekday, in the shape the issue asked for.
#[test]
fn describe_says_which_day_today_is() {
    assert_eq!(today_line(date(2026, 9, 23)), "2026-09-23 Wednesday");
    assert_eq!(today_line(date(2026, 1, 4)), "2026-01-04 Sunday");

    // And the app really publishes it — the line above would keep passing if the field were
    // dropped from the describe. That is read off the running app's describe now, not off its
    // source: `tests::describe::the_describe_says_which_day_today_is`.
}

// ── Moving an appointment ────────────────────────────────────────

#[test]
fn moving_an_event_to_another_time_keeps_how_long_it_runs() {
    // "Move the standup to 10:00" says nothing about length. An update that reset it to the
    // default hour would be changing something nobody asked about.
    let moved = rescheduled(
        "2026-09-22T09:00:00",
        "2026-09-22T09:15:00",
        false,
        None,
        Some("10:00"),
        None,
    )
    .unwrap()
    .expect("a time was given, so it moves");
    assert_eq!(moved, ("2026-09-22T10:00:00".into(), "2026-09-22T10:15:00".into()));
}

#[test]
fn moving_an_event_to_another_day_keeps_its_time_and_its_length() {
    let moved =
        rescheduled("2026-09-22T09:00:00", "2026-09-22T09:15:00", false, Some("2026-10-01"), None, None)
            .unwrap()
            .expect("a date was given, so it moves");
    assert_eq!(moved, ("2026-10-01T09:00:00".into(), "2026-10-01T09:15:00".into()));
}

#[test]
fn a_length_given_is_the_length_it_gets_and_the_start_stays() {
    let moved =
        rescheduled("2026-09-22T09:00:00", "2026-09-22T09:15:00", false, None, None, Some(45))
            .unwrap()
            .expect("a duration was given, so it changes");
    assert_eq!(moved, ("2026-09-22T09:00:00".into(), "2026-09-22T09:45:00".into()));
}

#[test]
fn an_update_that_says_nothing_about_when_leaves_the_times_alone() {
    // A rename is an update too, and rebuilding the timestamps for one is how a rename comes
    // to move a meeting.
    assert_eq!(
        rescheduled("2026-09-22T09:00:00", "2026-09-22T09:15:00", false, None, None, None)
            .unwrap(),
        None
    );
}

#[test]
fn an_event_moved_late_in_the_day_ends_at_the_end_of_it_rather_than_at_a_time_that_is_not_one() {
    // The same clamp `start_and_end` applies on create: 23:30 plus an hour used to be
    // "24:30", which the store refuses outright.
    let moved = rescheduled(
        "2026-09-22T09:00:00",
        "2026-09-22T10:00:00",
        false,
        None,
        Some("23:30"),
        None,
    )
    .unwrap()
    .expect("a time was given");
    assert_eq!(moved.1, "2026-09-22T23:59:00");
}

#[test]
fn an_all_day_event_moved_to_another_day_is_still_all_day() {
    let moved =
        rescheduled("2026-09-22T00:00:00", "2026-09-22T23:59:00", true, Some("2026-09-25"), None, None)
            .unwrap()
            .expect("a date was given");
    assert_eq!(moved, ("2026-09-25T00:00:00".into(), "2026-09-25T23:59:00".into()));
    assert_eq!(all_day_bounds("2026-09-25").unwrap(), moved);
}

#[test]
fn a_time_on_an_all_day_event_is_refused_rather_than_quietly_making_it_a_timed_one() {
    // Applying it would turn a day-long event into a one-hour appointment at a time nobody
    // chose, and the caller would be told it had been moved.
    let refused =
        rescheduled("2026-09-22T00:00:00", "2026-09-22T23:59:00", true, None, Some("09:00"), None)
            .unwrap_err();
    assert!(refused.contains("all day"), "{refused}");
    let refused =
        rescheduled("2026-09-22T00:00:00", "2026-09-22T23:59:00", true, None, None, Some(30))
            .unwrap_err();
    assert!(refused.contains("all day"), "{refused}");
}

#[test]
fn an_update_that_cannot_be_worked_out_says_which_part_it_could_not_read() {
    // A file edited by hand can hold anything. The caller gets the reason rather than a
    // guessed time or a panic in a dispatch.
    let refused =
        rescheduled("2026-09-22T09:00:00", "whenever", false, None, Some("10:00"), None)
            .unwrap_err();
    assert!(refused.contains("duration_min"), "it has to name the way out: {refused}");

    let refused =
        rescheduled("whenever", "2026-09-22T10:00:00", false, None, Some("10:00"), None)
            .unwrap_err();
    assert!(refused.contains("starts at"), "{refused}");

    let refused =
        rescheduled("2026-09-22T09:00:00", "2026-09-22T10:00:00", false, Some("next friday"), None, None)
            .unwrap_err();
    assert!(refused.contains("next friday"), "{refused}");

    let refused =
        rescheduled("2026-09-22T09:00:00", "2026-09-22T10:00:00", false, None, None, Some(-5))
            .unwrap_err();
    assert!(refused.contains("negative"), "{refused}");
}

#[test]
fn a_whole_day_runs_from_midnight_to_the_last_minute_of_it() {
    assert_eq!(
        all_day_bounds("2026-09-22").unwrap(),
        ("2026-09-22T00:00:00".to_string(), "2026-09-22T23:59:00".to_string())
    );
    assert!(all_day_bounds("the 22nd").is_none());
}

#[test]
fn an_all_day_add_needs_no_time() {
    // The describe says `time` is not used with `all_day`, and the call sent that way used
    // to be refused as "needs argument `time`" before it reached anything (#297). A
    // whole-day event has no clock: the answer is the empty one, and a `time` that arrived
    // anyway is not consulted, exactly as `all_day` promises.
    assert_eq!(added_clock(None, true).unwrap(), "");
    assert_eq!(added_clock(Some("14:30"), true).unwrap(), "");
}

#[test]
fn a_timed_add_without_a_time_is_refused_in_a_sentence_that_names_time() {
    // The optionality stops where the whole day stops: an event at a particular hour is
    // nothing without it. The refusal is the handler's own sentence, not the format
    // complaint about an empty string, and it points at the way out.
    let refused = added_clock(None, false).unwrap_err();
    assert!(refused.contains("`time`"), "{refused}");
    assert!(refused.contains("all_day"), "{refused}");
    let refused = added_clock(Some("   "), false).unwrap_err();
    assert!(refused.contains("`time`"), "{refused}");
    // A `time` that is there but cannot hold a clock keeps its format refusal.
    let refused = added_clock(Some("1430"), false).unwrap_err();
    assert!(refused.contains("14:30") && refused.contains("1430"), "{refused}");
    assert_eq!(added_clock(Some(" 14:30 "), false).unwrap(), "14:30");
}

#[test]
fn an_all_day_event_the_app_stores_is_one_the_grid_leaves_alone() {
    // The two halves agreeing: what `all_day_bounds` writes is what `all_day_columns` reads,
    // so an all-day event added through the surface is on its day's header and not given an
    // hour it never had.
    let (start, end) = all_day_bounds("2026-09-22").unwrap();
    let event = SourceEvent {
        id: "a".into(),
        title: "Conference".into(),
        start,
        end,
        is_all_day: true,
        color_index: 0,
    };
    let week = week_view(&[event], date(2026, 9, 22), WeekStart::Sunday);
    assert!(week.events.is_empty(), "an all-day event is not on the hour grid");
    assert_eq!(week.all_day[2], vec!["Conference".to_string()], "Tuesday's column");
}
