//! The person's yes to getting promised things ready (#1184, round 3: HIGH-B, MEDIUM-C).
//!
//! The yes is a person-only act, in the class of the shell's `PERSON_ONLY` controls: it is given by
//! a real button (its own callback, never a row's expand toggle) on the brief card, on Settings →
//! Companion, which the shell wires to [`person_answer`]. It is **not** a tool and
//! no recipe step or model turn can reach it: `tools/` does not name this module
//! (`tests/commitment_prep_rules.rs` reads the sources and fails if it does). The model-callable
//! tool can only refuse (`commitment_store::refuse`), which is always safe.
//!
//! The answer is tied to a question: a yes counts only while a question is outstanding, and only
//! for the item it was put on. The question is outstanding from the moment the brief card shows
//! it ([`brief_actions`]), not from the moment the item was stored, so an item that expires unseen
//! does not use up the only asking. Who answered and which item it answered are kept.

use crate::anticipation::{AnticipationEngine, PreparedItem, PreparedKind};
use crate::commitment_store as store;

/// The action ids the buttons send to the shell.
pub const YES: &str = "commitment_prep:yes:";
pub const NO: &str = "commitment_prep:no:";
pub const TURN_ON: &str = "commitment_prep:on";
pub const TURN_OFF: &str = "commitment_prep:off";

/// Where the person turns it back on, in the words of every sentence that points at it.
pub const WHERE: &str = "Settings \u{2192} Companion";

/// The question outstanding: what the buttons answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub yes: String,
    pub no: String,
    pub text: String,
}

/// What a surface shows about preparing: whether it is on, and the question if one waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub on: bool,
    pub question: Option<Question>,
}

/// What a press says back: the sentence (if it answered anything), what Settings shows, and the cards.
pub type Reply = (Option<String>, Status, Vec<crate::commitment_cards::Card>);

/// The `no` button's id for a `yes` button's id.
pub fn no_for(action_id: &str) -> Option<String> {
    action_id.strip_prefix(YES).map(|item| format!("{NO}{item}"))
}

/// The sentence of a proposed item without the question it ends with, for a surface that puts the
/// question as its own heading.
pub fn without_question(text: &str) -> &str {
    text.trim_end().strip_suffix(store::QUESTION).unwrap_or(text).trim_end()
}

/// The proposed items that are waiting for a yes or a no.
fn proposed(conn: &rusqlite::Connection, now: f64) -> Vec<PreparedItem> {
    AnticipationEngine::get_relevant_at(conn, now)
        .into_iter()
        .filter(|i| i.kind == PreparedKind::CommitmentPrep && i.data["state"] == "proposed")
        .collect()
}

/// What a surface shows, as of `now`, without putting the question.
pub fn status(conn: &rusqlite::Connection, now: f64) -> Status {
    let on = store::enabled(conn);
    let question = (on && !store::consented(conn)).then(|| {
        let waiting = proposed(conn, now);
        let outstanding = store::asking(conn);
        let item = waiting.iter().find(|i| Some(&i.id) == outstanding.as_ref()).or_else(|| waiting.first())?;
        Some(Question { yes: format!("{YES}{}", item.id), no: format!("{NO}{}", item.id), text: item.description.clone() })
    });
    Status { on, question: question.flatten() }
}

/// [`status`] for a surface that shows the question to the person: showing it is what marks it asked
/// and outstanding for that item, so its buttons can answer it.
pub fn ask(conn: &rusqlite::Connection, now: f64) -> Status {
    let status = status(conn, now);
    if let Some(item) = status.question.as_ref().and_then(|q| q.yes.strip_prefix(YES)) {
        if store::asking(conn).as_deref() != Some(item) {
            store::mark_asked(conn, item);
        }
    }
    status
}

/// One row of the brief card, with its buttons: `action_id` is the yes (or the turn-on) button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefAction {
    pub icon: &'static str,
    pub label: String,
    pub content: String,
    pub action_id: String,
}

/// What the brief card shows about preparing. Showing the question is what marks it asked and
/// outstanding. After the person's own No (or Turn off) nothing is shown: the way back is the switch
/// in Settings \u{2192} Companion. Only after a model turn refused is it offered here, because the person
/// did not choose that.
pub fn brief_actions(conn: &rusqlite::Connection, now: f64) -> Vec<BriefAction> {
    let status = ask(conn, now);
    if !status.on {
        let by_person = store::answered(conn).is_some_and(|(by, _)| by == "person");
        if by_person {
            return Vec::new();
        }
        return vec![BriefAction {
            icon: "\u{2726}",
            label: "Get promised things ready: off".into(),
            content: "Yantrik turned this off, not you. Files would be opened in Mind View and emails drafted; nothing is ever sent.".into(),
            action_id: TURN_ON.into(),
        }];
    }
    status.question.map(|q| vec![question_action(&q)]).unwrap_or_default()
}

/// The brief card's row for a question. A neutral mark: a tick before a question reads as done.
pub fn question_action(q: &Question) -> BriefAction {
    BriefAction { icon: "\u{2726}", label: "Get promised things ready?".into(), content: without_question(&q.text).into(), action_id: q.yes.clone() }
}

/// A press of one of the buttons. What was said back to the person, or `None` when the press is not
/// this module's or does not answer anything (a stale card, a repeated press).
pub fn person_answer(conn: &rusqlite::Connection, action_id: &str) -> Option<String> {
    if action_id == TURN_ON {
        if store::enabled(conn) {
            return None;
        }
        store::record_answer(conn, true, "person", "on");
        return Some("Getting promised things ready is on.".into());
    }
    if action_id == TURN_OFF {
        if !store::enabled(conn) {
            return None;
        }
        store::record_answer(conn, false, "person", "off");
        return Some(format!("Okay. I won't get promised things ready. {WHERE} turns it back on."));
    }
    let (yes, item) = match (action_id.strip_prefix(YES), action_id.strip_prefix(NO)) {
        (Some(item), _) => (true, item),
        (_, Some(item)) => (false, item),
        _ => return None,
    };
    // Only the item whose question was put to the person, on the brief or in Settings (`ask`).
    if store::asking(conn).as_deref() != Some(item) || store::consented(conn) {
        return None;
    }
    store::record_answer(conn, yes, "person", item);
    Some(if yes {
        "Got it. I'll get promised things ready, and never send anything.".to_string()
    } else {
        format!("Okay. I won't get promised things ready. {WHERE} turns it back on.")
    })
}
