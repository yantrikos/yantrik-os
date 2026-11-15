//! The meeting brief's one interrupt: "Orion launch sync in 10 minutes. Your brief is ready."
//!
//! The shell makes the brief and draws its card; the announcement is the notification service's
//! to grant, through the same path the calendar's own reminders take (`Store::add_os_notice_at`, as
//! the reserved `desktop` harness, family `calendar`). So the day's budget, quiet hours, quiet
//! initiative and the person's On / Inbox only / Off for Calendar all apply, and a toast held
//! past the start is never shown (`useful_until` is the start). Nothing else posts it: the shell
//! asks, from the desktop only, and never toasts a brief itself.
//!
//! One interrupt per event and start. The event's own reminder, its early "leave early" heads-up
//! and this notice are cards for one meeting, so whichever is second does not interrupt again:
//! - the brief's notice exists and was granted an interrupt, or it took the place of an early card
//!   the shell showed as a toast: the reminder is not posted ([`covers`]);
//! - the reminder, or the early card, was shown as a toast: the brief is stored for the inbox
//!   only, and takes that card's place there. The early heads-up is not a moment of its own.
//!
//! Two cards, one toast. If the brief cannot interrupt (the day's budget, quiet initiative, the
//! person's Calendar setting, quiet hours) and the event's own card is still waiting to toast, both
//! cards stay: the person's own reminder is the meeting's toast and is never traded for an inbox
//! card, and the brief is an inbox card beside it. Both end at the start (`useful_until`). The
//! store decides this on the grant it makes ([`Store::add_os_notice_at`]), not the brief on its
//! request, so that it is true whatever the grant turns out to be.
//!
//! "Already the meeting's toast" is what the shell reported as shown ([`toasted`]), never the
//! grant: a granted card the shell is holding has made no toast, and one replaced while held never
//! will.

use chrono::{DateTime, Duration, Local, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use yantrik_ipc_contracts::notifications::notice::{Attention, Family, FamilySetting, Notice, OS_HARNESS};

use crate::reminders::{minutes_phrase, notice_id as calendar_id, Kind};
use crate::store::Store;

/// What the brief's card is filed as: the calendar's own name.
const NAME: &str = "Calendar";

/// The longest event id the calendar issues, and the notice id bound this must fit.
const MAX_EVENT_ID: usize = 64;

/// What the shell says about the brief that is ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub event_id: String,
    pub title: String,
    /// The event's start, unix seconds.
    pub start: i64,
}

impl Ready {
    /// Read the shell's request. A fault here is the shell's, so it is refused, not retried.
    pub fn parse(params: &serde_json::Value) -> Result<Ready, String> {
        let event_id = params["event_id"].as_str().unwrap_or_default().to_string();
        let ok = !event_id.is_empty() && event_id.len() <= MAX_EVENT_ID && event_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok {
            return Err("`event_id` is not an id the calendar issues".into());
        }
        let title = params["title"].as_str().unwrap_or_default().trim().to_string();
        let start = params["start"].as_i64().ok_or("`start` is the event's start in unix seconds")?;
        Ok(Ready { event_id, title, start })
    }
}

/// The notice id of the brief for one event at one start. The start is the compact local stamp the
/// calendar's own notice ids use, so the two agree on which meeting they are about.
pub fn notice_id(event: &str, start: &NaiveDateTime) -> String {
    format!("cal-brief:{event}:{}", start.format("%Y%m%dT%H%M"))
}

/// The desktop's notice with this id, whether or not it was dismissed since. Looked up in place:
/// this runs for every due reminder every tick, and must not copy the store.
fn held(store: &Store, id: &str) -> Option<Notice> {
    let state = store.lock();
    state.notifications.iter().find_map(|n| n.notice.as_ref().filter(|x| x.notice_id == id && x.harness.id == OS_HARNESS)).cloned()
}

/// Whether the brief's notice already holds this event's interrupt, so the event's own reminder
/// must not make a second one. That is so when the brief was granted one, and when it took the
/// place of an early card that had already interrupted. A brief the budget, the quiet hours or
/// the person's settings kept in the inbox, with no early card before it, interrupts nobody, and
/// the reminder is the person's own.
pub fn covers(store: &Store, event: &str, start: &NaiveDateTime) -> bool {
    held(store, &notice_id(event, start))
        .is_some_and(|n| n.granted == Attention::Interrupt || toasted(store, &calendar_id(Kind::Early, event, start)))
}

/// Whether the shell reported the desktop's notice `id` as shown as a toast (`mark_shown`, from
/// `NOTICE_SHOWN`). What the person saw, not what was granted: a granted interrupt the shell is
/// holding (a call, do-not-disturb, a locked screen) is not a toast yet, and a card replaced
/// while held is never shown.
pub(crate) fn toasted(store: &Store, id: &str) -> bool {
    held(store, id).is_some_and(|n| n.shown_as.as_deref() == Some("toast"))
}

/// Announce the brief of `ready` at `now`, and answer as the store does.
pub fn announce<Tz: TimeZone>(store: &Store, ready: &Ready, now: DateTime<Tz>) -> serde_json::Value {
    let Some(starts) = Utc.timestamp_opt(ready.start, 0).single() else {
        return serde_json::json!({ "stored": false, "refused": "`start` is not a time" });
    };
    let tz = now.timezone();
    let now = now.with_timezone(&Utc);
    // A brief for a meeting that has begun is noise: nothing is posted.
    let left = starts - now;
    if left <= Duration::zero() {
        return serde_json::json!({ "stored": false, "refused": "the meeting has started" });
    }
    let start = starts.with_timezone(&tz).naive_local();
    let reminder_id = calendar_id(Kind::Reminder, &ready.event_id, &start);
    let early_id = calendar_id(Kind::Early, &ready.event_id, &start);
    let reminder = held(store, &reminder_id);
    let early = held(store, &early_id);
    let settings = store.initiative();
    // Quiet hours hold every interrupt, and the event's own reminder is the one that is held and
    // toasted when they end; two held toasts for one meeting would come out together.
    let quiet = settings.quiet_hours.is_some_and(|q| q.ends_after(now.with_timezone(&Local)).is_some());
    // The reminder or the early card was already shown as a toast: the brief only takes its place
    // in the inbox. One that was granted but is still held is replaced, and the brief's is the
    // toast.
    let interrupt = !toasted(store, &reminder_id) && !toasted(store, &early_id) && !quiet;
    // A brief the person switched off is stored hidden, and must not take a card with it. Of the
    // two it may replace, the reminder is the one still up when it has come (it took the early
    // card's place itself).
    let card = if reminder.is_some() { Some(reminder_id) } else { early.map(|_| early_id) };
    // Always named: whether the card is really replaced is the store's call, on the grant it
    // makes (a brief that will not toast leaves a reminder that is still waiting to toast).
    let replaces = card.filter(|_| settings.setting(&Family::Calendar) != FamilySetting::Off);
    let minutes = (left.num_seconds() + 59) / 60;
    let mut notice = serde_json::json!({
        "notice_id": notice_id(&ready.event_id, &start),
        "family": "calendar",
        "title": format!("{} {}", if ready.title.is_empty() { "Your meeting" } else { &ready.title }, minutes_phrase(minutes)),
        "body": "Your brief is ready.",
        "why": "The desktop prepared this from your own mail, notes and files before the meeting.",
        "attention": if interrupt { "interrupt" } else { "inbox" },
        // A heads-up shown after its moment is noise.
        "useful_until": starts.to_rfc3339_opts(SecondsFormat::Secs, true),
        "action": { "kind": "open_event", "ref": ready.event_id },
    });
    if let Some(card) = replaces {
        notice["supersedes"] = card.into();
    }
    store.add_os_notice_at(NAME, notice, now)
}
