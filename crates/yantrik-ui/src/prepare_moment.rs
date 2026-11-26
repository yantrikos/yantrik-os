//! "Prepare for what's next" on the desktop: hands the agenda's candidate to the notifications
//! service, and carries what the person did with the card back to the agenda's ledger (#1416).
//!
//! The agenda (`yantrik_companion::agenda::prepare`) decides whether there is a moment and what it
//! says: a plain-code rule over titles, counts and times. The notifications service makes the
//! card and grants its attention, through the same path as every other card (the day's two
//! interrupts, quiet hours, quiet initiative, the family's On / Inbox only / Off), so over budget
//! it waits in the inbox. This file is only the courier, and runs from the Think arm's worker:
//!
//! - one post per tick at most, on a thread of its own: the service may be down or slow, and the
//!   Think arm never waits on it;
//! - a moment is written in the ledger once the service stored it, so a refused or lost post is
//!   tried again, and the service's own card id (`prep:<event>`) makes a repeat the same card;
//! - the card's buttons (Open the note, Ask, Not now) report here from the UI thread, and the next
//!   tick writes them down as opened or dismissed.
//!
//! Private mode and the Brain and Prepare switches are the agenda's own (`candidates` says nothing
//! when any is off).

use std::sync::Mutex;
use std::time::Duration;

use rusqlite::Connection;
use yantrik_companion::agenda::prepare::{self, Candidate, Outcome, RULE_EVENT_NOTE};
use yantrik_companion::agenda::Switches;
use yantrik_ipc_contracts::notifications::notice::MOMENT_PREPARE;
use yantrik_ipc_contracts::notifications::NOTICE_PREPARE;

use crate::wire::meeting_brief_notice::{answer_of, Answer};

const POST_TIMEOUT: Duration = Duration::from_secs(5);
const ATTEMPTS: u32 = 3;
const RETRY: Duration = Duration::from_secs(5);

/// Posts being made, and posts the service stored that the ledger has not written yet.
static IN_FLIGHT: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STORED: Mutex<Vec<(Candidate, f64)>> = Mutex::new(Vec::new());
/// What the person pressed on a card, until the next tick writes it down: `(event id, outcome)`.
static PRESSED: Mutex<Vec<(String, Outcome)>> = Mutex::new(Vec::new());

fn held<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The event a `prep:` card is for, from its notice id.
fn event_of(notice_id: &str) -> Option<&str> {
    notice_id.strip_prefix("prep:").filter(|e| !e.is_empty())
}

/// The person pressed one of a card's buttons (from the UI thread). Anything but a `prep:` card
/// is not ours and is ignored.
pub fn pressed(notice_id: &str, outcome: Outcome) {
    if let Some(event) = event_of(notice_id) {
        held(&PRESSED).push((event.to_string(), outcome));
    }
}

fn params(c: &Candidate) -> serde_json::Value {
    serde_json::json!({
        "event_id": c.event_id,
        "title": c.event_title,
        "start": c.start as i64,
        "note_title": c.note_title,
        "open_items": c.open_items,
        "why": c.reason,
        "ask": c.ask_text(),
    })
}

fn post_with_retries(c: &Candidate) -> Answer {
    for attempt in 0..ATTEMPTS {
        match answer_of(&crate::notice_desk::call(NOTICE_PREPARE, params(c), POST_TIMEOUT)) {
            Answer::Later if attempt + 1 < ATTEMPTS => std::thread::sleep(RETRY),
            other => return other,
        }
    }
    Answer::Later
}

/// One pass from the Think arm, after the agenda's own tick. `now` is unix seconds.
pub fn run(conn: &Connection, now: f64, switches: Switches) {
    let on = crate::notice_desk::initiative_known().map_or(true, |s| s.moment_on(MOMENT_PREPARE));
    let mut done: Vec<(Candidate, f64)> = std::mem::take(&mut *held(&STORED));
    for (c, at) in done.drain(..) {
        held(&IN_FLIGHT).retain(|e| *e != c.event_id);
        if let Err(e) = prepare::offered(conn, &c, at) {
            tracing::warn!(error = %e, "the agenda could not write down an offered moment");
        }
    }
    for (event, outcome) in std::mem::take(&mut *held(&PRESSED)) {
        let _ = prepare::record(conn, RULE_EVENT_NOTE, &event, outcome, now);
    }
    let _ = prepare::settle_ignored(conn, now);

    let Ok(found) = prepare::candidates(conn, now, switches, on) else { return };
    // At most one a minute, and never one that is already being posted: one quiet moment at a time.
    let Some(c) = found.into_iter().find(|c| !held(&IN_FLIGHT).contains(&c.event_id)) else { return };
    held(&IN_FLIGHT).push(c.event_id.clone());
    let spawned = std::thread::Builder::new().name("yos-prepare".into()).spawn(move || {
        match post_with_retries(&c) {
            Answer::Done => held(&STORED).push((c, now)),
            Answer::Refused => {
                tracing::warn!(event = %c.event_id, "the notifications service refused a prepare moment");
                held(&IN_FLIGHT).retain(|e| *e != c.event_id);
            }
            Answer::Later => held(&IN_FLIGHT).retain(|e| *e != c.event_id),
        }
    });
    if spawned.is_err() {
        tracing::debug!("could not start the prepare moment's courier");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_prepare_card_reports_a_press() {
        pressed("cal-brief:e1:20261127T1500", Outcome::Opened);
        pressed("prep:", Outcome::Opened);
        assert!(held(&PRESSED).is_empty());
        pressed("prep:e1", Outcome::Dismissed);
        assert_eq!(held(&PRESSED).drain(..).collect::<Vec<_>>(), [("e1".to_string(), Outcome::Dismissed)]);
    }

    #[test]
    fn the_post_names_the_event_the_note_and_the_words_for_the_lens() {
        let c = Candidate {
            rule: RULE_EVENT_NOTE,
            event_id: "e1".into(),
            event_title: "Record the 60 s demo video".into(),
            start: 1_800_000_000.0,
            note_title: "Launch prep".into(),
            open_items: 4,
            words: "demo video".into(),
            reason: "why".into(),
        };
        let p = params(&c);
        assert_eq!((p["event_id"].as_str(), p["open_items"].as_u64(), p["start"].as_i64()), (Some("e1"), Some(4), Some(1_800_000_000)));
        assert!(p["ask"].as_str().unwrap().contains("Launch prep"));
    }
}
