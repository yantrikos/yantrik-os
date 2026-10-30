//! The one calendar, as the mind's tools see it.
//!
//! There were two. The Calendar app and the control surface used `calendar-service`, one JSON
//! file per event under `~/.local/share/yantrik/calendar/`; the companion's own calendar tools
//! kept a SQLite table of their own. Neither read the other, so "put it on my calendar", asked of
//! the companion, landed somewhere the Calendar app would never show, and an appointment made in
//! the app was invisible to the mind. `design/calendar-2026-09-20.md` found it and did not fix it.
//!
//! One owner per domain: `calendar-service` owns calendar data, and everything here is a caller.
//! The trait exists so the tools can be tested without a socket — the fake in the tests answers
//! the same six questions the service does — and so a tool's failure path is exercised rather
//! than assumed. It is deliberately the service's own shape: the typed parameters out of
//! `yantrik_ipc_contracts::calendar`, never hand-written JSON keys. Hand-written keys are what
//! left the app asking for `start` where the service required `start_date`, and that bug was
//! invisible from either file on its own.

use yantrik_ipc_contracts::calendar::{
    method, CalendarEvent, CreateEventParams, DeleteEventParams, EventsParams, GetEventParams,
    UpdateEventParams, UpsertRemoteEventParams,
};

/// What a calendar store has to be able to answer.
///
/// Every method returns the service's own reason on failure. A tool reports what it observed, so
/// a reason that is thrown away here becomes a tool that says "failed to update event" and means
/// nothing by it.
pub trait CalendarBackend: Send + Sync {
    fn list(&self, params: &EventsParams) -> Result<Vec<CalendarEvent>, String>;
    fn get(&self, id: &str) -> Result<CalendarEvent, String>;
    fn create(&self, params: &CreateEventParams) -> Result<CalendarEvent, String>;
    fn update(&self, params: &UpdateEventParams) -> Result<CalendarEvent, String>;
    fn delete(&self, id: &str) -> Result<(), String>;
    fn upsert_remote(&self, params: &UpsertRemoteEventParams) -> Result<CalendarEvent, String>;
}

/// The real one: `calendar-service` over its socket.
///
/// The service is registered `autostart: false` — the machine rail calls it "on demand" — so
/// every call starts it first if it is down. `yantrik_ipc_transport::service::client` is the same
/// function the Calendar app uses, and inside the shell it starts the service through the shell's
/// own ServiceManager without leaving the process. A service that cannot be started comes back as
/// an error naming what could not be reached, and the tool says so rather than falling back to a
/// private store, which is how the machine came to have two calendars.
pub struct ServiceCalendar;

impl ServiceCalendar {
    /// The transport's two seconds is for a service answering out of memory; this one reads and
    /// writes files, and may be cold when first called.
    const BUDGET_SECS: u64 = 10;

    const SERVICE: &'static str = "calendar";

    fn call<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        method_name: &str,
        params: &P,
    ) -> Result<R, String> {
        let client = yantrik_ipc_transport::service::client(Self::SERVICE)?.with_timeout(std::time::Duration::from_secs(Self::BUDGET_SECS));
        client.call_typed(method_name, params).map_err(|e| e.message)
    }
}

impl CalendarBackend for ServiceCalendar {
    fn list(&self, params: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        Self::call(method::EVENTS, params)
    }

    fn get(&self, id: &str) -> Result<CalendarEvent, String> {
        Self::call(method::GET_EVENT, &GetEventParams { id: id.to_string() })
    }

    fn create(&self, params: &CreateEventParams) -> Result<CalendarEvent, String> {
        Self::call(method::CREATE_EVENT, params)
    }

    fn update(&self, params: &UpdateEventParams) -> Result<CalendarEvent, String> {
        Self::call(method::UPDATE_EVENT, params)
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        let client = yantrik_ipc_transport::service::client(Self::SERVICE)?.with_timeout(std::time::Duration::from_secs(Self::BUDGET_SECS));
        // The reply is null; what matters is that the service did not refuse. Deleting something
        // that is not there is a refusal, and the caller is told so.
        client
            .call(
                method::DELETE_EVENT,
                serde_json::to_value(DeleteEventParams { id: id.to_string() })
                    .map_err(|e| e.to_string())?,
            )
            .map(|_| ())
            .map_err(|e| e.message)
    }

    fn upsert_remote(&self, params: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
        Self::call(method::UPSERT_REMOTE, params)
    }
}
