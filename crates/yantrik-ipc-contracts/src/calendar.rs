//! Calendar service contract — event CRUD, sync, scheduling.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use crate::email::ServiceError;

/// How far ahead an event is announced when the event does not say. The old reminders timer had
/// exactly one lead, hard-coded to this, so events stored before `reminder_minutes` existed and
/// callers that do not send it keep the behaviour they have always had.
pub const DEFAULT_REMINDER_MINUTES: u32 = 10;

/// The farthest ahead a reminder may be set. One week: past that, "remind me about this" is a
/// different feature (a recurring nudge, a task list), and an absurd value a caller fat-fingers
/// should be refused at the door rather than stored into somebody's calendar.
pub const MAX_REMINDER_MINUTES: u32 = 7 * 24 * 60;

fn default_reminder_minutes() -> u32 {
    DEFAULT_REMINDER_MINUTES
}

/// Parse a stored event stamp: `2026-03-18T10:00:00`, or bare `2026-03-18` as the start of that
/// day. Local and naive — no timezone, no offset — which is how the calendar store writes them.
///
/// Here rather than in the calendar service because the event files now have two readers that
/// must agree on the format: the service that owns them, and the notifications service's
/// reminder timer, which reads them directly so reminders survive the calendar service not
/// running. Two copies of this parser in two crates is exactly the drift this crate exists to
/// prevent.
pub fn parse_stamp(s: &str) -> Option<NaiveDateTime> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt);
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0);
    }
    None
}

/// The names of the calendar service's JSON-RPC methods.
///
/// Here rather than spelled out at each call site, because the two ends of this wire drifted
/// apart while both looked correct on their own page.
pub mod method {
    pub const EVENTS: &str = "calendar.events";
    pub const GET_EVENT: &str = "calendar.get_event";
    pub const CREATE_EVENT: &str = "calendar.create_event";
    pub const UPDATE_EVENT: &str = "calendar.update_event";
    pub const DELETE_EVENT: &str = "calendar.delete_event";
    /// Store a remote calendar's event under the id it already has out there.
    pub const UPSERT_REMOTE: &str = "calendar.upsert_remote";
    /// "Has anything changed here?", in two numbers. Takes no parameters. See
    /// [`super::CalendarRevision`].
    pub const REVISION: &str = "calendar.revision";
}

/// The answer to [`method::REVISION`]: what the store is at, in two numbers.
///
/// It exists so that an open window does not have to re-list a month to find out whether anything
/// moved. The Calendar app re-read the store only when the date range on screen changed, which was
/// correct while the app was the only writer and stopped being correct the day this machine got one
/// calendar with several: an event the mind created through its own tools, or one Google sync
/// pulled in, or one a second caller added through the app's own surface, did not appear in an open
/// window until it navigated away and came back.
///
/// Both numbers are needed. `events` alone misses an edit in place, which changes no file's
/// existence; `newest_nanos` alone misses a create and a delete that land inside one filesystem
/// timestamp. `newest_nanos` covers the store directory's own modification time as well as every
/// event file's, because adding or removing an event touches the directory rather than any
/// surviving file.
///
/// Reading it never changes it, which is the property the whole arrangement rests on: a window
/// polling this must never be the reason it moves. It is also a `stat` per file and no parse, so
/// asking it is cheaper than the listing it exists to avoid.
///
/// What it cannot see: two writes that leave the count unchanged and land inside one filesystem
/// timestamp tick — an edit undone and redone within the same nanosecond. Nothing writes a
/// calendar that fast, and the alternative is a counter the service would have to keep, which a
/// file written by hand or restored from a backup would then be invisible to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarRevision {
    /// How many events the store holds.
    pub events: u64,
    /// The newest modification time under the store, in nanoseconds since the Unix epoch.
    ///
    /// Zero when nothing is stored, or when the times cannot be read at all — which compares
    /// equal to itself, so a machine that cannot answer this half reports changes on the count
    /// alone rather than reporting one every time it is asked.
    pub newest_nanos: u64,
}

/// Parameters for [`method::EVENTS`].
///
/// The request types below exist because this contract used to carry only the data types, and
/// each end wrote its own parameter names by hand. They disagreed: the app asked for `start` and
/// `end` where the service required `start_date` and `end_date`, so every listing failed and the
/// calendar could not show an event it had just stored; delete sent `event_id` where the service
/// read `id`. Both sides now build and parse the same struct, so a rename cannot land on one end
/// alone — it stops compiling on the other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsParams {
    /// Inclusive lower bound, `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS`.
    pub start_date: String,
    /// Inclusive upper bound, same formats. An event overlapping the range is included.
    pub end_date: String,
}

/// Parameters for [`method::GET_EVENT`].
///
/// One event by the id the store gave it. A caller that is about to change or remove an event
/// often has to know something about it first — whether it came from a remote calendar, and under
/// which id out there — and reading the whole month to find out is a worse question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetEventParams {
    pub id: String,
}

/// Parameters for [`method::CREATE_EVENT`].
///
/// `is_all_day` and `attendees` arrived with the companion's own calendar tools, which accepted
/// both long before this contract carried them. The stored [`CalendarEvent`] has always had a
/// home for each; the parameters simply did not reach it, so an all-day event asked for by the
/// mind was stored as a timed one and its attendees were dropped without a word. Both default, so
/// a caller that does not send them — the Calendar app — is unaffected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEventParams {
    pub title: String,
    /// ISO 8601, `YYYY-MM-DDTHH:MM:SS`.
    pub start: String,
    /// ISO 8601. An event that ends before it starts is refused.
    pub end: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub is_all_day: bool,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    /// Who created this event, in the one spelling every surface agrees on: the program the
    /// kernel's peer credentials lead to, or `agent <mind>:<conversation>` where the call
    /// carried an agent token (#201).
    ///
    /// Set by the creating surface from what the machine established about its caller — never
    /// from anything inside the request's own arguments, which a caller writes — and `None`
    /// for an event made from a window's own form, made by a caller nothing could identify, or
    /// made before this record existed. The store keeps it as stored; what it is worth is
    /// decided by the surface that reads it back, not here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// How many minutes before the start to announce this event. `None` means the caller did
    /// not say, and the store applies [`DEFAULT_REMINDER_MINUTES`]. All-day events are never
    /// announced: there is no time of day to announce them at.
    ///
    /// Optional and skipped when unset so a caller built against the old contract — one that
    /// never heard of reminders per event — sends the same bytes it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder_minutes: Option<u32>,
}

/// Parameters for [`method::UPDATE_EVENT`]. Every field but `id` is optional; those left out
/// keep the value the stored event already has.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateEventParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_all_day: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attendees: Option<Vec<Attendee>>,
    /// The id this event has on the calendar it came from, once it has one.
    ///
    /// Set after pushing a locally made event out to Google, so the next sync recognises it as
    /// the event it already has rather than storing a second copy beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    /// Move the event's reminder. `None` leaves whatever it has alone, like every other field
    /// here. See [`CreateEventParams::reminder_minutes`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder_minutes: Option<u32>,
}

/// Parameters for [`method::UPSERT_REMOTE`] — one event from a remote calendar, keyed by the id
/// it has out there.
///
/// Sync had no idempotent way in. It wrote Google's events into a private SQLite table, which the
/// Calendar app could not see, and the only shape the service offered was create — so syncing the
/// same week twice would have stored every event twice. Keying on `remote_id` makes a re-sync a
/// no-op on unchanged events and an edit on changed ones, and puts them where the app looks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertRemoteEventParams {
    /// The event's id on the remote calendar. Never the store's own id.
    pub remote_id: String,
    pub title: String,
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub is_all_day: bool,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
}

/// Parameters for [`method::DELETE_EVENT`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteEventParams {
    pub id: String,
}

/// A calendar event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub start: String,       // ISO 8601
    pub end: String,         // ISO 8601
    pub is_all_day: bool,
    pub location: Option<String>,
    pub attendees: Vec<Attendee>,
    pub recurrence: Option<String>,
    pub calendar_id: String,
    pub remote_id: Option<String>,
    /// Who created this event, when the surface that created it could establish one. See
    /// [`CreateEventParams::creator`].
    ///
    /// Kept in the event's own file and never changed by an update, so the record survives the
    /// calendar app — and this service — restarting between a create and a delete. `None` for
    /// every event stored before the field existed: a file with no `creator` key reads back as
    /// an event nobody is on record as having made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// How many minutes before the start this event is announced.
    ///
    /// Defaults on read, so an event file written before the field existed reads back with the
    /// same ten-minute lead the one fixed timer always used. The value is stored rather than
    /// left to the reader because the reader — the notifications service's reminder timer — is
    /// a different program from the writer, and a person who asked for a different lead asked
    /// for it once, not on every machine that happens to read the file.
    #[serde(default = "default_reminder_minutes")]
    pub reminder_minutes: u32,
}

/// An event attendee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    pub name: String,
    pub email: String,
    pub status: AttendeeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttendeeStatus {
    Pending,
    Accepted,
    Declined,
    Tentative,
}

/// A day cell for month view rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayCell {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub event_count: i32,
    pub is_today: bool,
    pub is_current_month: bool,
}

/// Calendar service operations.
pub trait CalendarService: Send + Sync {
    fn list_events(&self, calendar_id: &str, start: &str, end: &str) -> Result<Vec<CalendarEvent>, ServiceError>;
    fn get_event(&self, calendar_id: &str, event_id: &str) -> Result<CalendarEvent, ServiceError>;
    fn create_event(&self, calendar_id: &str, event: CalendarEvent) -> Result<CalendarEvent, ServiceError>;
    fn update_event(&self, calendar_id: &str, event: CalendarEvent) -> Result<CalendarEvent, ServiceError>;
    fn delete_event(&self, calendar_id: &str, event_id: &str) -> Result<(), ServiceError>;
    fn month_cells(&self, year: i32, month: u32) -> Result<Vec<DayCell>, ServiceError>;
    fn sync(&self, calendar_id: &str) -> Result<(), ServiceError>;
}
