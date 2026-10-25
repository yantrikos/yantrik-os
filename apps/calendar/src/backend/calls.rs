//! The calls this app makes, typed: each one the same request, the same parse and the same
//! sentence on failure as before the framework.

use chrono::NaiveDate;
use serde::de::DeserializeOwned;
use yantrik_ipc_contracts::calendar::{
    method, CalendarEvent, CalendarRevision, CreateEventParams, DeleteEventParams, EventsParams,
    GetEventParams, UpdateEventParams,
};

use super::CalendarApi;
use crate::events::{self, CalEvent};

fn params(value: impl serde::Serialize) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// The listing request for `from..=to`, both days whole.
fn range(from: NaiveDate, to: NaiveDate) -> Result<serde_json::Value, String> {
    params(EventsParams {
        start_date: format!("{}T00:00:00", from.format("%Y-%m-%d")),
        end_date: format!("{}T23:59:59", to.format("%Y-%m-%d")),
    })
}

fn call<T: DeserializeOwned>(
    api: &dyn CalendarApi,
    method_name: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    let result = api.call(method_name, params).map_err(|e| e.message)?;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

/// The store's events for `from..=to`, as the service sent them (with `location`, which the
/// window's own `CalEvent` has no use for). `events_between` reads these directly.
pub fn service_events(
    api: &dyn CalendarApi,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CalendarEvent>, String> {
    call(api, method::EVENTS, range(from, to)?)
}

/// Every event overlapping `from..=to`.
///
/// The range is a parameter because the week view needs one. The app asked for exactly the month
/// on screen, and a week straddling a month boundary is the common case at both ends of every
/// month: the first week of October would have been drawn with nothing on 28, 29 or 30 September
/// and said nothing about it. `views::visible_range` decides what to ask for.
pub fn events_in_range(
    api: &dyn CalendarApi,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CalEvent>, String> {
    Ok(events::from_store(&service_events(api, from, to)?))
}

/// [`events_in_range`] from a service already listening; it is never started for this.
pub fn events_in_range_running(
    api: &dyn CalendarApi,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CalEvent>, String> {
    let result = api.call_running(method::EVENTS, range(from, to)?).map_err(|e| e.message)?;
    let listed: Vec<CalendarEvent> = serde_json::from_value(result).map_err(|e| e.to_string())?;
    Ok(events::from_store(&listed))
}

/// What the store is at, or `None` when the question could not be put.
///
/// Only asked of a service that is already listening. Starting the calendar when it is down is
/// what `service::client` does, and a window sitting idle must never be the reason a service comes
/// up — that is the lesson `meeting_prep` learned on the companion's side, where a playbook
/// evaluated every think cycle would have started the calendar because it ran. It also keeps this
/// off the transport's circuit breaker: a failed connect trips it for the next few seconds, and a
/// check nobody asked for should not be able to make the next real call fail.
pub fn revision(api: &dyn CalendarApi) -> Option<CalendarRevision> {
    if !api.is_up() {
        return None;
    }
    let value = api.call_running(method::REVISION, serde_json::json!({})).ok()?;
    serde_json::from_value(value).ok()
}

/// Store a new event and answer with what the store kept: the id it gave the event and the
/// reminder lead it recorded. The stored event, with the id the store gave it, is the proof it
/// landed, not a hope.
pub fn create(api: &dyn CalendarApi, request: &CreateEventParams) -> Result<(String, u32), String> {
    let event: CalendarEvent = call(api, method::CREATE_EVENT, params(request)?)?;
    Ok((event.id, event.reminder_minutes))
}

pub fn delete(api: &dyn CalendarApi, event_id: &str) -> Result<(), String> {
    api.call(method::DELETE_EVENT, params(DeleteEventParams { id: event_id.to_string() })?)
        .map_err(|e| e.message)?;
    Ok(())
}

pub fn get(api: &dyn CalendarApi, event_id: &str) -> Result<CalendarEvent, String> {
    call(api, method::GET_EVENT, params(GetEventParams { id: event_id.to_string() })?)
}

/// Whether the store still holds this event: `Ok(true)`, `Ok(false)`, or `Err` when the question
/// could not be put at all.
///
/// The distinction matters exactly once, immediately after a delete. A service that has gone away
/// answers a read with an error too, and reading that as "it is gone" would be a delete reporting
/// an outcome it never observed — the fabrication the whole of `design/calendar-2026-09-20.md` is
/// about, pointing the other way. The store refuses an unknown id with `-32602`, which is an
/// answer; a transport failure is `-32000`, which is not.
pub fn still_there(api: &dyn CalendarApi, event_id: &str) -> Result<bool, String> {
    match api.call(method::GET_EVENT, params(GetEventParams { id: event_id.to_string() })?) {
        Ok(_) => Ok(true),
        Err(e) if e.code == -32602 => Ok(false),
        Err(e) => Err(e.message),
    }
}

pub fn update(api: &dyn CalendarApi, request: &UpdateEventParams) -> Result<CalendarEvent, String> {
    call(api, method::UPDATE_EVENT, params(request)?)
}
