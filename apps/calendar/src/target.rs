//! What a call to `delete_event` or `update_event` acts on, in words a person can check.
//!
//! The shell's approval card asks this (`app.name_target`) for the action a mind wants to run,
//! and draws the rows in place of the raw id: a person asked "may this be deleted?" answers about
//! "Dentist, Fri 25 Sep 2026, 13:00–14:00", not about `01a0c718-…`. The rows come from the
//! store's own copy of the event — never from the request's words — and say whether the event
//! repeats, because deleting a repeating event deletes every occurrence of it: one file holds the
//! series, so there is no single occurrence to take away.

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, Offset, TimeZone};
use yantrik_app_runtime::control::Target;
use yantrik_ipc_contracts::calendar::CalendarEvent;

/// What the action does to the event, for the line that says how much of a series it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Delete,
    Change,
}

/// The card's rows for one stored event. `handles` are the argument names the event was found
/// by (`id`, or `title` and `date`), which the card leaves under Details.
pub fn rows(event: &CalendarEvent, act: Act, handles: &[&str]) -> Target {
    let recurring = event.recurrence.as_deref().is_some_and(|r| !r.trim().is_empty());
    let occurrences = match (recurring, act) {
        (true, Act::Delete) => format!(
            "the whole series: it repeats {}, and every occurrence is deleted",
            cadence(event.recurrence.as_deref().unwrap_or_default())
        ),
        (true, Act::Change) => format!(
            "the whole series: it repeats {}, and every occurrence changes",
            cadence(event.recurrence.as_deref().unwrap_or_default())
        ),
        (false, _) => "this event only; it does not repeat".to_string(),
    };
    Target {
        rows: vec![
            ("Event".into(), event.title.clone()),
            ("When".into(), when(event)),
            ("Calendar".into(), calendar(event)),
            ("Occurrences".into(), occurrences),
        ],
        series: recurring,
        handles: handles.iter().map(|h| h.to_string()).collect(),
    }
}

/// "Fri 25 Sep 2026, 13:00–14:00 (local time, UTC+01:00)", or "…, all day".
fn when(event: &CalendarEvent) -> String {
    let Some((start, zone)) = instant(&event.start) else {
        return event.start.clone();
    };
    let day = start.format("%a %-d %b %Y");
    if event.is_all_day {
        return format!("{day}, all day");
    }
    let end = instant(&event.end).map(|(end, _)| end);
    let span = match end {
        Some(end) if end.date() == start.date() => format!("{}\u{2013}{}", start.format("%H:%M"), end.format("%H:%M")),
        Some(end) if end > start => format!("{} \u{2013} {}", start.format("%H:%M"), end.format("%a %-d %b %H:%M")),
        _ => start.format("%H:%M").to_string(),
    };
    format!("{day}, {span} ({zone})")
}

/// The start as a wall-clock time and the zone it is in: the offset it was stored with (a synced
/// event), or this machine's own at that date (one made here, stored as local time).
fn instant(text: &str) -> Option<(NaiveDateTime, String)> {
    if let Ok(at) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Some((at.naive_local(), utc(at.offset().local_minus_utc())));
    }
    let naive = crate::views::parse_datetime(text)?;
    let offset = Local.from_local_datetime(&naive).earliest().map(|at| at.offset().fix().local_minus_utc()).unwrap_or(0);
    Some((naive, format!("local time, {}", utc(offset))))
}

fn utc(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let m = seconds.abs() / 60;
    format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)
}

/// Which calendar, as a person knows it: this computer's own, or the account it syncs with.
fn calendar(event: &CalendarEvent) -> String {
    let id = event.calendar_id.trim();
    match (event.remote_id.is_some(), id) {
        (false, "" | "default") => "on this computer".to_string(),
        (false, id) => id.to_string(),
        (true, "" | "default") => "synced from an online account".to_string(),
        (true, id) => format!("{id} (synced)"),
    }
}

/// "weekly", from an RRULE's FREQ; "on a schedule" for anything else.
fn cadence(rule: &str) -> &'static str {
    let freq = rule
        .split([';', ':', '\n'])
        .find_map(|part| part.trim().strip_prefix("FREQ="))
        .unwrap_or_default();
    match freq {
        "DAILY" => "daily",
        "WEEKLY" => "weekly",
        "MONTHLY" => "monthly",
        "YEARLY" => "yearly",
        _ => "on a schedule",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(recurrence: Option<&str>) -> CalendarEvent {
        CalendarEvent {
            id: "01a0c718-3931-7342-b9c7-8de36140ddb0".into(),
            title: "Dentist".into(),
            description: String::new(),
            start: "2026-09-25T13:00:00+01:00".into(),
            end: "2026-09-25T14:00:00+01:00".into(),
            is_all_day: false,
            location: None,
            attendees: Vec::new(),
            recurrence: recurrence.map(str::to_string),
            calendar_id: "default".into(),
            remote_id: None,
            creator: None,
            reminder_minutes: 10,
        }
    }

    fn row<'a>(t: &'a Target, label: &str) -> &'a str {
        t.rows.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str()).unwrap_or("")
    }

    #[test]
    fn a_single_event_is_named_by_title_date_time_zone_and_calendar() {
        let t = rows(&event(None), Act::Delete, &["id"]);
        assert_eq!(t.rows[0], ("Event".to_string(), "Dentist".to_string()), "the name comes first");
        assert_eq!(row(&t, "When"), "Fri 25 Sep 2026, 13:00\u{2013}14:00 (UTC+01:00)");
        assert_eq!(row(&t, "Calendar"), "on this computer");
        assert_eq!(row(&t, "Occurrences"), "this event only; it does not repeat");
        assert!(!t.series);
        assert_eq!(t.handles, ["id"]);
        assert!(!t.rows.iter().any(|(_, v)| v.contains("01a0c718")), "the raw id is not a row");
    }

    #[test]
    fn a_recurring_event_says_the_whole_series_goes() {
        let mut synced = event(Some("RRULE:FREQ=WEEKLY;BYDAY=FR"));
        synced.calendar_id = "pranab@example.com".into();
        synced.remote_id = Some("g-1".into());
        let t = rows(&synced, Act::Delete, &["id"]);
        assert!(t.series);
        assert_eq!(row(&t, "Occurrences"), "the whole series: it repeats weekly, and every occurrence is deleted");
        assert_eq!(row(&t, "Calendar"), "pranab@example.com (synced)");
        let changed = rows(&synced, Act::Change, &["id"]);
        assert!(row(&changed, "Occurrences").ends_with("every occurrence changes"));
    }

    #[test]
    fn local_and_all_day_events_read_as_a_person_would_say_them() {
        let mut local = event(None);
        local.start = "2026-09-25T09:30:00".into();
        local.end = "2026-09-25T10:00:00".into();
        let when = row(&rows(&local, Act::Delete, &["id"]), "When").to_string();
        assert!(when.starts_with("Fri 25 Sep 2026, 09:30\u{2013}10:00 (local time, UTC"), "{when}");
        let mut all_day = event(None);
        all_day.start = "2026-09-25".into();
        all_day.end = "2026-09-26".into();
        all_day.is_all_day = true;
        assert_eq!(row(&rows(&all_day, Act::Delete, &["id"]), "When"), "Fri 25 Sep 2026, all day");
    }
}
