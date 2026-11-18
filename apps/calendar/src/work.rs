//! The work the jobs do, off the UI thread: reading the store again, and the three changes, each
//! read back from the store before it is reported.
//!
//! Pure over [`CalendarApi`]: no model, no window. A job is handed what it needs by value and
//! returns what it found; `update` decides what that means for the window.

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::NaiveDate;
use yantrik_ipc_contracts::calendar::{
    CalendarEvent, CalendarRevision, CreateEventParams, UpdateEventParams, MAX_REMINDER_MINUTES,
};

use crate::asks::{NameAsk, UpdateAsk};
use crate::backend::calls;
use crate::backend::CalendarApi;
use crate::events::{self, CalEvent};
use crate::views;

/// A range of days, both ends included.
pub type Range = (NaiveDate, NaiveDate);

/// What the window held when a read was asked for: the range its events were read for and the
/// store's revision then.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub range: Option<Range>,
    pub revision: Option<CalendarRevision>,
}

/// One read of the store, numbered in the order reads were taken.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub seq: u64,
    /// The range this read was for.
    pub wanted: Range,
    pub read: Read,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Read {
    /// Neither the range nor the store moved: what is in hand stands.
    Unmoved,
    Listed {
        events: Vec<CalEvent>,
        revision: Option<CalendarRevision>,
    },
    /// The listing failed. The range and the token are left unrecorded on purpose: the next
    /// redraw asks again, instead of a calendar that failed one read once staying empty until it
    /// is restarted.
    Failed(String),
}

/// Whether a read may start calendar-service to list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// A read somebody asked for: a navigation, a write's read-back, `describe`.
    MayStart,
    /// The twenty-second watch, which may be ticking in a window nobody is looking at: it lists
    /// only from a service already listening, and reads nothing from one that is down.
    Never,
}

/// Read the store again if the range wanted is not the one held, if the store has moved, or if
/// `force` says so.
///
/// This used to be the range test alone, which was right while the window was the only writer.
/// It is not: the mind's own calendar tools and Google sync go through the same service, and a
/// second caller can add something through this app's own surface while the person is on another
/// day. So the range test gained a second question — `calendar.revision`, two numbers and one
/// round trip to a local socket — and the month is re-listed when either says to.
///
/// The revision is asked *before* the listing, never after. A write landing between the two
/// leaves this holding the older token, so the next check re-reads once more than it had to; the
/// other order would record a token for events that did not include that write and never look
/// again. Being wrong in the cheap direction is the point.
pub fn reread(
    api: &dyn CalendarApi,
    sequence: &AtomicU64,
    wanted: Range,
    held: &Held,
    force: bool,
    start: Start,
) -> Listing {
    let seq = sequence.fetch_add(1, Ordering::SeqCst) + 1;
    let now = calls::revision(api);
    if start == Start::Never && now.is_none() {
        // Down, or not answering: nothing to list from without starting it.
        return Listing { seq, wanted, read: Read::Unmoved };
    }
    if now.is_none() && held.range == Some(wanted) {
        // The question could not be put, and the window already holds this range: keep it, even
        // for a read that would otherwise list whatever the revision says. Listing now would go
        // through the door that starts the service and, if that fails, throw a month away because
        // nothing is there to ask (delta review of #921, P14).
        return Listing { seq, wanted, read: Read::Unmoved };
    }
    let moved = match (&now, &held.revision) {
        (Some(now), Some(held)) => now != held,
        // Nothing to compare against: a first read, or one that failed and left no token behind.
        (Some(_), None) => true,
        // The question could not be put at all — the service is not listening, or did not answer.
        // Keep what is in hand rather than throwing a month away because nothing is there to ask.
        (None, _) => false,
    };
    if !force && held.range == Some(wanted) && !moved {
        return Listing { seq, wanted, read: Read::Unmoved };
    }
    let listed = match start {
        Start::MayStart => calls::events_in_range(api, wanted.0, wanted.1),
        Start::Never => calls::events_in_range_running(api, wanted.0, wanted.1),
    };
    let read = match listed {
        Ok(events) => Read::Listed { events, revision: now },
        Err(e) => Read::Failed(e),
    };
    Listing { seq, wanted, read }
}

/// The store's id for the event a delete names, and the name to use in a notice.
///
/// Read from the store now, not off the screen: the day named may not be the day the window is
/// showing, and the events in hand are only the visible range.
pub fn named_event(api: &dyn CalendarApi, ask: &NameAsk) -> Result<(String, String), String> {
    let (title, date, day) = match ask {
        NameAsk::Id(id) => return Ok((id.clone(), id.clone())),
        NameAsk::Titled { title, date, day } => (title, date, *day),
    };
    let on_that_day = calls::events_in_range(api, day, day)?;
    match views::named_on(&events::event_refs(&on_that_day), title, date) {
        views::Named::One(event) => Ok((event.id, format!("“{title}”"))),
        views::Named::None => Err(format!(
            "nothing called “{title}” on {date}; the day holds: {}",
            if on_that_day.is_empty() {
                "nothing".to_string()
            } else {
                on_that_day.iter().map(|e| format!("“{}”", e.title)).collect::<Vec<_>>().join(", ")
            }
        )),
        // Never a guess. Two events of one name on one day is an ordinary thing for a calendar to
        // hold, and picking one would remove the wrong appointment and report success — which is
        // the trash-icon bug 617dac9 fixed, rebuilt on the surface.
        views::Named::Ambiguous(candidates) => Err(format!(
            "{} events on {date} are called “{title}”; say which by id: {}",
            candidates.len(),
            candidates
                .iter()
                .map(|e| format!("{} at {}", e.id, e.start.split('T').nth(1).unwrap_or(&e.start)))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Take an event off the calendar and answer with its title, or say why it is still there.
///
/// Three round trips rather than one, all of them to a local socket and all of them necessary.
/// The event is read first so the answer can name what was removed rather than repeat an id back
/// at the caller. It is read again afterwards because "the service did not refuse" and "the event
/// is gone" are different sentences, and a calendar that answered the first while meaning the
/// second is the whole of `design/calendar-2026-09-20.md`. A read that cannot be made at all is
/// not read as "gone" — see `calls::still_there`.
pub fn delete_through(api: &dyn CalendarApi, event_id: &str) -> Result<String, String> {
    let event = calls::get(api, event_id)?;
    calls::delete(api, event_id)?;
    if calls::still_there(api, event_id)? {
        return Err(format!(
            "the calendar still holds “{}” under id {event_id} after deleting it",
            event.title
        ));
    }
    Ok(event.title)
}

/// Change an appointment, and answer with what the store holds afterwards — read back, not taken
/// from the reply the update handed us. `update_event` answers with the event it built, and this
/// calendar's history is of answers built from intentions.
pub fn update_through(api: &dyn CalendarApi, ask: &UpdateAsk) -> Result<CalendarEvent, String> {
    if let Some(m) = ask.reminder_minutes {
        if m > MAX_REMINDER_MINUTES {
            return Err(format!(
                "`reminder_minutes` cannot be longer than {MAX_REMINDER_MINUTES}, and was {m}"
            ));
        }
    }
    let title = ask.title.as_deref();
    // Read before written, because moving an event has to know how long it already runs. The
    // caller said "10:00", not "10:00 for an hour".
    let current = calls::get(api, &ask.id)?;
    let times = views::rescheduled(
        &current.start,
        &current.end,
        current.is_all_day,
        ask.date.as_deref(),
        ask.time.as_deref(),
        ask.duration_min,
    )
    .map_err(|e| format!("“{}”: {e}", current.title))?;

    if let Some(t) = title {
        if t.trim().is_empty() {
            return Err("an event needs a title".into());
        }
    }
    if title.is_none() && times.is_none() && ask.notes.is_none() && ask.reminder_minutes.is_none() {
        return Err(
            "nothing to change: give a `title`, a `date`, a `time`, a `duration_min`, `notes` \
             or `reminder_minutes`"
                .into(),
        );
    }

    let request = UpdateEventParams {
        id: ask.id.clone(),
        title: title.map(|t| t.trim().to_string()),
        start: times.as_ref().map(|(start, _)| start.clone()),
        end: times.as_ref().map(|(_, end)| end.clone()),
        description: ask.notes.clone(),
        reminder_minutes: ask.reminder_minutes,
        ..Default::default()
    };
    calls::update(api, &request)?;

    // Observed, from the store, after the write.
    let stored = calls::get(api, &ask.id)?;
    if let Some((start, end)) = &times {
        if &stored.start != start || &stored.end != end {
            return Err(format!(
                "asked the calendar for {start} to {end} and it kept {} to {}",
                stored.start, stored.end
            ));
        }
    }
    if let Some(t) = title {
        if stored.title != t.trim() {
            return Err(format!(
                "asked the calendar to call it “{}” and it kept “{}”",
                t.trim(),
                stored.title
            ));
        }
    }
    if let Some(m) = ask.reminder_minutes {
        if stored.reminder_minutes != m {
            return Err(format!(
                "asked the calendar to remind {m} minutes before it and it kept {}",
                stored.reminder_minutes
            ));
        }
    }
    Ok(stored)
}

/// A new event, as the store is asked to keep it.
///
/// The form has no field for a location, a colour or attendees, and this is not the place to add
/// one. `is_all_day` comes from the caller because the contract carries it and the surface offers
/// it; attendees have no way in from this app and are not invented here. `creator` is who is
/// asking, verified, and the record a later `delete_own_event` is checked against (#201): inside
/// a surface dispatch it is the caller the kernel and the reach file establish; from the window's
/// own form there is no caller on the socket and nothing is recorded, so an event a person typed
/// into the form is nobody's "own" but a person's.
pub struct NewEvent {
    pub title: String,
    pub start: String,
    pub end: String,
    pub notes: String,
    pub all_day: bool,
    pub reminder_minutes: Option<u32>,
    pub creator: Option<String>,
}

pub fn create(api: &dyn CalendarApi, new: &NewEvent) -> Result<(String, u32), String> {
    calls::create(
        api,
        &CreateEventParams {
            title: new.title.clone(),
            start: new.start.clone(),
            end: new.end.clone(),
            description: new.notes.clone(),
            location: None,
            color: String::new(),
            is_all_day: new.all_day,
            attendees: Vec::new(),
            creator: new.creator.clone(),
            reminder_minutes: new.reminder_minutes,
        },
    )
}
