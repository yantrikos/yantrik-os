//! The week and day views' arithmetic, tested without a desktop or a service. These cases came
//! from `tests/calendar-core`, which compiled this module by path; they live beside it now.

use chrono::NaiveDate;

use super::SourceEvent;

mod grid;
mod naming;
mod times;

pub(super) fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
}

// The id is the title here. The store's is a uuid7 and nothing in these cases turns on its
// shape, while a readable one makes a failed assertion say which event it was about.
pub(super) fn event(title: &str, start: &str, end: &str) -> SourceEvent {
    SourceEvent {
        id: title.into(),
        title: title.into(),
        start: start.into(),
        end: end.into(),
        is_all_day: false,
        color_index: 0,
    }
}

pub(super) fn all_day(title: &str, day: &str) -> SourceEvent {
    SourceEvent {
        id: title.into(),
        title: title.into(),
        start: format!("{day}T00:00:00"),
        end: format!("{day}T23:59:59"),
        is_all_day: true,
        color_index: 0,
    }
}
