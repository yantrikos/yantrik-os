//! Calendar tools — list events, create, update, delete.
//!
//! These are thin callers of `calendar-service`, the machine's one calendar. They used to be
//! local-first into a SQLite table of their own, which meant "put it on my calendar" landed
//! somewhere the Calendar app would never show, and an appointment made in the app was invisible
//! here. `design/calendar-2026-09-20.md` found that split and this is where it closes.
//!
//! Three rules follow from being a caller rather than a store, and each replaces something that
//! used to be done the other way:
//!
//! * **The service's parameters, never hand-written keys.** `yantrik_ipc_contracts::calendar`
//!   carries them. The app once asked for `start` where the service required `start_date`, and
//!   the calendar was blind for it; a tool writing its own JSON would be the same bug again.
//! * **Report what was observed.** Create answers with the id the service stored it under.
//!   Deleting or updating something that is not there is an error, in the service's words. A
//!   service that will not start is an error that says so.
//! * **No silent fallback.** There is nowhere else to write. A failure is a failure, and the
//!   person is told, because the alternative is what the September audit found: an appointment
//!   reported as kept and existing on no day.
//!
//! Google Calendar is still synced when an OAuth2 account is configured, and is a quiet no-op
//! when one is not. It now writes *through* the service — see `crate::calendar::sync`, which also
//! says what that sync does not do.

use std::sync::Arc;

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::calendar::backend::{CalendarBackend, ServiceCalendar};
use crate::calendar::{stamps, sync};
use crate::config::EmailAccountConfig;
use yantrik_ipc_contracts::calendar::{
    Attendee, AttendeeStatus, CalendarEvent, CreateEventParams, EventsParams, UpdateEventParams,
};

/// Register all calendar tools.
///
/// The calendar is `calendar-service`, running or startable. Google sync is optional: with no
/// OAuth2 account the tools work exactly as they do with one, minus the syncing.
pub fn register(reg: &mut ToolRegistry, accounts: Vec<EmailAccountConfig>, cal_account: Option<String>) {
    if !sync::configured(&accounts) {
        tracing::info!("Calendar: no OAuth2 account — the machine's calendar only, no Google sync");
    }
    let shared = Arc::new(Calendars {
        accounts,
        preferred: cal_account,
        backend: Arc::new(ServiceCalendar),
    });
    reg.register(Box::new(CalendarTodayTool { cal: shared.clone() }));
    reg.register(Box::new(CalendarListEventsTool { cal: shared.clone() }));
    reg.register(Box::new(CalendarCreateEventTool { cal: shared.clone() }));
    reg.register(Box::new(CalendarDeleteEventTool { cal: shared.clone() }));
    reg.register(Box::new(CalendarUpdateEventTool { cal: shared }));
}

/// What every calendar tool needs: the store, and whatever Google account syncs with it.
struct Calendars {
    accounts: Vec<EmailAccountConfig>,
    preferred: Option<String>,
    backend: Arc<dyn CalendarBackend>,
}

impl Calendars {
    fn google(&self) -> bool {
        sync::configured(&self.accounts)
    }

    fn token(&self) -> Result<String, String> {
        sync::token_for(&self.accounts, self.preferred.as_deref())
    }

    /// Bring Google's view of a window into the service before reading it back.
    ///
    /// Reading comes from the service either way. This only makes sure the service has been told
    /// about anything that was added at Google since the last pull. With no account configured it
    /// does not even open the companion's database: there is nothing to record.
    fn pull(&self, ctx: &ToolContext, from: chrono::NaiveDate, to: chrono::NaiveDate, force: bool) -> sync::Sync {
        if !self.google() {
            return sync::Sync::default();
        }
        sync::pull(
            &self.accounts,
            self.preferred.as_deref(),
            &ctx.db.conn(),
            self.backend.as_ref(),
            from,
            to,
            force,
        )
    }
}

// ── Reading and writing what a tool saw ──

/// One event, as a line a model can act on. The id first, because every other tool here takes it.
fn format_event(e: &CalendarEvent) -> String {
    let when = if e.is_all_day {
        format!("{} (all day)", day_of(&e.start))
    } else {
        // The day as well as the clock: these tools answer ranges, and a bare `14:00 - 15:00`
        // over a week is not enough to act on.
        format!("{} {} - {}", day_of(&e.start), clock(&e.start), clock(&e.end))
    };
    let mut line = format!("[{}] {} | {}", e.id, e.title, when);
    if let Some(loc) = e.location.as_deref().filter(|l| !l.is_empty()) {
        line.push_str(&format!(" @ {loc}"));
    }
    line
}

fn day_of(stamp: &str) -> &str {
    stamp.split('T').next().unwrap_or(stamp)
}

fn clock(stamp: &str) -> &str {
    stamp.split('T').nth(1).unwrap_or(stamp)
}

fn format_list(events: &[CalendarEvent], header: &str, detailed: bool) -> String {
    let mut out = format!("{header}\n\n");
    for e in events {
        out.push_str(&format_event(e));
        out.push('\n');
        if detailed {
            if let Some(note) = first_line(&e.description) {
                out.push_str(&format!("  Note: {note}\n"));
            }
            if !e.attendees.is_empty() {
                let who: Vec<&str> = e
                    .attendees
                    .iter()
                    .map(|a| if a.name.is_empty() { a.email.as_str() } else { a.name.as_str() })
                    .collect();
                out.push_str(&format!("  With: {}\n", who.join(", ")));
            }
        }
    }
    out
}

fn first_line(description: &str) -> Option<String> {
    let text = description.trim();
    if text.is_empty() {
        return None;
    }
    let one = text.lines().next().unwrap_or(text);
    Some(if one.chars().count() > 100 {
        format!("{}...", one.chars().take(97).collect::<String>())
    } else {
        one.to_string()
    })
}

/// The arguments a tool read, so the ones it did not can be named.
///
/// A tool that quietly drops what it was given is the fabrication this whole file was rewritten
/// over, one size smaller: `reminders: [...]` accepted and forgotten reads as a reminder that was
/// set. The calendar store has no home for reminders or recurrence, so the answer says so.
fn unread_arguments(args: &serde_json::Value, read: &[&str]) -> Vec<String> {
    let Some(map) = args.as_object() else { return Vec::new() };
    map.keys()
        .filter(|k| !read.contains(&k.as_str()))
        .cloned()
        .collect()
}

fn note_unstored(out: &mut String, args: &serde_json::Value, read: &[&str]) {
    let dropped = unread_arguments(args, read);
    if !dropped.is_empty() {
        out.push_str(&format!(
            "NOT STORED: {} — this calendar has no field for {}. Nothing was saved for {}.\n",
            dropped.join(", "),
            if dropped.len() == 1 { "it" } else { "them" },
            if dropped.len() == 1 { "it" } else { "them" },
        ));
    }
}

/// Attendees as the contract carries them, from whatever the model wrote.
///
/// Accepts `["ana@example.com"]` and `[{"name": "Ana", "email": "..."}]`, because both are things
/// a model writes and refusing one of them over its shape helps nobody.
fn attendees_from(args: &serde_json::Value) -> Vec<Attendee> {
    let Some(list) = args.get("attendees").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            if let Some(email) = item.as_str() {
                return Some(Attendee {
                    name: String::new(),
                    email: email.trim().to_string(),
                    status: AttendeeStatus::Pending,
                });
            }
            let email = item.get("email").and_then(|v| v.as_str())?;
            Some(Attendee {
                name: item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                email: email.trim().to_string(),
                status: AttendeeStatus::Pending,
            })
        })
        .filter(|a| !a.email.is_empty())
        .collect()
}

/// The range a listing asks the service for, as whole days.
fn range(from: chrono::NaiveDate, to: chrono::NaiveDate) -> Option<EventsParams> {
    let (start_date, _) = stamps::day_bounds(from)?;
    let (_, end_date) = stamps::day_bounds(to)?;
    Some(EventsParams { start_date, end_date })
}

fn parse_day(text: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%dT%H:%M:%S")
                .ok()
                .map(|dt| dt.date())
        })
}

/// A sync that ran and had something to say. Silence is the normal case and stays silent.
fn sync_note(outcome: &sync::Sync) -> Option<String> {
    outcome.note.as_ref().map(|n| format!("Google Calendar: {n}\n"))
}

/// A stored event's start and end in the shapes Google's API takes.
///
/// Dates for an all-day event, with the end as the day *after* the last one because that is how
/// Google reads it; an RFC 3339 instant with this machine's offset otherwise, because a naive
/// stamp with no offset is refused unless a time zone is sent beside it. The tools used to hand
/// Google whatever string the model had typed, which worked when the model happened to include
/// an offset and failed quietly when it did not.
fn google_bounds(event: &CalendarEvent) -> Option<(String, String)> {
    if event.is_all_day {
        return Some((
            stamps::date_of(&event.start).to_string(),
            stamps::day_after(&event.end)?,
        ));
    }
    Some((
        stamps::to_rfc3339_local(&event.start)?,
        stamps::to_rfc3339_local(&event.end)?,
    ))
}

// ── calendar_today ──

struct CalendarTodayTool {
    cal: Arc<Calendars>,
}

impl Tool for CalendarTodayTool {
    fn name(&self) -> &'static str { "calendar_today" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_today",
                "description": "Get TODAY's calendar events only. For tomorrow or other dates, use calendar_list_events instead.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let today = stamps::today();
        let pulled = self.cal.pull(ctx, today, today, false);
        self.run(sync_note(&pulled))
    }
}

impl CalendarTodayTool {
    /// Today, read from the calendar.
    ///
    /// Split from `execute` so the answer can be tested against a calendar that is not a socket,
    /// including one that refuses — which is the case that used to read as an empty day.
    fn run(&self, note: Option<String>) -> String {
        let today = stamps::today();
        let Some(params) = range(today, today) else {
            return format!("Could not work out the bounds of {today}.");
        };

        match self.cal.backend.list(&params) {
            Ok(events) => {
                let mut out = if events.is_empty() {
                    format!("No events scheduled for today ({today}).\nTip: if the user asked about a different date, use calendar_list_events with start_date and end_date parameters.\n")
                } else {
                    format_list(&events, &format!("Today's events ({today}) — {} events:", events.len()), false)
                };
                if let Some(note) = note {
                    out.push_str(&note);
                }
                out
            }
            Err(e) => format!("Could not read the calendar: {e}"),
        }
    }
}

// ── calendar_list_events ──

struct CalendarListEventsTool {
    cal: Arc<Calendars>,
}

impl Tool for CalendarListEventsTool {
    fn name(&self) -> &'static str { "calendar_list_events" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_list_events",
                "description": "List calendar events within a date range",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "start_date": {
                            "type": "string",
                            "description": "Start date in YYYY-MM-DD format. Defaults to today."
                        },
                        "end_date": {
                            "type": "string",
                            "description": "End date in YYYY-MM-DD format. Defaults to 7 days from start."
                        },
                        "query": {
                            "type": "string",
                            "description": "Filter events whose title, notes or location mention this text."
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of events to return. Default: 25."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        // The window is worked out twice: here, loosely, for the range to sync, and strictly in
        // `run` for the range to read and the arguments to refuse. That keeps `run` needing
        // nothing but the arguments and a calendar.
        let today = stamps::today();
        let from = args
            .get("start_date")
            .and_then(|v| v.as_str())
            .and_then(parse_day)
            .unwrap_or(today);
        let to = args
            .get("end_date")
            .and_then(|v| v.as_str())
            .and_then(parse_day)
            .unwrap_or_else(|| from + chrono::Duration::days(7));
        // A search wants what is there now, so it does not wait out the sync's freshness window.
        let pulled = self.cal.pull(ctx, from, to.max(from), args.get("query").is_some());
        self.run(args, sync_note(&pulled))
    }
}

impl CalendarListEventsTool {
    fn run(&self, args: &serde_json::Value, note: Option<String>) -> String {
        let today = stamps::today();
        let start = match args.get("start_date").and_then(|v| v.as_str()) {
            Some(text) => match parse_day(text) {
                Some(d) => d,
                None => return format!("`start_date` is not a date: {text}"),
            },
            None => today,
        };
        let end = match args.get("end_date").and_then(|v| v.as_str()) {
            Some(text) => match parse_day(text) {
                Some(d) => d,
                None => return format!("`end_date` is not a date: {text}"),
            },
            None => start + chrono::Duration::days(7),
        };
        if end < start {
            return format!("`end_date` ({end}) is before `start_date` ({start}).");
        }

        let query = args.get("query").and_then(|v| v.as_str()).map(str::to_lowercase);
        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(25) as usize;

        let Some(params) = range(start, end) else {
            return format!("Could not work out the bounds of {start}..{end}.");
        };

        match self.cal.backend.list(&params) {
            Ok(mut events) => {
                if let Some(needle) = &query {
                    // Filtered here rather than at Google, because the events are this machine's
                    // now and only some of them came from there.
                    events.retain(|e| {
                        e.title.to_lowercase().contains(needle)
                            || e.description.to_lowercase().contains(needle)
                            || e.location.as_deref().unwrap_or("").to_lowercase().contains(needle)
                    });
                }
                let total = events.len();
                events.truncate(max);
                let mut out = if events.is_empty() {
                    format!("No events found between {start} and {end}.\nTip: use calendar_create_event to add a new event.\n")
                } else {
                    let header = if total > events.len() {
                        format!("Events from {start} to {end} — {total} found, first {} shown:", events.len())
                    } else {
                        format!("Events from {start} to {end} — {total} found:")
                    };
                    format_list(&events, &header, true)
                };
                if let Some(note) = note {
                    out.push_str(&note);
                }
                out
            }
            Err(e) => format!("Could not read the calendar: {e}"),
        }
    }
}

// ── calendar_create_event ──

struct CalendarCreateEventTool {
    cal: Arc<Calendars>,
}

impl CalendarCreateEventTool {
    /// Read the arguments into the service's create parameters, or say what is missing.
    fn params(args: &serde_json::Value) -> Result<CreateEventParams, String> {
        let summary = args
            .get("summary")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: summary")?;
        let start = args
            .get("start")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: start")?;
        let end = args
            .get("end")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: end")?;
        let all_day = args.get("all_day").and_then(|v| v.as_bool()).unwrap_or(false);

        // The model writes times in whatever shape it likes — an offset, a bare date — and the
        // store keeps one. Converting here means a refusal names the argument rather than
        // arriving from the service as a sentence about a field the caller never wrote.
        let (start_stamp, end_stamp) = if all_day {
            stamps::all_day_bounds(start, end)
                .or_else(|| {
                    let s = stamps::to_store_stamp(start)?;
                    Some((s.clone(), stamps::to_store_stamp(end).unwrap_or(s)))
                })
                .ok_or_else(|| format!("`start` or `end` is not a date: {start} to {end}"))?
        } else {
            (
                stamps::to_store_stamp(start)
                    .ok_or_else(|| format!("`start` is not a date and time: {start}"))?,
                stamps::to_store_stamp(end)
                    .ok_or_else(|| format!("`end` is not a date and time: {end}"))?,
            )
        };

        Ok(CreateEventParams {
            title: summary.to_string(),
            start: start_stamp,
            end: end_stamp,
            description: args.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            location: args
                .get("location")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .filter(|s| !s.is_empty()),
            color: String::new(),
            is_all_day: all_day,
            attendees: attendees_from(args),
            // No creator: the companion's tools are not the calendar's surface, and the
            // record of who made an event belongs to the caller a surface verified (#201).
            // An event a mind makes through its own tools keeps needing `delete_event`,
            // which asks, until a tool call learns to establish its agent the same way.
            creator: None,
        })
    }
}

impl Tool for CalendarCreateEventTool {
    fn name(&self) -> &'static str { "calendar_create_event" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_create_event",
                "description": "Create a new calendar event on this machine's calendar, the one the Calendar app shows. Syncs to Google if an account is connected.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "Event title/name."
                        },
                        "start": {
                            "type": "string",
                            "description": "Start time in ISO8601 format (e.g., '2026-03-10T14:00:00') or a date for all-day (e.g., '2026-03-10')."
                        },
                        "end": {
                            "type": "string",
                            "description": "End time in ISO8601 format, or a date for all-day events."
                        },
                        "description": {
                            "type": "string",
                            "description": "Event description/notes."
                        },
                        "location": {
                            "type": "string",
                            "description": "Event location."
                        },
                        "all_day": {
                            "type": "boolean",
                            "description": "Whether this is an all-day event. Default: false."
                        },
                        "attendees": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Email addresses of the people invited."
                        }
                    },
                    "required": ["summary", "start", "end"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        self.run(args)
    }
}

impl CalendarCreateEventTool {
    fn run(&self, args: &serde_json::Value) -> String {
        let params = match Self::params(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        // The service first, always. It owns the event, and its answer carries the id everything
        // else on this machine will name the event by.
        let stored = match self.cal.backend.create(&params) {
            Ok(event) => event,
            Err(e) => return format!("The event was NOT created: {e}"),
        };

        let mut out = format!("Event created: {}\n", stored.title);
        out.push_str(&format!("ID: {}\n", stored.id));
        out.push_str(&format!("When: {} - {}\n", stored.start, stored.end));
        if let Some(loc) = stored.location.as_deref().filter(|l| !l.is_empty()) {
            out.push_str(&format!("Where: {loc}\n"));
        }
        if !stored.attendees.is_empty() {
            out.push_str(&format!("With: {}\n", stored.attendees.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", ")));
        }

        if self.cal.google() {
            out.push_str(&self.push_to_google(&stored));
        }

        note_unstored(
            &mut out,
            args,
            &["summary", "start", "end", "description", "location", "all_day", "attendees"],
        );
        out
    }

    /// Send a newly stored event out to Google, and record its id there.
    ///
    /// Recording the remote id is the half that matters: without it the next pull sees an event
    /// it has never been told about and stores a second copy beside this one.
    fn push_to_google(&self, stored: &CalendarEvent) -> String {
        let token = match self.cal.token() {
            Ok(t) => t,
            Err(e) => return format!("Not sent to Google Calendar: {e}\n"),
        };
        let Some((start, end)) = google_bounds(stored) else {
            return format!(
                "Kept here, not sent to Google Calendar: {} to {} is not a time this can be \
                 expressed as.\n",
                stored.start, stored.end
            );
        };
        let remote = crate::calendar::create_event(
            &token,
            None,
            &stored.title,
            &start,
            &end,
            Some(stored.description.as_str()).filter(|d| !d.is_empty()),
            stored.location.as_deref(),
            stored.is_all_day,
        );
        match remote {
            Ok(event) => {
                let record = self.cal.backend.update(&UpdateEventParams {
                    id: stored.id.clone(),
                    remote_id: Some(event.id.clone()),
                    ..Default::default()
                });
                match record {
                    Ok(_) => format!("Also on Google Calendar ({}).\n", event.id),
                    Err(e) => format!(
                        "On Google Calendar ({}), but this machine could not record that: {e}. \
                         The next sync will store it a second time.\n",
                        event.id
                    ),
                }
            }
            Err(e) => format!("Kept here, not sent to Google Calendar: {e}\n"),
        }
    }
}

// ── calendar_delete_event ──

struct CalendarDeleteEventTool {
    cal: Arc<Calendars>,
}

impl Tool for CalendarDeleteEventTool {
    fn name(&self) -> &'static str { "calendar_delete_event" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_delete_event",
                "description": "Delete a calendar event by its ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "event_id": {
                            "type": "string",
                            "description": "The event ID to delete, as calendar_today or calendar_list_events reported it."
                        }
                    },
                    "required": ["event_id"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        self.run(args)
    }
}

impl CalendarDeleteEventTool {
    fn run(&self, args: &serde_json::Value) -> String {
        let event_id = match args.get("event_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: event_id".to_string(),
        };

        // Read it before removing it: whether it came from Google, and under which id there, is
        // only knowable while it is still stored.
        let existing = match self.cal.backend.get(event_id) {
            Ok(event) => event,
            Err(e) => return format!("The event was NOT deleted: {e}"),
        };

        if let Err(e) = self.cal.backend.delete(event_id) {
            return format!("The event was NOT deleted: {e}");
        }

        let mut out = format!("Event {event_id} deleted: {}\n", existing.title);
        if let Some(remote_id) = existing.remote_id.as_deref() {
            if self.cal.google() {
                out.push_str(&match self.cal.token() {
                    Ok(token) => match crate::calendar::delete_event(&token, None, remote_id) {
                        Ok(()) => "Removed from Google Calendar too.\n".to_string(),
                        Err(e) => format!(
                            "Still on Google Calendar ({remote_id}): {e}. The next sync will put it back.\n"
                        ),
                    },
                    Err(e) => format!("Still on Google Calendar ({remote_id}): {e}\n"),
                });
            } else {
                out.push_str(&format!(
                    "It came from Google ({remote_id}) and no account is connected, so it was not removed there.\n"
                ));
            }
        }
        out
    }
}

// ── calendar_update_event ──

struct CalendarUpdateEventTool {
    cal: Arc<Calendars>,
}

impl Tool for CalendarUpdateEventTool {
    fn name(&self) -> &'static str { "calendar_update_event" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_update_event",
                "description": "Update an existing calendar event",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "event_id": {
                            "type": "string",
                            "description": "The event ID to update."
                        },
                        "summary": {
                            "type": "string",
                            "description": "New event title."
                        },
                        "start": {
                            "type": "string",
                            "description": "New start time (ISO8601)."
                        },
                        "end": {
                            "type": "string",
                            "description": "New end time (ISO8601)."
                        },
                        "description": {
                            "type": "string",
                            "description": "New description."
                        },
                        "location": {
                            "type": "string",
                            "description": "New location."
                        },
                        "all_day": {
                            "type": "boolean",
                            "description": "Whether this is an all-day event."
                        },
                        "attendees": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "The full list of attendee email addresses, replacing whatever is there."
                        }
                    },
                    "required": ["event_id"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        self.run(args)
    }
}

impl CalendarUpdateEventTool {
    fn run(&self, args: &serde_json::Value) -> String {
        let event_id = match args.get("event_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: event_id".to_string(),
        };

        let text = |key: &str| args.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let mut params = UpdateEventParams {
            id: event_id.to_string(),
            title: text("summary"),
            description: text("description"),
            location: text("location"),
            is_all_day: args.get("all_day").and_then(|v| v.as_bool()),
            attendees: args.get("attendees").map(|_| attendees_from(args)),
            ..Default::default()
        };
        for (key, slot) in [("start", &mut params.start), ("end", &mut params.end)] {
            if let Some(raw) = args.get(key).and_then(|v| v.as_str()) {
                match stamps::to_store_stamp(raw) {
                    Some(stamp) => *slot = Some(stamp),
                    None => return format!("`{key}` is not a date and time: {raw}"),
                }
            }
        }

        let updated = match self.cal.backend.update(&params) {
            Ok(event) => event,
            Err(e) => return format!("The event was NOT updated: {e}"),
        };

        let mut out = format!("Event updated: {}\n", updated.title);
        out.push_str(&format!("When: {} - {}\n", updated.start, updated.end));
        if let Some(loc) = updated.location.as_deref().filter(|l| !l.is_empty()) {
            out.push_str(&format!("Where: {loc}\n"));
        }

        if let Some(remote_id) = updated.remote_id.as_deref() {
            if self.cal.google() {
                // The whole event as it is stored now, not the arguments this call happened to
                // carry: a patch that sent only a new title would leave Google holding the old
                // time, and the next sync would bring it back.
                let (start, end) = match google_bounds(&updated) {
                    Some(pair) => (Some(pair.0), Some(pair.1)),
                    None => (None, None),
                };
                out.push_str(&match self.cal.token() {
                    Ok(token) => match crate::calendar::update_event(
                        &token,
                        None,
                        remote_id,
                        Some(updated.title.as_str()),
                        start.as_deref(),
                        end.as_deref(),
                        Some(updated.description.as_str()),
                        updated.location.as_deref(),
                        Some(updated.is_all_day),
                    ) {
                        Ok(_) => "Google Calendar updated too.\n".to_string(),
                        Err(e) => format!(
                            "Google Calendar still holds the old version ({remote_id}): {e}. \
                             The next sync will restore it.\n"
                        ),
                    },
                    Err(e) => format!("Google Calendar not updated ({remote_id}): {e}\n"),
                });
            }
        }

        note_unstored(
            &mut out,
            args,
            &["event_id", "summary", "start", "end", "description", "location", "all_day", "attendees"],
        );
        out
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use yantrik_ipc_contracts::calendar::UpsertRemoteEventParams;

    /// A calendar that lives in memory and can be told to refuse.
    ///
    /// The tools reach the real one over a socket, and a socket is not something a unit test
    /// should need. More to the point: the case that has to be exercised is the calendar saying
    /// no, because that is the case the old tools answered by writing somewhere else and
    /// reporting success.
    #[derive(Default)]
    struct FakeCalendar {
        events: Mutex<Vec<CalendarEvent>>,
        refuse: Option<String>,
    }

    impl FakeCalendar {
        fn refusing(reason: &str) -> Self {
            Self { events: Mutex::new(Vec::new()), refuse: Some(reason.to_string()) }
        }
        fn count(&self) -> usize {
            self.events.lock().unwrap().len()
        }
        fn check(&self) -> Result<(), String> {
            match &self.refuse {
                Some(reason) => Err(reason.clone()),
                None => Ok(()),
            }
        }
    }

    impl CalendarBackend for FakeCalendar {
        fn list(&self, _params: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
            self.check()?;
            Ok(self.events.lock().unwrap().clone())
        }
        fn get(&self, id: &str) -> Result<CalendarEvent, String> {
            self.check()?;
            self.events
                .lock()
                .unwrap()
                .iter()
                .find(|e| e.id == id)
                .cloned()
                .ok_or_else(|| format!("No event here with id {id}"))
        }
        fn create(&self, params: &CreateEventParams) -> Result<CalendarEvent, String> {
            self.check()?;
            let mut events = self.events.lock().unwrap();
            let event = CalendarEvent {
                id: format!("stored-{}", events.len() + 1),
                title: params.title.clone(),
                description: params.description.clone(),
                start: params.start.clone(),
                end: params.end.clone(),
                is_all_day: params.is_all_day,
                location: params.location.clone(),
                attendees: params.attendees.clone(),
                recurrence: None,
                calendar_id: "default".into(),
                remote_id: None,
                creator: params.creator.clone(),
            };
            events.push(event.clone());
            Ok(event)
        }
        fn update(&self, params: &UpdateEventParams) -> Result<CalendarEvent, String> {
            self.check()?;
            let mut events = self.events.lock().unwrap();
            let event = events
                .iter_mut()
                .find(|e| e.id == params.id)
                .ok_or_else(|| format!("No event here with id {}", params.id))?;
            if let Some(v) = &params.title {
                event.title = v.clone();
            }
            if let Some(v) = &params.start {
                event.start = v.clone();
            }
            if let Some(v) = &params.end {
                event.end = v.clone();
            }
            if let Some(v) = &params.remote_id {
                event.remote_id = Some(v.clone());
            }
            Ok(event.clone())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.check()?;
            let mut events = self.events.lock().unwrap();
            let before = events.len();
            events.retain(|e| e.id != id);
            if events.len() == before {
                return Err(format!("No event here with id {id}"));
            }
            Ok(())
        }
        fn upsert_remote(&self, params: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
            self.check()?;
            let mut events = self.events.lock().unwrap();
            if let Some(found) =
                events.iter_mut().find(|e| e.remote_id.as_deref() == Some(&params.remote_id))
            {
                found.title = params.title.clone();
                return Ok(found.clone());
            }
            let event = CalendarEvent {
                id: format!("stored-{}", events.len() + 1),
                title: params.title.clone(),
                description: params.description.clone(),
                start: params.start.clone(),
                end: params.end.clone(),
                is_all_day: params.is_all_day,
                location: params.location.clone(),
                attendees: params.attendees.clone(),
                recurrence: None,
                calendar_id: "default".into(),
                remote_id: Some(params.remote_id.clone()),
                creator: None,
            };
            events.push(event.clone());
            Ok(event)
        }
    }

    /// The tools over a given calendar, with no Google account configured.
    fn calendars(backend: Arc<dyn CalendarBackend>) -> Arc<Calendars> {
        Arc::new(Calendars { accounts: Vec::new(), preferred: None, backend })
    }

    fn create_args() -> serde_json::Value {
        serde_json::json!({
            "summary": "Launch review",
            "start": "2026-09-22T14:00:00",
            "end": "2026-09-22T15:00:00"
        })
    }

    // ── What the tools report ────────────────────────────────────────

    #[test]
    fn create_answers_with_the_id_the_calendar_stored_it_under() {
        let backend = Arc::new(FakeCalendar::default());
        let tool = CalendarCreateEventTool { cal: calendars(backend.clone()) };
        let answer = tool.run(&create_args());
        assert!(answer.contains("ID: stored-1"), "{answer}");
        assert_eq!(backend.count(), 1);
    }

    #[test]
    fn a_calendar_that_refuses_a_create_is_reported_as_a_failure_not_a_success() {
        // The September audit's finding in this file's own shape: the tool used to fall through
        // to a private store and answer "Event created (local)".
        let backend = Arc::new(FakeCalendar::refusing(
            "could not ask the desktop to start the calendar service: Binary not found",
        ));
        let tool = CalendarCreateEventTool { cal: calendars(backend.clone()) };
        let answer = tool.run(&create_args());
        assert!(answer.starts_with("The event was NOT created"), "{answer}");
        assert!(answer.contains("Binary not found"), "the reason has to survive: {answer}");
        assert_eq!(backend.count(), 0);
    }

    #[test]
    fn an_argument_the_calendar_has_no_field_for_is_reported_as_not_stored() {
        let tool = CalendarCreateEventTool { cal: calendars(Arc::new(FakeCalendar::default())) };
        let mut args = create_args();
        args["reminders"] = serde_json::json!([{ "minutes": 10 }]);
        args["recurrence"] = serde_json::json!("FREQ=WEEKLY");
        let answer = tool.run(&args);
        assert!(answer.contains("NOT STORED"), "{answer}");
        assert!(answer.contains("reminders"), "{answer}");
        assert!(answer.contains("recurrence"), "{answer}");
    }

    #[test]
    fn what_the_calendar_does_store_is_not_reported_as_dropped() {
        let backend = Arc::new(FakeCalendar::default());
        let tool = CalendarCreateEventTool { cal: calendars(backend.clone()) };
        let answer = tool.run(&serde_json::json!({
            "summary": "Retro", "start": "2026-09-22", "end": "2026-09-23",
            "all_day": true, "location": "Studio",
            "attendees": ["ana@example.com"]
        }));
        assert!(!answer.contains("NOT STORED"), "{answer}");
        let stored = backend.events.lock().unwrap()[0].clone();
        assert!(stored.is_all_day);
        assert_eq!(stored.location.as_deref(), Some("Studio"));
        assert_eq!(stored.attendees.len(), 1);
        assert_eq!(stored.attendees[0].email, "ana@example.com");
        // Google's exclusive end date becomes the last day this calendar draws it on.
        assert_eq!(stored.start, "2026-09-22T00:00:00");
        assert_eq!(stored.end, "2026-09-22T23:59:59");
    }

    #[test]
    fn a_time_with_an_offset_is_stored_as_a_time_this_calendar_keeps() {
        let backend = Arc::new(FakeCalendar::default());
        let tool = CalendarCreateEventTool { cal: calendars(backend.clone()) };
        tool.run(&serde_json::json!({
            "summary": "Call", "start": "2026-09-22T14:00:00+05:30",
            "end": "2026-09-22T15:00:00+05:30"
        }));
        let stored = backend.events.lock().unwrap()[0].clone();
        assert!(!stored.start.contains('+'), "the store parses one shape only: {}", stored.start);
        assert_eq!(stored.start.len(), "2026-09-22T14:00:00".len());
    }

    #[test]
    fn deleting_something_that_is_not_there_is_the_calendars_refusal_not_a_success() {
        let tool = CalendarDeleteEventTool { cal: calendars(Arc::new(FakeCalendar::default())) };
        let answer = tool.run(&serde_json::json!({ "event_id": "nope" }));
        assert!(answer.starts_with("The event was NOT deleted"), "{answer}");
        assert!(answer.contains("No event here with id nope"), "{answer}");
    }

    #[test]
    fn deleting_something_that_is_there_removes_it() {
        let backend = Arc::new(FakeCalendar::default());
        let cal = calendars(backend.clone());
        CalendarCreateEventTool { cal: cal.clone() }.run(&create_args());
        let answer =
            CalendarDeleteEventTool { cal }.run(&serde_json::json!({ "event_id": "stored-1" }));
        assert!(answer.contains("deleted"), "{answer}");
        assert_eq!(backend.count(), 0);
    }

    #[test]
    fn updating_something_that_is_not_there_is_reported_as_a_failure() {
        let tool = CalendarUpdateEventTool { cal: calendars(Arc::new(FakeCalendar::default())) };
        let answer = tool.run(&serde_json::json!({ "event_id": "nope", "summary": "Moved" }));
        assert!(answer.starts_with("The event was NOT updated"), "{answer}");
        assert!(answer.contains("No event here with id nope"), "{answer}");
    }

    #[test]
    fn a_calendar_that_cannot_be_read_is_said_rather_than_reported_as_an_empty_day() {
        // An empty answer and an unreachable calendar read the same to whoever asked, and only
        // one of them means there is nothing on today.
        let backend = Arc::new(FakeCalendar::refusing("the calendar service did not come up"));
        let answer = CalendarTodayTool { cal: calendars(backend) }.run(None);
        assert!(answer.contains("Could not read the calendar"), "{answer}");
        assert!(answer.contains("did not come up"), "{answer}");
    }

    #[test]
    fn today_lists_what_the_calendar_holds_with_the_id_to_act_on_it_by() {
        let backend = Arc::new(FakeCalendar::default());
        let cal = calendars(backend);
        let today = stamps::today();
        CalendarCreateEventTool { cal: cal.clone() }.run(&serde_json::json!({
            "summary": "Standup",
            "start": format!("{today}T09:00:00"),
            "end": format!("{today}T09:15:00")
        }));
        let answer = CalendarTodayTool { cal }.run(None);
        assert!(answer.contains("Standup"), "{answer}");
        assert!(answer.contains("[stored-1]"), "{answer}");
    }

    #[test]
    fn a_listing_that_cannot_read_the_calendar_says_so() {
        let backend = Arc::new(FakeCalendar::refusing("the calendar service did not come up"));
        let answer = CalendarListEventsTool { cal: calendars(backend) }.run(
            &serde_json::json!({ "start_date": "2026-09-01", "end_date": "2026-09-30" }),
            None,
        );
        assert!(answer.contains("Could not read the calendar"), "{answer}");
    }

    // ── The migration ────────────────────────────────────────────────

    /// One row in the old local table, written the way `create_local_event` used to write it.
    fn old_local_row(conn: &rusqlite::Connection, id: &str, summary: &str, start: &str, end: &str) {
        conn.execute(
            "INSERT INTO calendar_events (id, summary, description, location, start, end,
                                          is_all_day, status, html_link, cached_at, source)
             VALUES (?1, ?2, NULL, NULL, ?3, ?4, 0, 'confirmed', NULL, 0.0, 'local')",
            rusqlite::params![id, summary, start, end],
        )
        .expect("the old table takes a row");
    }

    fn old_table() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("a scratch database");
        crate::calendar::ensure_table(&conn);
        conn
    }

    #[test]
    fn the_migration_moves_local_events_once_and_only_once() {
        let conn = old_table();
        old_local_row(&conn, "local_a", "Dentist", "2026-09-22T10:00:00", "2026-09-22T11:00:00");
        old_local_row(&conn, "local_b", "Haircut", "2026-09-23T10:00:00", "2026-09-23T11:00:00");

        let backend = FakeCalendar::default();
        let first = crate::calendar::migrate::run(&conn, &backend);
        assert_eq!(first.pending, 2);
        assert_eq!(first.moved, 2);
        assert!(first.failed.is_empty(), "{:?}", first.failed);
        assert_eq!(backend.count(), 2);

        // A restart of the shell is not a second appointment.
        let second = crate::calendar::migrate::run(&conn, &backend);
        assert!(second.nothing_to_do(), "{second:?}");
        assert_eq!(backend.count(), 2);
    }

    #[test]
    fn a_migration_the_calendar_refuses_marks_nothing_and_is_tried_again() {
        let conn = old_table();
        old_local_row(&conn, "local_a", "Dentist", "2026-09-22T10:00:00", "2026-09-22T11:00:00");

        let down = FakeCalendar::refusing("the calendar service did not come up");
        let attempt = crate::calendar::migrate::run(&conn, &down);
        assert_eq!(attempt.pending, 1);
        assert_eq!(attempt.moved, 0);
        assert_eq!(attempt.failed.len(), 1);

        // Nothing was marked, so the appointment is still waiting rather than lost.
        let up = FakeCalendar::default();
        let retry = crate::calendar::migrate::run(&conn, &up);
        assert_eq!(retry.moved, 1);
        assert_eq!(up.count(), 1);
    }

    #[test]
    fn rows_cached_from_google_are_left_where_they_are() {
        // Only what the mind made locally has to be carried across. A Google event comes back
        // from Google, through the service, keyed on the id it has there.
        let conn = old_table();
        conn.execute(
            "INSERT INTO calendar_events (id, summary, description, location, start, end,
                                          is_all_day, status, html_link, cached_at, source)
             VALUES ('abc123', 'From Google', NULL, NULL, '2026-09-22T10:00:00',
                     '2026-09-22T11:00:00', 0, 'confirmed', NULL, 0.0, 'google')",
            [],
        )
        .unwrap();

        let backend = FakeCalendar::default();
        let report = crate::calendar::migrate::run(&conn, &backend);
        assert!(report.nothing_to_do(), "{report:?}");
        assert_eq!(backend.count(), 0);
    }

    #[test]
    fn a_local_row_whose_time_is_not_a_time_is_named_rather_than_silently_skipped() {
        let conn = old_table();
        old_local_row(&conn, "local_a", "Sometime", "next tuesday", "soon");
        let backend = FakeCalendar::default();
        let report = crate::calendar::migrate::run(&conn, &backend);
        assert_eq!(report.moved, 0);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].contains("Sometime"), "{:?}", report.failed);
    }
}
