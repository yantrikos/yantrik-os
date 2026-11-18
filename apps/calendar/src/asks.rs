//! What the surface's calls carry, read once into plain data: the event to add, the event a
//! delete names, the change an update asks for.
//!
//! The checks here are the ones the old handlers made before they touched the store, with the
//! same sentences and in the same order. A caller that breaks one is refused before anything is
//! started, and nothing is said in the window: these are the caller's mistakes, not the person's.

use chrono::NaiveDate;
use serde_json::Value;
use yantrik_ipc_contracts::calendar::MAX_REMINDER_MINUTES;

use crate::views;
use crate::who::Caller;

/// A `YYYY-MM-DD` in the shape the store matches a day by. Checked rather than let the service
/// reject a malformed timestamp: the error a caller can act on names the format it should have
/// used.
fn looks_like_a_date(date: &str) -> Result<(), String> {
    if date.len() != 10 || date.matches('-').count() != 2 {
        return Err(format!("`date` should look like 2026-09-06, not `{date}`"));
    }
    Ok(())
}

/// A text argument, trimmed, or `None` when absent or empty.
fn given(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `duration_min`, when given.
fn duration_arg(args: &Value) -> Result<Option<i32>, String> {
    match args.get("duration_min") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(v.as_i64().ok_or("`duration_min` must be a number of minutes")? as i32)),
    }
}

/// The `reminder_minutes` argument as both surface actions read it: absent is "did not say",
/// and anything present must be a number of minutes inside the contract's cap. Checked at the
/// door rather than passed to the service to refuse, for the same reason `date` is checked
/// here: the error a caller can act on names the range it should have stayed inside.
pub fn reminder_arg(args: &Value) -> Result<Option<u32>, String> {
    match args.get("reminder_minutes") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let m = v.as_i64().ok_or("`reminder_minutes` must be a number of minutes")?;
            if !(0..=i64::from(MAX_REMINDER_MINUTES)).contains(&m) {
                return Err(format!(
                    "`reminder_minutes` must be between 0 and {MAX_REMINDER_MINUTES}, and was {m}"
                ));
            }
            Ok(Some(m as u32))
        }
    }
}

/// An event to put on the calendar, from `add_event`.
#[derive(Clone, Debug, PartialEq)]
pub struct AddAsk {
    pub title: String,
    pub date: String,
    /// Empty for an all-day event, which has no clock.
    pub time: String,
    pub notes: String,
    pub duration_min: Option<i32>,
    pub all_day: bool,
    pub reminder_minutes: Option<u32>,
    pub caller: Caller,
}

impl AddAsk {
    pub fn parse(args: &Value, caller: Caller) -> Result<AddAsk, String> {
        let title = args["title"].as_str().unwrap_or_default().trim().to_string();
        let date = args["date"].as_str().unwrap_or_default().trim().to_string();
        let all_day = args["all_day"].as_bool().unwrap_or(false);
        if title.is_empty() {
            return Err("`title` is empty".into());
        }
        looks_like_a_date(&date)?;
        // Whether this call needed a `time` at all is decided here, not by the declaration:
        // `time` is optional on the surface because an all-day event has no clock to give, but a
        // timed one is nothing without it.
        let time = views::added_clock(args["time"].as_str(), all_day)?;
        let duration_min = duration_arg(args)?;
        if all_day && duration_min.is_some() {
            return Err("an all-day event has no length to set; drop `duration_min` or drop \
                        `all_day`"
                .into());
        }
        let reminder_minutes = reminder_arg(args)?;
        if all_day && reminder_minutes.is_some() {
            return Err("an all-day event is never announced; drop `reminder_minutes` or drop \
                        `all_day`"
                .into());
        }
        let notes = args["notes"].as_str().unwrap_or_default().to_string();
        Ok(AddAsk { title, date, time, notes, duration_min, all_day, reminder_minutes, caller })
    }
}

/// Which event a delete names: by id, or by exactly one title on one date. Never a guess.
///
/// The resolution both delete actions share, so the two cannot drift — `delete_event` and
/// `delete_own_event` must answer with the same event for the same words, and differ only in
/// the rule that runs afterwards. The store is asked about a title in the job
/// (`work::named_event`); what is checked here is the shape of the words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameAsk {
    Id(String),
    Titled { title: String, date: String, day: NaiveDate },
}

impl NameAsk {
    pub fn parse(args: &Value) -> Result<NameAsk, String> {
        match (given(args, "id"), given(args, "title"), given(args, "date")) {
            (Some(id), _, _) => Ok(NameAsk::Id(id)),
            (None, Some(title), Some(date)) => {
                looks_like_a_date(&date)?;
                let day = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .map_err(|_| format!("`{date}` is not a date"))?;
                Ok(NameAsk::Titled { title, date, day })
            }
            _ => Err("name the event by `id`, or by `title` and `date` together".into()),
        }
    }
}

/// What both update doors take, parsed once.
///
/// The split of #332 changes who may call, not what a call carries: `update_event`
/// (`sensitive`, any event) and `update_own_event` (`standard`, the caller's own) must read
/// the same call the same way and differ only in the ownership rule that runs afterwards —
/// the same arrangement `NameAsk` gives the two delete doors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateAsk {
    pub id: String,
    pub title: Option<String>,
    pub date: Option<String>,
    pub time: Option<String>,
    pub duration_min: Option<i32>,
    pub notes: Option<String>,
    pub reminder_minutes: Option<u32>,
}

impl UpdateAsk {
    pub fn parse(args: &Value) -> Result<UpdateAsk, String> {
        let id = given(args, "id").ok_or("`id` is empty")?;
        let date = given(args, "date");
        let time = given(args, "time");
        if let Some(date) = &date {
            looks_like_a_date(date)?;
        }
        if let Some(time) = &time {
            if !time.contains(':') {
                return Err(format!("`time` should look like 14:30, not `{time}`"));
            }
        }
        let duration_min = duration_arg(args)?;
        // `notes` is taken as given, empty string included: clearing the notes on an event is a
        // thing a caller may mean, and `given` would read that as "not said".
        let notes = args.get("notes").and_then(|v| v.as_str()).map(str::to_string);
        let reminder_minutes = reminder_arg(args)?;
        Ok(UpdateAsk { id, title: given(args, "title"), date, time, duration_min, notes, reminder_minutes })
    }
}
