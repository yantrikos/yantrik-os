//! The cards for a calendar event the companion is about to delete or change (#1311):
//! `calendar_delete_event` asks `delete_for_good` ("not recoverable") right before the
//! backend's delete, with the event's title, time and people; `calendar_update_event` asks
//! `change_item` right before the backend's update, with the event and, for every field the call
//! changes, what it is now and what it becomes. Titles, places and people come from whoever made
//! the event, so each is quoted.

use yantrik_companion_tools::asks::{act, quoted, Card, Question};
use yantrik_ipc_contracts::calendar::{Attendee, CalendarEvent, UpdateEventParams};

use crate::calendar::backend::CalendarBackend;

fn people(attendees: &[Attendee]) -> String {
    attendees
        .iter()
        .map(|a| if a.name.is_empty() { a.email.clone() } else { format!("{} <{}>", a.name, a.email) })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The event as the card shows it.
fn event(tool: &str, e: &CalendarEvent) -> Card {
    let mut card = Card::new(tool).text("title", &e.title).text("start", &e.start).text("end", &e.end);
    if !e.attendees.is_empty() {
        card = card.text("people", &people(&e.attendees));
    }
    match e.location.as_deref().filter(|l| !l.is_empty()) {
        Some(place) => card.text("location", place),
        None => card,
    }
}

/// `calendar_delete_event`: ask, then delete `id` (`existing` is what was read before). When its
/// Google copy goes too the card says so.
pub fn delete(backend: &dyn CalendarBackend, id: &str, existing: &CalendarEvent, google_too: bool) -> Result<(), String> {
    let mut card = event("calendar_delete_event", existing);
    if google_too && existing.remote_id.is_some() {
        card = card.plain("also", "its copy on Google Calendar is deleted too");
    }
    act(Question::DeleteForGood, &card, || backend.delete(id))
}

/// `{before} → {after}`, each quoted.
fn change(before: &str, after: &str) -> String {
    let q = |t: &str| quoted(t).as_str().unwrap_or_default().to_string();
    format!("{} → {}", q(before), q(after))
}

/// `calendar_update_event`: ask, then update. An event that cannot be read is an error and
/// nobody is asked.
pub fn update(backend: &dyn CalendarBackend, params: &UpdateEventParams) -> Result<CalendarEvent, String> {
    let before = backend.get(&params.id)?;
    let rows = [
        ("title", before.title.clone(), params.title.clone()),
        ("start", before.start.clone(), params.start.clone()),
        ("end", before.end.clone(), params.end.clone()),
        ("description", before.description.clone(), params.description.clone()),
        ("location", before.location.clone().unwrap_or_default(), params.location.clone()),
        ("all_day", before.is_all_day.to_string(), params.is_all_day.map(|b| b.to_string())),
        ("people", people(&before.attendees), params.attendees.as_deref().map(people)),
        ("reminder_minutes", before.reminder_minutes.to_string(), params.reminder_minutes.map(|m| m.to_string())),
    ];
    let card = rows
        .into_iter()
        .filter_map(|(key, was, now)| now.filter(|n| *n != was).map(|n| (key, change(&was, &n))))
        .fold(Card::new("calendar_update_event").text("event", &before.title), |card, (key, value)| card.plain(key, value));
    act(Question::ChangeItem, &card, || backend.update(params))
}
