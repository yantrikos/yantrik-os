//! The intents that move the window and read the store: `select_day`, `show_date`,
//! `events_between`, `show_month`, `go_to_today` and `set_view`. Each is answered in the command
//! where its own read lands (`answers.rs`).

use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Intent, Intents};

use super::{explicit, refused};
use crate::command::{Command, Step};
use crate::events;
use crate::model::Model;
use crate::views::{self, ViewMode};
use crate::Calendar;

/// The five that move about the month, in the order they were always published.
pub fn days_and_months(table: &mut Intents<Calendar>) {
    table.add(explicit(
        Intent::new(
            Action::new("select_day", "Show what is on one day of the month shown")
                .arg(Param::integer("day").describe("Day of the month, 1-31")),
            |a| {
                let day = a.raw()["day"].as_i64().ok_or_else(|| refused("`day` must be a number".into()))? as i32;
                if !(1..=31).contains(&day) {
                    return Err(refused(format!("{day} is not a day of the month")));
                }
                Ok(Command::SelectDay(day))
            },
        )
        .reply(answer),
    ));
    table.add(explicit(
        // Any day, in any month, in one call. The window could be moved a month at a time
        // (`show_month`) or back to today, and `select_day` picks a day of the month shown, so
        // "what is on 25 October" took counting months first (yantrik-mind, 2026-09-27).
        Intent::new(
            Action::new("show_date", "Show what is on one day, in whatever month it is")
                .arg(Param::text("date").describe("YYYY-MM-DD")),
            |a| {
                let date = a.raw()["date"].as_str().unwrap_or_default().trim().to_string();
                if events::ymd(&date).is_none() {
                    return Err(refused(format!("`date` should look like 2026-10-25, not `{date}`")));
                }
                Ok(Command::ShowDate(date))
            },
        )
        .reply(answer),
    ));
    table.add(explicit(
        // A read: it asks the store, not the grid, so a mind never has to steer the view
        // (`set_view`, `show_date`) to see a range.
        Intent::new(events_between_action(), |a| {
            let from = a.raw()["from"].as_str().unwrap_or_default();
            let to = a.raw()["to"].as_str().unwrap_or_default();
            Ok(Command::Between(views::parse_range(from, to).map_err(refused)?))
        })
        .reply(answer),
    ));
    table.add(explicit(
        Intent::new(
            Action::new("show_month", "Move to the next or previous month")
                .arg(Param::text("direction").describe("next | previous")),
            |a| match a.raw()["direction"].as_str().unwrap_or_default().to_lowercase().as_str() {
                "next" | "forward" => Ok(Command::ShowMonth(Step::Next)),
                "previous" | "prev" | "back" => Ok(Command::ShowMonth(Step::Previous)),
                other => Err(refused(format!("`direction` is next or previous, not `{other}`"))),
            },
        )
        .reply(answer),
    ));
    table.add(explicit(
        Intent::new(Action::new("go_to_today", "Jump back to the current month and day"), |_| Ok(Command::Today))
            .reply(answer),
    ));
}

/// `events_between`, as published. A function so a test can read the grade the surface will
/// carry: this is a read, and a mind reading must not need a standard grant or move the window.
pub fn events_between_action() -> Action {
    Action::new(
        "events_between",
        "List the events from one date to another, inclusive, up to 62 days, read from the \
         calendar store whatever the window is showing. Each has date, start, end, title and location",
    )
    .arg(Param::text("from").describe("First day, YYYY-MM-DD"))
    .arg(Param::text("to").describe("Last day, YYYY-MM-DD, at most 62 days after `from`"))
    .risk("safe")
}

/// `set_view`, published last, as it always was.
pub fn view_switch(table: &mut Intents<Calendar>) {
    table.add(explicit(
        Intent::new(
            Action::new("set_view", "Switch between the month, week and day views")
                .arg(Param::text("view").describe("month | week | day")),
            |a| match a.raw()["view"].as_str().unwrap_or_default().to_lowercase().as_str() {
                "month" => Ok(Command::SetView(ViewMode::Month)),
                "week" => Ok(Command::SetView(ViewMode::Week)),
                "day" => Ok(Command::SetView(ViewMode::Day)),
                other => Err(refused(format!("unknown view `{other}`; use month, week or day"))),
            },
        )
        .reply(answer),
    ));
}

/// What a reading intent answers: what `answers.rs` wrote when its read landed.
fn answer(m: &Model) -> serde_json::Value {
    m.answer.clone()
}
