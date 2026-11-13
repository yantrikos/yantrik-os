//! The cards the shell raises for preparing promised things (#1184, round 5).
//!
//! The brief card is claimed at boot and gone in five minutes, so a question put later in the day,
//! and the work done for a promise, need a place where the person is. These cards are that place:
//! one per item, drawn from what is stored, so there is never more than one for an item however
//! often they are asked for, and one goes when its item is answered or expires.
//!
//! - A proposed item: the question, with Yes and No.
//! - A prepared item: what was done, in plain words, with **Got it** (puts this card away, consent
//!   unchanged) and **Stop getting things ready** (turns preparing off).
//!
//! The buttons are the person's alone, as the brief card's are (`commitment_consent`): the shell
//! draws them pointer-only, and they reach `commitment_consent::person_answer` through the one
//! button callback and nothing else.

use crate::anticipation::{AnticipationEngine, PreparedItem, PreparedKind};
use crate::commitment_consent::{self as consent, SEEN, TURN_OFF, YES};
use crate::commitment_store as store;

/// Most prepared items shown at once, so a busy week does not cover the screen.
const MOST_PREPARED: usize = 3;

/// One card: the item it is for, what it says, and its buttons. `no_id` is empty for a single button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub item: String,
    pub title: String,
    pub text: String,
    pub yes_id: String,
    pub yes_label: &'static str,
    pub no_id: String,
    pub no_label: &'static str,
}

/// The cards to show, as of `now`: the question first, then what was prepared. Showing the question
/// marks it asked, as the brief card does, so its buttons can answer it. Nothing is shown when
/// preparing is off, and nothing is asked then.
pub fn cards(conn: &rusqlite::Connection, now: f64) -> Vec<Card> {
    let status = consent::ask(conn, now);
    if !status.on {
        return Vec::new();
    }
    let question = status.question.map(|q| Card {
        item: q.yes.strip_prefix(YES).unwrap_or_default().to_string(),
        title: store::QUESTION.into(),
        text: consent::without_question(&q.text).into(),
        yes_id: q.yes,
        yes_label: "Yes",
        no_id: q.no,
        no_label: "No",
    });
    let done = prepared(conn, now).into_iter().map(|i| Card {
        yes_id: format!("{SEEN}{}", i.id),
        item: i.id,
        title: "Got something ready".into(),
        text: i.description,
        yes_label: "Got it",
        no_id: TURN_OFF.into(),
        no_label: "Stop getting things ready",
    });
    question.into_iter().chain(done).collect()
}

/// The prepared items whose cards are on screen as of `now`: newest first, at most [`MOST_PREPARED`],
/// and none while preparing is off or not yet agreed to.
pub fn prepared(conn: &rusqlite::Connection, now: f64) -> Vec<PreparedItem> {
    if !store::enabled(conn) || !store::consented(conn) {
        return Vec::new();
    }
    let mut done: Vec<PreparedItem> = AnticipationEngine::get_relevant_at(conn, now).into_iter().filter(did_something).collect();
    done.sort_by(|a, b| b.created_at.total_cmp(&a.created_at));
    done.truncate(MOST_PREPARED);
    done
}

/// A prepared item that opened something or drafted something: not one still trying, and not one
/// that did nothing.
fn did_something(i: &PreparedItem) -> bool {
    i.kind == PreparedKind::CommitmentPrep
        && i.data["state"] == "prepared"
        && (i.data["draft"].is_object() || i.data["opened"].as_array().is_some_and(|o| !o.is_empty()))
}
