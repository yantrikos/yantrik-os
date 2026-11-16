//! The cards the shell raises for preparing promised things (#1184, round 5).
//!
//! The brief card is claimed at boot and gone in five minutes, so a question put later in the day,
//! and the work done for a promise, need a place where the person is. These cards are that place:
//! one per item, drawn from what is stored, so there is never more than one for an item however
//! often they are asked for, and one goes when its item is answered or expires.
//!
//! - A proposed item: the question, with Yes and No.
//! - A prepared item: what was found and drafted, in plain words, with the person's own ways in
//!   (one open button per located file, up to two, and one for the draft), **Got it** (puts this card away, consent
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

/// A file the preparation found, for the person to open from the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardFile {
    pub label: String,
    pub path: String,
}

/// Most open buttons for files on one card; the rest are said as "and N more".
pub const MOST_FILES_SHOWN: usize = 2;

/// One card: the item it is for, what it says, and its buttons. `no_id` is empty for a single button.
/// `files` are all the files found (the shell shows the first [`MOST_FILES_SHOWN`]), and
/// `draft_for` is the commitment whose draft waits in Email, when one does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub item: String,
    pub title: String,
    pub text: String,
    pub yes_id: String,
    pub yes_label: &'static str,
    pub no_id: String,
    pub no_label: &'static str,
    pub files: Vec<CardFile>,
    pub draft_for: Option<String>,
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
        files: Vec::new(),
        draft_for: None,
    });
    let done = prepared(conn, now).into_iter().map(|i| Card {
        yes_id: format!("{SEEN}{}", i.id),
        files: located(&i),
        draft_for: i.data["draft"].is_object().then(|| i.data["commitment_id"].as_i64().map(|c| c.to_string())).flatten(),
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

/// The files a preparation found, in the order the promise named them.
fn located(i: &PreparedItem) -> Vec<CardFile> {
    let found = i.data["files"].as_array().map(Vec::as_slice).unwrap_or_default();
    found
        .iter()
        .filter_map(|f| Some(CardFile { label: f["label"].as_str()?.to_string(), path: f["path"].as_str()?.to_string() }))
        .collect()
}

/// A prepared item that found a file or drafted something: not one still trying, and not one
/// that did nothing.
fn did_something(i: &PreparedItem) -> bool {
    i.kind == PreparedKind::CommitmentPrep
        && i.data["state"] == "prepared"
        && (i.data["draft"].is_object() || i.data["files"].as_array().is_some_and(|f| !f.is_empty()))
}
