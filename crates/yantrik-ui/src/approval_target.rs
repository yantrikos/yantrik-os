//! What a card's action would act on, named by the app that owns it — never by the caller.
//!
//! # Why this is its own module
//!
//! An outside design review (approval-safety condition, October 2026): the card for
//! `calendar.delete_event` read "Deletes: id: 01a0c718-…". The person could not tell which event
//! they were deleting, and a card whose question cannot be answered is a card that gets waved
//! through. The app knows what its ids stand for, so the shell asks it (`app.name_target`, read
//! like `app.explain`) and draws the rows it answers: title, date and time, calendar, whether a
//! recurring series goes with it; a file's path and size.
//!
//! # The rule
//!
//! A DESTRUCTIVE card (`approvals::shown_whole`) whose action acts on a named thing and whose
//! target the app could not name — not running, an id it does not hold, no namer for the action —
//! can be declined and nothing else: the store refuses to grant it ([`blocked`]), and the card
//! says why. A grant is never made for a deletion whose target the person could not see.
//!
//! Nothing here reads the request's arguments as display text or its purpose at all: the rows
//! are the app's answer, and the arguments only travel TO the app, as the question.
//!
//! # What the grant holds the handler to
//!
//! The app answers an opaque identity beside the rows — an event's id, a path's device and inode,
//! a container's full id, a pid and its start time — and an answer without one names nothing.
//! The record keeps it, `consume` hands it back with the grant, and the app's handler resolves the
//! arguments again when it runs and refuses when they now point at something else
//! (`yantrik_surface::held_to_grant`; security review of #652, H1). The card can name one thing
//! only if that thing is the one acted on.

use yantrik_app_runtime::control::Target;

/// What the card knows about the target of one call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Named {
    /// The action does not act on a named thing, or nobody needed to ask: the card is what it was.
    #[default]
    NotAsked,
    /// The app named it.
    Resolved(Resolved),
    /// The action acts on a named thing and the app could not say what.
    Unresolved,
}

/// The app's name for a call's target, as the record keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// `(label, value)`, the first naming the thing itself. Bounded on arrival ([`from_reply`]).
    pub rows: Vec<(String, String)>,
    /// The call takes a whole recurring series with it: "Delete series".
    pub series: bool,
    /// What the rows name, as the app tells it apart (`Target::identity`): never drawn, handed
    /// back with the grant for the handler to hold its own resolution to.
    pub identity: String,
    /// The arguments the rows stand for, as the app DECLARED them in `describe`
    /// (`target_handles`) — never as one answer says.
    pub handles: Vec<String>,
}

/// Verbs whose action destroys or takes away one named thing. A destructive card for one of
/// these needs its target named even when the app publishes no namer — an app cannot avoid the
/// rule by saying nothing. `update`, `move` and the like are left out: what they act on survives.
const OBJECT_VERBS: &[&str] = &[
    "delete", "remove", "erase", "purge", "trash", "wipe", "uninstall", "unlink", "discard", "drop",
    "kill", "terminate", "forget", "overwrite", "destroy",
];

/// At most this many rows from an app's answer reach the card: the foot is pinned, and its height
/// has to stay a known number of lines.
pub const ROWS: usize = 6;

/// A label longer than this is not a label.
const LABEL_CHARS: usize = 24;

/// How much of one value the record keeps. The card cuts again after escaping (`VALUE_CHARS`).
const KEPT_CHARS: usize = 240;

/// How much of one escaped value one row of the card carries.
pub const VALUE_CHARS: usize = 120;

/// An identity longer than this is not an identity: a full container id is 64 characters, a path
/// with its device and inode a few hundred.
const IDENTITY_CHARS: usize = 4096;

/// The line a destructive card shows when its target could not be named.
pub fn unavailable(verb: &str) -> String {
    format!("Target details unavailable \u{00b7} the app could not say what this would {verb}")
}

/// The object verb an action's id leads with, if any — `delete_event` → "delete" — reading past
/// one leading namespace, since the shell's own ids carry their screen first (`files_delete`).
fn object_verb(action: &str) -> Option<String> {
    action
        .split(['_', '-', '.'])
        .filter(|t| !t.is_empty())
        .take(2)
        .map(str::to_ascii_lowercase)
        .find(|t| OBJECT_VERBS.contains(&t.as_str()))
}

/// Whether the action's id says it takes a named thing away.
pub fn acts_on_object(action: &str) -> bool {
    object_verb(action).is_some()
}

/// The bare verb for [`unavailable`]: `delete_event` → "delete", anything else "act on".
pub fn verb_of(action: &str) -> String {
    object_verb(action).unwrap_or_else(|| "act on".to_string())
}

/// Whether the card has to name its target: the app declared a namer for the action, or the
/// action takes a named thing away.
pub fn needed(action: &str, declared: bool) -> bool {
    declared || acts_on_object(action)
}

/// The store's rule: a destructive card whose target was asked for and not named cannot be
/// allowed. `destructive` is `approvals::shown_whole` on the record.
pub fn blocked(named: &Named, destructive: bool) -> bool {
    destructive && *named == Named::Unresolved
}

/// The app's answer to `app.name_target`, as the card may use it. `target: null`, rows that are
/// not strings, no rows at all, or no identity are all the app not naming it. Bounded on arrival:
/// at most [`ROWS`] rows, labels of a few words, values cut at [`KEPT_CHARS`]; escaping is the
/// card's (`approval_wording::visible`), so the record keeps what the app said. `handles` are the
/// ones the action declared in `describe`.
pub fn from_reply(reply: &serde_json::Value, handles: &[String]) -> Named {
    let target = &reply["target"];
    let Some(list) = target["rows"].as_array() else { return Named::Unresolved };
    let rows: Vec<(String, String)> = list
        .iter()
        .filter_map(|row| {
            let label = row["label"].as_str()?.trim();
            let value = row["value"].as_str()?.trim();
            (!label.is_empty() && !value.is_empty() && label.chars().count() <= LABEL_CHARS)
                .then(|| (label.to_string(), clip(value, KEPT_CHARS)))
        })
        .take(ROWS)
        .collect();
    let identity = target["identity"].as_str().unwrap_or_default();
    if rows.is_empty() || identity.is_empty() || identity.chars().count() > IDENTITY_CHARS {
        return Named::Unresolved;
    }
    Named::Resolved(Resolved {
        rows,
        series: target["series"].as_bool().unwrap_or(false),
        identity: identity.to_string(),
        handles: handles.to_vec(),
    })
}

/// The same, for an action the shell publishes itself (`yantrik_app_runtime::control::published_target`).
pub fn from_local(answer: Result<Option<Target>, String>, action: &str, handles: &[String]) -> Named {
    match answer {
        Ok(Some(target)) => from_reply(&serde_json::json!({ "target": target.to_json() }), handles),
        Ok(None) => Named::Unresolved,
        Err(_) if acts_on_object(action) => Named::Unresolved,
        Err(_) => Named::NotAsked,
    }
}

/// The identity a grant for this card hands back to the handler: the named target's, or `None`.
pub fn identity(named: &Named) -> Option<String> {
    match named {
        Named::Resolved(t) => Some(t.identity.clone()),
        _ => None,
    }
}

/// Whether the first row, the one the card's "Deletes:" line says, is longer escaped than the card
/// draws ([`VALUE_CHARS`]): then the face keeps every argument, the handle included, so the whole
/// bound value is still in front of the person ("Exactly:").
pub fn first_row_cut(named: &Named) -> bool {
    matches!(named, Named::Resolved(t)
        if t.rows.first().is_some_and(|(_, v)| crate::approval_wording::visible(v).chars().count() > VALUE_CHARS))
}

/// The card's argument rows for its face: every argument except the declared handles the app's
/// rows stand for, which stay whole in the argument box under Details. Matched by exact key, on
/// the value the grant binds — never by the text of a row — and only for a scalar identifier (a
/// string or a number): a flag, a list or an object says how, not which, and is never hidden. All
/// of them when the name line is cut ([`first_row_cut`]).
pub fn face_args(named: &Named, args: &serde_json::Value, rows: impl Fn(&serde_json::Value) -> Vec<String>) -> Vec<String> {
    let (Named::Resolved(target), Some(map)) = (named, args.as_object()) else {
        return rows(args);
    };
    if first_row_cut(named) {
        return rows(args);
    }
    let hidden = |k: &String, v: &serde_json::Value| target.handles.contains(k) && (v.is_string() || v.is_number());
    let rest: serde_json::Map<String, serde_json::Value> =
        map.iter().filter(|(k, v)| !hidden(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect();
    if rest.is_empty() {
        Vec::new()
    } else {
        rows(&serde_json::Value::Object(rest))
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\u{2026}")
}

#[cfg(test)]
#[path = "approval_target_tests.rs"]
mod approval_target_tests;
