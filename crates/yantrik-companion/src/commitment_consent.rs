//! The person's yes to getting promised things ready (#1184, round 3: HIGH-B, MEDIUM-C).
//!
//! The yes is a person-only act, in the class of the shell's `PERSON_ONLY` controls: it is given by
//! a click on the brief card, which the shell wires to [`person_answer`]. It is **not** a tool and
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

/// The action ids the brief card's buttons send to the shell.
pub const YES: &str = "commitment_prep:yes:";
pub const NO: &str = "commitment_prep:no:";
pub const TURN_ON: &str = "commitment_prep:on";

/// One button-bearing row of the brief card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefAction {
    pub icon: &'static str,
    pub label: String,
    pub content: String,
    pub action_id: String,
}

/// The proposed item that is waiting for a yes or a no, if there is one.
fn proposed(conn: &rusqlite::Connection, now: f64) -> Vec<PreparedItem> {
    AnticipationEngine::get_relevant_at(conn, now)
        .into_iter()
        .filter(|i| i.kind == PreparedKind::CommitmentPrep && i.data["state"] == "proposed")
        .collect()
}

/// What the brief card shows about preparing, as of `now`. Showing the question is what marks it
/// asked and outstanding.
pub fn brief_actions(conn: &rusqlite::Connection, now: f64) -> Vec<BriefAction> {
    if !store::enabled(conn) {
        // The person turned it off; the way back is theirs to press.
        return vec![BriefAction {
            icon: "\u{2726}",
            label: "Get promised things ready: off".into(),
            content: "Press to turn it back on. Files would be opened in Mind View and emails drafted; nothing is ever sent.".into(),
            action_id: TURN_ON.into(),
        }];
    }
    if store::consented(conn) {
        return Vec::new();
    }
    let waiting = proposed(conn, now);
    let outstanding = store::asking(conn);
    let Some(item) = waiting.iter().find(|i| Some(&i.id) == outstanding.as_ref()).or_else(|| waiting.first()) else {
        return Vec::new();
    };
    store::mark_asked(conn, &item.id);
    vec![
        BriefAction { icon: "\u{2713}", label: "Yes, get these ready".into(), content: item.description.clone(), action_id: format!("{YES}{}", item.id) },
        BriefAction { icon: "\u{2715}", label: "No, don't".into(), content: "Nothing will be opened or drafted for promises.".into(), action_id: format!("{NO}{}", item.id) },
    ]
}

/// A click on a brief-card button. What was said back to the person, or `None` when the click is
/// not this module's or does not answer an outstanding question (a stale card, a repeated click).
pub fn person_answer(conn: &rusqlite::Connection, action_id: &str) -> Option<String> {
    if action_id == TURN_ON {
        if store::enabled(conn) {
            return None;
        }
        store::record_answer(conn, true, "person", "on");
        return Some("Getting promised things ready is on.".into());
    }
    let (yes, item) = match (action_id.strip_prefix(YES), action_id.strip_prefix(NO)) {
        (Some(item), _) => (true, item),
        (_, Some(item)) => (false, item),
        _ => return None,
    };
    if store::asking(conn).as_deref() != Some(item) || store::consented(conn) {
        return None;
    }
    store::record_answer(conn, yes, "person", item);
    Some(if yes { "Got it. I'll get promised things ready, and never send anything." } else { "Okay. I won't get promised things ready." }.into())
}
