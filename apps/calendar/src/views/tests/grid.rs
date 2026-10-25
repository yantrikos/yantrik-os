//! Where an event lands on the week and day grids, which days the store is asked for, and
//! `events_between`.

use super::{all_day, date, event};
use crate::views::{
    day_view, events_between, last_day_of_month, parse_range, selected_date, visible_range,
    week_bounds, week_view, ViewMode, WeekStart, MAX_RANGE_DAYS,
};

// -- Where an event lands ----------------------------------------

#[test]
fn an_event_is_in_the_column_and_at_the_hour_it_was_stored_at() {
    // 2026-09-22 is a Tuesday, so the third column of a week that starts on Sunday.
    let week = week_view(
        &[event("Standup", "2026-09-22T09:30:00", "2026-09-22T09:45:00")],
        date(2026, 9, 22), WeekStart::Sunday,
);
    assert_eq!(week.events.len(), 1);
    let block = &week.events[0];
    assert_eq!(block.day_index, 2);
    assert_eq!(block.start_hour, 9);
    assert_eq!(block.start_min, 30);
}

#[test]
fn a_blocks_height_is_the_minutes_between_its_start_and_its_end() {
    let week = week_view(
        &[event("Review", "2026-09-22T14:00:00", "2026-09-22T15:30:00")],
        date(2026, 9, 22), WeekStart::Sunday,
);
    assert_eq!(week.events[0].duration_min, 90);
}

#[test]
fn the_day_view_puts_everything_in_the_one_column_in_order() {
    let day = day_view(
        &[
            event("Later", "2026-09-22T16:00:00", "2026-09-22T17:00:00"),
            event("Earlier", "2026-09-22T09:00:00", "2026-09-22T10:00:00"),
        ],
        date(2026, 9, 22),
    );
    let titles: Vec<&str> = day.events.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Earlier", "Later"]);
    assert!(day.events.iter().all(|e| e.day_index == 0));
    assert_eq!(day.title, "Tuesday, 22 September 2026");
}

// -- Midnight ----------------------------------------------------

#[test]
fn an_event_running_past_midnight_is_clipped_at_the_day_and_continued_on_the_next() {
    let week = week_view(
        &[event("Deploy window", "2026-09-22T22:00:00", "2026-09-23T01:00:00")],
        date(2026, 9, 22), WeekStart::Sunday,
);
    assert_eq!(week.events.len(), 2, "one block per day it covers");

    let tuesday = &week.events[0];
    assert_eq!(tuesday.day_index, 2);
    assert_eq!((tuesday.start_hour, tuesday.start_min), (22, 0));
    assert_eq!(tuesday.duration_min, 120, "clipped at the end of Tuesday");

    let wednesday = &week.events[1];
    assert_eq!(wednesday.day_index, 3);
    assert_eq!((wednesday.start_hour, wednesday.start_min), (0, 0));
    assert_eq!(wednesday.duration_min, 60);
}

#[test]
fn the_day_view_shows_only_the_part_of_a_spanning_event_that_is_on_that_day() {
    let day = day_view(
        &[event("Deploy window", "2026-09-22T22:00:00", "2026-09-23T01:00:00")],
        date(2026, 9, 23),
    );
    assert_eq!(day.events.len(), 1);
    assert_eq!((day.events[0].start_hour, day.events[0].start_min), (0, 0));
    assert_eq!(day.events[0].duration_min, 60);
}

#[test]
fn an_event_ending_exactly_at_midnight_does_not_leave_an_empty_block_on_the_next_day() {
    let week = week_view(
        &[event("Evening", "2026-09-22T22:00:00", "2026-09-23T00:00:00")],
        date(2026, 9, 22), WeekStart::Sunday,
);
    assert_eq!(week.events.len(), 1);
    assert_eq!(week.events[0].duration_min, 120);
}

// -- What is not on the grid -------------------------------------

#[test]
fn an_event_outside_the_week_shown_is_not_in_it() {
    let events = [
        event("This week", "2026-09-22T09:00:00", "2026-09-22T10:00:00"),
        event("Next week", "2026-09-29T09:00:00", "2026-09-29T10:00:00"),
        event("Last week", "2026-09-15T09:00:00", "2026-09-15T10:00:00"),
    ];
    let week = week_view(&events, date(2026, 9, 22), WeekStart::Sunday);
    let titles: Vec<&str> = week.events.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["This week"]);
}

#[test]
fn an_all_day_event_is_not_given_an_hour_it_never_had() {
    let week = week_view(&[all_day("Company holiday", "2026-09-23")], date(2026, 9, 22), WeekStart::Sunday);
    assert!(week.events.is_empty(), "nothing on the hour grid");
    assert_eq!(week.all_day[3], ["Company holiday"]);
    // The column header is the only place on a grid of hours that can say so.
    assert_eq!(week.labels[3], "Wed 23 \u{b7} 1 all day");

    let day = day_view(&[all_day("Company holiday", "2026-09-23")], date(2026, 9, 23));
    assert!(day.events.is_empty());
    assert_eq!(day.all_day, ["Company holiday"]);
    assert_eq!(day.title, "Wednesday, 23 September 2026 \u{b7} 1 all day");
}

#[test]
fn an_event_whose_time_will_not_parse_is_skipped_rather_than_drawn_or_fatal() {
    let events = [
        event("Nonsense start", "next tuesday", "2026-09-22T10:00:00"),
        event("Nonsense end", "2026-09-22T09:00:00", "soon"),
        event("Backwards", "2026-09-22T15:00:00", "2026-09-22T14:00:00"),
        event("Empty", "", ""),
        event("Real", "2026-09-22T11:00:00", "2026-09-22T12:00:00"),
    ];
    let week = week_view(&events, date(2026, 9, 22), WeekStart::Sunday);
    let titles: Vec<&str> = week.events.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Real"]);

    let day = day_view(&events, date(2026, 9, 22));
    assert_eq!(day.events.len(), 1);
}

// -- The week, and where it starts -------------------------------

#[test]
fn the_week_starts_on_sunday_because_the_month_grid_does() {
    // calendar.slint draws Sun..Sat over the month, so a date's column has to be the same
    // number in both views or the same day is in two places.
    let (start, end) = week_bounds(date(2026, 9, 22), WeekStart::Sunday);
    assert_eq!(start, date(2026, 9, 20));
    assert_eq!(end, date(2026, 9, 26));

    // A Sunday is the start of its own week, not the end of the one before.
    assert_eq!(week_bounds(date(2026, 9, 20), WeekStart::Sunday).0, date(2026, 9, 20));
}

/// The bug on VM 520: a mind asked for the week Monday 28 September to Sunday 4 October, and
/// the only week view started on Sunday, so it always showed 27 Sep - 3 Oct.
#[test]
fn a_monday_week_runs_monday_to_sunday_and_shows_the_range_a_mind_asked_for() {
    let (start, end) = week_bounds(date(2026, 9, 28), WeekStart::Monday);
    assert_eq!((start, end), (date(2026, 9, 28), date(2026, 10, 4)));
    // A Sunday is the last day of its week, not the first of the next.
    assert_eq!(week_bounds(date(2026, 10, 4), WeekStart::Monday).0, date(2026, 9, 28));
    // The same date, on a Sunday week, is the 27th to the 3rd: the setting changes the answer.
    assert_eq!(week_bounds(date(2026, 9, 28), WeekStart::Sunday).0, date(2026, 9, 27));

    let week = week_view(&[], date(2026, 9, 30), WeekStart::Monday);
    assert_eq!(
        week.labels,
        ["Mon 28", "Tue 29", "Wed 30", "Thu 1", "Fri 2", "Sat 3", "Sun 4"]
    );
    // And the store is asked for the days that week needs.
    let (from, to) = visible_range(2026, 9, ViewMode::Week, 30, WeekStart::Monday);
    assert_eq!((from, to), (date(2026, 9, 1), date(2026, 10, 4)));
}

#[test]
fn an_event_lands_in_the_column_of_its_weekday_whichever_day_the_week_starts_on() {
    let events = [event("Standup", "2026-09-28T09:00:00", "2026-09-28T09:30:00")];
    let column = |first| week_view(&events, date(2026, 9, 28), first).events[0].day_index;
    assert_eq!((column(WeekStart::Sunday), column(WeekStart::Monday)), (1, 0));
}

// -- events_between ----------------------------------------------

fn listed_event(title: &str, start: &str, location: Option<&str>) -> yantrik_ipc_contracts::calendar::CalendarEvent {
    yantrik_ipc_contracts::calendar::CalendarEvent {
        id: title.into(),
        calendar_id: "default".into(),
        title: title.into(),
        description: String::new(),
        start: start.into(),
        end: start.replace("T09", "T10"),
        location: location.map(str::to_string),
        is_all_day: false,
        attendees: Vec::new(),
        recurrence: None,
        remote_id: None,
        creator: None,
        reminder_minutes: 10,
    }
}

#[test]
fn events_between_returns_exactly_the_in_range_events_across_a_month_boundary() {
    let events = [
        listed_event("Before", "2026-09-27T09:00:00", None),
        listed_event("October one", "2026-10-01T09:00:00", Some("Room 4")),
        listed_event("Last of September", "2026-09-30T09:00:00", None),
        listed_event("First day", "2026-09-28T09:00:00", None),
        listed_event("Last day", "2026-10-04T09:00:00", None),
        listed_event("After", "2026-10-05T09:00:00", None),
    ];
    let listed = events_between(&events, date(2026, 9, 28), date(2026, 10, 4));
    let titles: Vec<&str> = listed.iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["First day", "Last of September", "October one", "Last day"]);
    assert_eq!(listed[2]["date"], "2026-10-01");
    assert_eq!(listed[2]["start"], "2026-10-01T09:00:00");
    assert_eq!(listed[2]["end"], "2026-10-01T10:00:00");
    assert_eq!(listed[2]["location"], "Room 4");
    assert!(listed[0]["location"].is_null());
    assert!(events_between(&events, date(2026, 11, 1), date(2026, 11, 2)).is_empty());
}

#[test]
fn events_between_refuses_a_range_over_62_days_or_a_backwards_or_unreadable_one() {
    assert_eq!(MAX_RANGE_DAYS, 62);
    // 62 days inclusive is the most: 1 Sep to 2 Nov.
    assert!(parse_range("2026-09-01", "2026-11-01").is_ok());
    let err = parse_range("2026-09-01", "2026-11-02").unwrap_err();
    assert!(err.contains("63 days") && err.contains("62"), "{err}");
    assert!(parse_range("2026-10-04", "2026-09-28").unwrap_err().contains("before"));
    assert!(parse_range("2026-09-28", "2026-09-28").is_ok(), "one day is a range");
    assert!(parse_range("28 Sep", "2026-10-04").unwrap_err().contains("`from`"));
    assert!(parse_range("2026-09-28", "").unwrap_err().contains("`to`"));
}

#[test]
fn the_column_headers_read_as_the_dates_of_the_week() {
    let week = week_view(&[], date(2026, 9, 22), WeekStart::Sunday);
    assert_eq!(
        week.labels,
        ["Sun 20", "Mon 21", "Tue 22", "Wed 23", "Thu 24", "Fri 25", "Sat 26"]
    );
}

// -- A week that is in two months --------------------------------

#[test]
fn a_week_straddling_a_month_boundary_holds_events_from_both_months() {
    // 2026-10-01 is a Thursday, so its week runs Sun 27 September to Sat 3 October.
    let events = [
        event("September", "2026-09-29T09:00:00", "2026-09-29T10:00:00"),
        event("October", "2026-10-01T09:00:00", "2026-10-01T10:00:00"),
    ];
    let week = week_view(&events, date(2026, 10, 1), WeekStart::Sunday);
    let placed: Vec<(&str, i32)> =
        week.events.iter().map(|e| (e.title.as_str(), e.day_index)).collect();
    assert_eq!(placed, [("September", 2), ("October", 4)]);
}

#[test]
fn the_week_view_asks_the_store_for_the_days_the_week_needs_not_just_the_month() {
    // The app fetched exactly the month on screen, so the September half of this week was
    // never read and the week drew four empty columns without saying why.
    let (from, to) = visible_range(2026, 10, ViewMode::Week, 1, WeekStart::Sunday);
    assert_eq!(from, date(2026, 9, 27), "back to the Sunday the week starts on");
    assert_eq!(to, date(2026, 10, 31), "and still the whole month for the month grid");

    // The last week of a month reaches the other way.
    let (from, to) = visible_range(2026, 9, ViewMode::Week, 30, WeekStart::Sunday);
    assert_eq!(from, date(2026, 9, 1));
    assert_eq!(to, date(2026, 10, 3));

    // The month view asks for the month and no more.
    assert_eq!(
        visible_range(2026, 9, ViewMode::Month, 22, WeekStart::Sunday),
        (date(2026, 9, 1), date(2026, 9, 30))
    );
}

// -- Which day the week and day views are about ------------------

#[test]
fn a_month_arrived_at_with_nothing_picked_opens_on_its_first_day() {
    assert_eq!(selected_date(2026, 9, 0), date(2026, 9, 1));
    assert_eq!(selected_date(2026, 9, -1), date(2026, 9, 1));
}

#[test]
fn a_day_that_is_past_the_end_of_a_shorter_month_is_clamped_rather_than_lost() {
    assert_eq!(selected_date(2026, 9, 31), date(2026, 9, 30));
    assert_eq!(selected_date(2026, 2, 31), date(2026, 2, 28));
    assert_eq!(last_day_of_month(2024, 2), 29);
    assert_eq!(last_day_of_month(2026, 12), 31);
}
