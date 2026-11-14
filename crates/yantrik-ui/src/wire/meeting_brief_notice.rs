//! The brief's one interrupt, asked of the notifications service.
//!
//! The shell makes the card; whether the person is interrupted is the service's to grant, through
//! the same path as the calendar's own reminders (`design/calendar-heads-up-2026-10-16.md`: one
//! grant path). So the day's budget, quiet hours, quiet initiative and the Calendar setting all
//! apply, and a toast held past the start is never shown. This file only says "a brief is ready",
//! once per brief, from a thread of its own: the service may be down or slow, and neither the
//! companion's worker nor the UI thread waits on it.

use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use std::time::Duration;

use yantrik_companion::meeting_brief::Card;
use yantrik_ipc_contracts::notifications::NOTICE_BRIEF;

/// How many times a refused-for-now post is tried, and how long between.
const ATTEMPTS: u32 = 4;
const RETRY: Duration = Duration::from_secs(5);
/// How long one post may take.
const POST_TIMEOUT: Duration = Duration::from_secs(5);

/// What the service is told: enough to name the meeting, nothing a brief holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub event_id: String,
    pub title: String,
    pub start: i64,
}

impl Ready {
    pub fn of(card: &Card) -> Ready {
        Ready { event_id: card.brief.event_id.clone(), title: card.brief.title.clone(), start: card.brief.start }
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({ "event_id": self.event_id, "title": self.title, "start": self.start })
    }
}

/// How the service answered, as far as the asker is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Done,
    /// Try again: the store could not take it just now, or the service was not reached.
    Later,
    /// Do not try again.
    Refused,
}

/// Read a post's answer: stored (or already held) is done; a refusal is final; anything else is
/// worth another try.
pub fn answer_of(reply: &Result<serde_json::Value, String>) -> Answer {
    match reply {
        Ok(v) if v["stored"] == true => Answer::Done,
        Ok(v) if v["refused"].is_string() => Answer::Refused,
        _ => Answer::Later,
    }
}

fn post(ready: &Ready) -> Answer {
    answer_of(&crate::notice_desk::call(NOTICE_BRIEF, ready.params(), POST_TIMEOUT))
}

/// Tell the service this brief is ready. Returns at once; one thread makes every post.
pub fn announce(card: &Card) {
    static SENDER: OnceLock<Option<Sender<Ready>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = channel::<Ready>();
        let spawned = std::thread::Builder::new().name("yos-meeting-brief-notice".into()).spawn(move || {
            for ready in rx {
                for attempt in 0..ATTEMPTS {
                    match post(&ready) {
                        Answer::Done => break,
                        Answer::Refused => {
                            tracing::warn!(event = %ready.event_id, "the notifications service refused a meeting brief's notice");
                            break;
                        }
                        Answer::Later if attempt + 1 < ATTEMPTS => std::thread::sleep(RETRY),
                        Answer::Later => tracing::warn!(event = %ready.event_id, "a meeting brief's notice could not be posted; the card is still shown"),
                    }
                }
            }
        });
        spawned.ok().map(|_| tx)
    });
    if let Some(tx) = sender {
        let _ = tx.send(Ready::of(card));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_service_is_told_names_the_meeting_and_nothing_a_brief_holds() {
        let r = Ready { event_id: "e1".into(), title: "Orion launch".into(), start: 1_000 };
        assert_eq!(r.params(), serde_json::json!({ "event_id": "e1", "title": "Orion launch", "start": 1_000 }));
    }

    #[test]
    fn a_stored_post_is_done_a_refusal_is_final_and_anything_else_is_tried_again() {
        assert_eq!(answer_of(&Ok(serde_json::json!({ "stored": true }))), Answer::Done);
        assert_eq!(answer_of(&Ok(serde_json::json!({ "stored": true, "duplicate": true }))), Answer::Done);
        assert_eq!(answer_of(&Ok(serde_json::json!({ "stored": false, "refused": "the meeting has started" }))), Answer::Refused);
        assert_eq!(answer_of(&Ok(serde_json::json!({ "stored": false, "retry_after_secs": 5 }))), Answer::Later);
        assert_eq!(answer_of(&Err("the service is down".into())), Answer::Later);
    }
}
