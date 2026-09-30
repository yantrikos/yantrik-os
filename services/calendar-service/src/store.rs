//! The event store: one JSON file per event, under `~/.local/share/yantrik/calendar/`.
//!
//! Separate from the RPC wiring in `main.rs` so the rules that decide whether a day has
//! anything on it can be tested without a socket, a service manager, or a desktop. The
//! calendar's whole job lives here.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use yantrik_ipc_contracts::calendar::{
    parse_stamp, CalendarEvent, CalendarRevision, CreateEventParams, EventsParams,
    UpdateEventParams, UpsertRemoteEventParams, DEFAULT_REMINDER_MINUTES, MAX_REMINDER_MINUTES,
};
use yantrik_ipc_contracts::email::ServiceError;

fn failed(message: impl Into<String>) -> ServiceError {
    ServiceError { code: -32000, message: message.into() }
}

fn bad_request(message: impl Into<String>) -> ServiceError {
    ServiceError { code: -32602, message: message.into() }
}

/// Whether `id` can name a file in this store's folder and nothing else.
///
/// Every id this service hands out is a uuid7 — lowercase hex and dashes, 36 characters — and a
/// remote calendar's id never becomes a filename (it lives inside the event file and is found by
/// scanning), so a real id is a short run of letters, digits and dashes. Anything else is not an
/// id this store ever issued: separators, `..`, a leading dot and absurd lengths all fall
/// outside that set. Every door that builds a path from an id checks it before touching the
/// filesystem, because ids arrive from callers — over a socket that checks nobody (#161) as
/// well as over one that does — and an id that is really a path is a read or a write to any
/// `.json` the person owns. The rule notes-service's `note_path` keeps (#320).
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The refusal for an id that is not one, saying what ids are so a caller that spelled a path
/// learns the store is keyed by names.
fn not_an_id(id: &str) -> ServiceError {
    bad_request(format!(
        "`{id}` is not an event id; ids are the names list_events reports, never paths"
    ))
}

/// Parse an ISO 8601 datetime. Accepts `2026-03-18T10:00:00` and bare `2026-03-18`,
/// the latter as the start of that day. The one implementation lives in the contracts crate
/// because the reminder timer reads these files too, from another service.
pub fn parse_iso_datetime(s: &str) -> Option<NaiveDateTime> {
    parse_stamp(s)
}

/// What a requested reminder lead becomes on disk: the caller's value, the default when the
/// caller did not say, and a refusal for a lead past the cap rather than a silently shortened
/// one — the caller gets told what would not fit instead of finding out later.
fn resolve_reminder(minutes: Option<u32>) -> Result<u32, ServiceError> {
    match minutes {
        Some(m) if m > MAX_REMINDER_MINUTES => Err(bad_request(format!(
            "`reminder_minutes` ({m}) is longer than the cap of {MAX_REMINDER_MINUTES}"
        ))),
        Some(m) => Ok(m),
        None => Ok(DEFAULT_REMINDER_MINUTES),
    }
}

/// A path's modification time in nanoseconds since the Unix epoch, or 0 when it has none.
///
/// Zero for anything unreadable rather than an error: this feeds [`EventStore::revision`], whose
/// whole job is to be the cheapest question on the socket, and a filesystem that will not report a
/// time is a reason to fall back to the event count, not a reason to fail.
fn modified_nanos(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// The events on disk.
pub struct EventStore {
    dir: PathBuf,
}

impl EventStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Where the events live. `describe_view` reports it, so a caller reads the directory this
    /// store actually answers from rather than assuming the default one.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn event_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn read_event(&self, path: &Path) -> Option<CalendarEvent> {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn write_event(&self, event: &CalendarEvent) -> Result<(), ServiceError> {
        // The id read back out of a file is as untrusted as one off the socket: it names the
        // path this write lands on, and `update` writes the event a file carried — so a file
        // planted with a path for its id cannot aim a later write outside the folder either.
        if !valid_id(&event.id) {
            return Err(not_an_id(&event.id));
        }
        // The directory can be missing on a machine where nothing has been saved yet, and a
        // calendar that refuses the first event anyone gives it is not a calendar.
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| failed(format!("Cannot create {}: {e}", self.dir.display())))?;
        let data = serde_json::to_string_pretty(event)
            .map_err(|e| failed(format!("Failed to serialize event: {e}")))?;
        // Temp file and rename, never a write in place: these files are a person's calendar,
        // and an in-place write interrupted by a crash or a full disk leaves half an event
        // where the whole one used to be. A rename either happens or does not, and the old
        // file stays readable up to the instant the new one takes its place.
        let path = self.event_path(&event.id);
        let temp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&temp, data)
            .map_err(|e| failed(format!("Failed to write event: {e}")))?;
        if let Err(e) = std::fs::rename(&temp, &path) {
            // A failed rename must not leave the temp file behind to be mistaken for an
            // event by anything scanning the directory.
            let _ = std::fs::remove_file(&temp);
            return Err(failed(format!("Failed to store event: {e}")));
        }
        Ok(())
    }

    /// One event by id, or `None` if nothing is stored under it.
    ///
    /// An id that is not one is also `None`, decided before the filesystem is touched: both
    /// callers that return a `Result` — the raw method and [`Self::update`] — answer a miss
    /// with -32602, so a path spelled at this door is refused without ever being opened.
    pub fn get(&self, id: &str) -> Option<CalendarEvent> {
        if !valid_id(id) {
            return None;
        }
        self.read_event(&self.event_path(id))
    }

    /// Every event on disk, in no particular order. The whole directory.
    ///
    /// Only [`Self::find_by_remote_id`] uses this, and only because a remote id is not a filename:
    /// the store is keyed by the id it gave the event, and a sync arrives holding the other one.
    /// A calendar is tens to hundreds of small files, so the scan is cheaper than a second index
    /// that could disagree with the files.
    fn all_events(&self) -> Vec<CalendarEvent> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .filter_map(|p| self.read_event(&p))
            .collect()
    }

    /// What the store is at: how many events it holds, and the newest modification time under it.
    ///
    /// The question an open window asks instead of re-listing a month. It stats, it does not
    /// read: parsing every file to answer "has anything changed" would cost more than the listing
    /// it exists to avoid. The directory's own time is in the maximum because creating or removing
    /// an event touches the directory rather than any surviving file, and every file's is in it
    /// because editing one touches only that file.
    ///
    /// A missing directory is a calendar with nothing in it rather than an error, the same rule
    /// [`Self::list`] follows — and the token it answers with is the same one an empty directory
    /// gives, so the first event stored moves it.
    ///
    /// Reading never changes it. That is the property a caller polling this depends on, and the
    /// reason nothing here writes, touches or creates anything.
    pub fn revision(&self) -> CalendarRevision {
        let mut newest = modified_nanos(&self.dir);
        let mut events = 0u64;
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                events += 1;
                newest = newest.max(modified_nanos(&path));
            }
        }
        CalendarRevision { events, newest_nanos: newest }
    }

    /// The stored event carrying this remote id, if the machine has already seen it.
    pub fn find_by_remote_id(&self, remote_id: &str) -> Option<CalendarEvent> {
        self.all_events()
            .into_iter()
            .find(|e| e.remote_id.as_deref() == Some(remote_id))
    }

    /// Every event overlapping the requested range, oldest first.
    ///
    /// Overlap, not containment: an event that starts before the range and ends inside it is on
    /// those days and has to be listed, or a month view loses anything spanning its first day.
    pub fn list(&self, params: &EventsParams) -> Result<Vec<CalendarEvent>, ServiceError> {
        let range_start = parse_iso_datetime(&params.start_date)
            .ok_or_else(|| bad_request(format!("`start_date` is not a date: {}", params.start_date)))?;
        let range_end = parse_iso_datetime(&params.end_date)
            .ok_or_else(|| bad_request(format!("`end_date` is not a date: {}", params.end_date)))?;

        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            // Nothing saved yet is an empty calendar, not a broken one.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(failed(format!("Cannot read calendar dir: {e}"))),
        };

        let mut events = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(event) = self.read_event(&path) else { continue };
            let ev_start = parse_iso_datetime(&event.start);
            let ev_end = parse_iso_datetime(&event.end);
            let starts_before_range_ends = ev_start.map(|s| s <= range_end).unwrap_or(true);
            let ends_after_range_starts = ev_end.or(ev_start).map(|e| e >= range_start).unwrap_or(true);
            if starts_before_range_ends && ends_after_range_starts {
                events.push(event);
            }
        }
        events.sort_by(|a, b| a.start.cmp(&b.start));
        Ok(events)
    }

    /// Store a new event and return it as stored, with the id it was given.
    pub fn create(&self, params: &CreateEventParams) -> Result<CalendarEvent, ServiceError> {
        let title = params.title.trim();
        if title.is_empty() {
            return Err(bad_request("`title` is empty"));
        }
        let start = parse_iso_datetime(&params.start)
            .ok_or_else(|| bad_request(format!("`start` is not a date and time: {}", params.start)))?;
        let end = parse_iso_datetime(&params.end)
            .ok_or_else(|| bad_request(format!("`end` is not a date and time: {}", params.end)))?;
        if end < start {
            return Err(bad_request(format!(
                "`end` ({}) is before `start` ({})",
                params.end, params.start
            )));
        }
        let reminder_minutes = resolve_reminder(params.reminder_minutes)?;

        let event = CalendarEvent {
            id: uuid7::uuid7().to_string(),
            title: title.to_string(),
            description: params.description.clone(),
            start: params.start.clone(),
            end: params.end.clone(),
            is_all_day: params.is_all_day,
            location: params.location.clone(),
            attendees: params.attendees.clone(),
            recurrence: None,
            calendar_id: "default".to_string(),
            remote_id: None,
            // Who made this, as the creating surface verified them and nobody since can argue
            // with: the record `delete_own_event` reads back when a caller asks to take the
            // event off without anybody being asked (#201). Stored in the event's own file, so
            // it outlives the app that took the call. This service keeps what it is handed and
            // decides nothing from it — the rule sits in the surface, which is the only place
            // that knows who is asking right now.
            creator: params.creator.clone(),
            reminder_minutes,
        };
        self.write_event(&event)?;
        Ok(event)
    }

    /// Store an event that belongs to a remote calendar, once, under the id it has out there.
    ///
    /// Idempotent by `remote_id`: the first call stores it and gives it an id of ours, every call
    /// after that edits the same file. Without this, the only way in was `create`, and syncing a
    /// week twice would have left two of every event — which is the shape the companion's private
    /// SQLite cache was hiding rather than solving.
    ///
    /// The stored id is never changed, because other things may already point at it.
    pub fn upsert_remote(
        &self,
        params: &UpsertRemoteEventParams,
    ) -> Result<CalendarEvent, ServiceError> {
        if params.remote_id.trim().is_empty() {
            return Err(bad_request("`remote_id` is empty"));
        }
        let title = params.title.trim();
        if title.is_empty() {
            return Err(bad_request("`title` is empty"));
        }
        let start = parse_iso_datetime(&params.start)
            .ok_or_else(|| bad_request(format!("`start` is not a date and time: {}", params.start)))?;
        let end = parse_iso_datetime(&params.end)
            .ok_or_else(|| bad_request(format!("`end` is not a date and time: {}", params.end)))?;
        if end < start {
            return Err(bad_request(format!(
                "`end` ({}) is before `start` ({})",
                params.end, params.start
            )));
        }

        let existing = self.find_by_remote_id(&params.remote_id);
        let event = CalendarEvent {
            // Keep the id the store already gave it; only a first sight mints one.
            id: existing
                .as_ref()
                .map(|e| e.id.clone())
                .unwrap_or_else(|| uuid7::uuid7().to_string()),
            title: title.to_string(),
            description: params.description.clone(),
            start: params.start.clone(),
            end: params.end.clone(),
            is_all_day: params.is_all_day,
            location: params.location.clone(),
            attendees: params.attendees.clone(),
            recurrence: existing.as_ref().and_then(|e| e.recurrence.clone()),
            calendar_id: existing
                .as_ref()
                .map(|e| e.calendar_id.clone())
                .unwrap_or_else(|| "default".to_string()),
            remote_id: Some(params.remote_id.clone()),
            // A re-sync edits an event; it does not adopt it. An event a surface caller made
            // and later pushed out to a remote calendar keeps the creator it was stored with,
            // so a sync cannot hand somebody else's event to the syncer.
            creator: existing.as_ref().and_then(|e| e.creator.clone()),
            // Same rule as the creator: the remote calendar has no opinion about this event's
            // reminder — the wire does not even carry one — so a re-sync keeps the lead the
            // person set here instead of resetting it to the default every time Google syncs.
            reminder_minutes: existing
                .as_ref()
                .map(|e| e.reminder_minutes)
                .unwrap_or(DEFAULT_REMINDER_MINUTES),
        };
        self.write_event(&event)?;
        Ok(event)
    }

    /// Change a stored event. Fields left out keep what they had.
    pub fn update(&self, params: &UpdateEventParams) -> Result<CalendarEvent, ServiceError> {
        // Before the read this update starts from: the argument's id names the file to open,
        // and an id that is a path would open one outside the folder.
        if !valid_id(&params.id) {
            return Err(not_an_id(&params.id));
        }
        let mut event = self
            .get(&params.id)
            .ok_or_else(|| bad_request(format!("No event here with id {}", params.id)))?;

        if let Some(v) = &params.title {
            if v.trim().is_empty() {
                return Err(bad_request("`title` is empty"));
            }
            event.title = v.trim().to_string();
        }
        if let Some(v) = &params.start {
            parse_iso_datetime(v).ok_or_else(|| bad_request(format!("`start` is not a date and time: {v}")))?;
            event.start = v.clone();
        }
        if let Some(v) = &params.end {
            parse_iso_datetime(v).ok_or_else(|| bad_request(format!("`end` is not a date and time: {v}")))?;
            event.end = v.clone();
        }
        if let (Some(s), Some(e)) = (parse_iso_datetime(&event.start), parse_iso_datetime(&event.end)) {
            if e < s {
                return Err(bad_request(format!(
                    "`end` ({}) is before `start` ({})",
                    event.end, event.start
                )));
            }
        }
        if let Some(v) = &params.description {
            event.description = v.clone();
        }
        if let Some(v) = &params.location {
            event.location = Some(v.clone());
        }
        if let Some(v) = params.is_all_day {
            // The reminder must not be stopped silently (#332). The notifications service
            // announces timed events only, and every stored event carries a lead, so making a
            // timed event all-day would keep a `reminder_minutes` on file that never fires
            // again. Refused rather than applied, in a sentence that says what to do instead:
            // keep the time, or take the event off and add it again as an all-day one. The
            // other direction gains a reminder rather than losing one and stays allowed, and
            // `upsert_remote` — the sync's own door — sets the flag from the remote and is not
            // an update the caller chose.
            if v && !event.is_all_day {
                return Err(bad_request(format!(
                    "`{}` is a timed event announced {} minutes before it starts, and all-day \
                     events are never announced: making it all-day would silently stop the \
                     reminder. Leave `all_day` out to keep the time, or delete the event and \
                     add it again as an all-day one",
                    event.title, event.reminder_minutes
                )));
            }
            event.is_all_day = v;
        }
        if let Some(v) = &params.attendees {
            event.attendees = v.clone();
        }
        // Set once, when a locally made event has just been pushed out to a remote calendar and
        // has an id there. An update that does not mention it leaves it alone, so editing a synced
        // event does not quietly orphan it from the calendar it came from — which would show up
        // on the next sync as a duplicate rather than as the edit it was.
        if let Some(v) = &params.remote_id {
            event.remote_id = Some(v.clone());
        }
        if let Some(v) = params.reminder_minutes {
            event.reminder_minutes = resolve_reminder(Some(v))?;
        }

        self.write_event(&event)?;
        Ok(event)
    }

    /// Remove an event. Removing something that was never here is an error, not a success:
    /// the caller asked for a state change that did not happen.
    pub fn delete(&self, id: &str) -> Result<(), ServiceError> {
        // The one door that removes files: an id that is a path would remove one outside the
        // folder, which is what notes-service's delete did before #320.
        if !valid_id(id) {
            return Err(not_an_id(id));
        }
        let path = self.event_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(bad_request(format!("No event here with id {id}")))
            }
            Err(e) => Err(failed(format!("Failed to delete event: {e}"))),
        }
    }
}
