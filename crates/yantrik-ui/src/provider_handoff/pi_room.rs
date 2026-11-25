//! What Pi is told of each model's room (#1402): `contextWindow` and `maxTokens` in its
//! models.json, and, for a model with little room, the compaction numbers in its settings.json
//! that make Pi's own rule (compact when the context passes `contextWindow - reserveTokens`)
//! fire in time. The room is `yantrik_gateway::Room::per_request`, the rule the gateway fits a
//! call by.

use std::path::Path;

use serde_json::{json, Value};
use yantrik_gateway::Room;

/// What Pi assumes of a model whose room nobody states.
const UNKNOWN_WINDOW: u64 = 128_000;
/// The most an answer is asked to be, in a room that holds it.
const ANSWER_CEILING: u64 = 32_768;
/// A room at or above this is large enough for Pi's own compaction defaults (a 16,384 reserve and
/// 20,000 kept).
const ROOMY: u64 = 64_000;

/// What Pi 0.99.1 keeps free of `contextWindow` when it sizes a call's `max_tokens`
/// (`clampMaxTokensToContext`: window - its estimate of the prompt - 4,096, at least 1). The window
/// Pi is told is never less than the room plus this, or Pi's own margin eats the answer: with the
/// window at 8,000 and a first call of 3,600 tokens it asked for ONE token (#1402, round 6).
const PI_SAFETY: u64 = 4_096;

/// What Pi is told of one model.
pub(super) struct Told {
    /// `contextWindow`.
    pub window: u64,
    /// `maxTokens`.
    pub answer: u64,
    /// `(reserveTokens, keepRecentTokens)` of a room too small for Pi's defaults (16,384 reserved,
    /// 20,000 kept), `None` for a room they suit.
    pub compaction: Option<(u64, u64)>,
}

/// The room is the account's per-request room, else the model's own context, else 128K; the window
/// is the model's real context, but at least the room plus [`PI_SAFETY`].
///
/// Pi compacts when `contextTokens > window - reserveTokens`, and the context includes its prompt
/// and every tool's schema (about 3,600 tokens in production). In a small room the compaction point
/// is a quarter below the room, so the next call may add a message or a tool's result before it
/// would not fit; the reserve is whatever puts it there. An eighth of the room is kept: the last two
/// or three exchanges. Measured on Pi 0.99.1 at 8,000 with the production extension and a stub that
/// honours `max_tokens`: every call asks for 2,000, compaction comes every fifth turn, no call is
/// over 6,200 tokens (#1402).
pub(super) fn told(room: Room, context: u32) -> Told {
    let real = room.context.or((context > 0).then_some(u64::from(context))).unwrap_or(UNKNOWN_WINDOW);
    let request = room.per_request().unwrap_or(real);
    if request >= ROOMY {
        return Told { window: real, answer: ANSWER_CEILING, compaction: None };
    }
    let window = real.max(request + PI_SAFETY);
    Told { window, answer: Room { per_minute: Some(request), ..Room::default() }.answer_room(ANSWER_CEILING), compaction: Some((window - request * 3 / 4, request / 8)) }
}

/// settings.json with `compaction.modelOverrides["yantrik/<id>"]` for each model in `models` that
/// needs one. The overrides of ours that are no longer needed are dropped, and nothing else of
/// the person's file is touched.
pub(super) fn settings_text(current: Option<&str>, provider: &str, models: &[(String, Option<(u64, u64)>)]) -> Result<Option<String>, String> {
    let mut root: Value = match current {
        Some(t) if !t.trim().is_empty() => serde_json::from_str(t).map_err(|e| format!("Pi's settings.json is not JSON ({e}), so it is left alone"))?,
        _ => json!({}),
    };
    let Some(root_map) = root.as_object_mut() else { return Err("Pi's settings.json is not an object, so it is left alone".into()) };
    let section = root_map.entry("compaction").or_insert_with(|| json!({}));
    let Some(section) = section.as_object_mut() else { return Err("Pi's settings.json `compaction` is not an object, so it is left alone".into()) };
    let overrides = section.entry("modelOverrides").or_insert_with(|| json!({}));
    let Some(overrides) = overrides.as_object_mut() else { return Err("Pi's settings.json `compaction.modelOverrides` is not an object, so it is left alone".into()) };
    let ours = format!("{provider}/");
    overrides.retain(|key, _| !key.starts_with(&ours));
    for (id, compaction) in models {
        if let Some((reserve, keep)) = compaction {
            overrides.insert(format!("{ours}{id}"), json!({ "reserveTokens": reserve, "keepRecentTokens": keep }));
        }
    }
    if overrides.is_empty() {
        section.remove("modelOverrides");
    }
    if section.is_empty() {
        root_map.remove("compaction");
    }
    // A file the person never had is not made to hold nothing.
    if current.is_none() && root.as_object().is_some_and(|o| o.is_empty()) {
        return Ok(None);
    }
    serde_json::to_string_pretty(&root).map(|t| Some(t + "\n")).map_err(|e| e.to_string())
}

/// Pi's settings.json.
pub(super) fn settings_json(home: &Path) -> std::path::PathBuf {
    home.join(".pi/agent/settings.json")
}

/// Whether the person has edited `path` since this handoff wrote it: its live bytes are not what
/// the marker says were written. `models.json` is ours whole. `settings.json` is the person's, and
/// only the `compaction.modelOverrides["yantrik/…"]` entries are ours: it is edited by hand when one
/// of them is not an object of the two numbers this writes (its other keys are merged, never lost).
fn edited_by_hand(home: &Path, path: &Path) -> bool {
    use super::Handoff as _;
    let Some(t) = super::marker(home, super::pi::Pi.harness()).and_then(|m| m.files.into_iter().find(|t| t.path == path)) else { return false };
    let (Some(written), Ok(live)) = (t.written, std::fs::read(path)) else { return false };
    if super::progress::sha(&live) == written {
        return false;
    }
    if path != settings_json(home) {
        return true;
    }
    let ours = |v: &Value| v.as_object().is_some_and(|o| !o.is_empty() && o.iter().all(|(k, n)| matches!(k.as_str(), "reserveTokens" | "keepRecentTokens") && n.is_u64()));
    serde_json::from_slice::<Value>(&live).ok().is_none_or(|v| {
        v.pointer("/compaction/modelOverrides").and_then(Value::as_object).is_some_and(|o| o.iter().any(|(k, n)| k.starts_with("yantrik/") && !ours(n)))
    })
}

/// What a refresh of Pi's room files decided.
pub(crate) enum Planned {
    /// These files, rewritten.
    Plan(super::Plan),
    /// What this build writes is what is there.
    Unchanged,
    /// Left alone, and why, in words for a log.
    Skipped(String),
}

/// Pi's models.json and settings.json again, with the rooms of the model now picked (the ask bar's
/// pick changed) or as this build writes them (the shell started, or the accounts became readable:
/// a machine updated from a build that wrote other numbers gets the new ones, #1402), under the token
/// Pi already holds. The plan has no restart: a running Pi is made to read the files again by its
/// harness, between turns (`/yantrik-room`).
pub(crate) fn plan_refresh(home: &Path, offer_with: impl FnOnce(super::Token) -> Result<super::Offer, String>) -> Result<Planned, String> {
    use super::Handoff as _;
    if !super::uses_gateway(home, super::pi::Pi.harness()) {
        return Ok(Planned::Skipped("Pi is not on Yantrik models".into()));
    }
    let models = super::read_json_object(&home.join(".pi/agent/models.json"))?;
    let Some(held) = models.get("providers").and_then(|p| p.get(super::pi::PROVIDER)).and_then(|p| p.get("apiKey")).and_then(Value::as_str) else {
        return Ok(Planned::Skipped("Pi's models.json holds no Yantrik token".into()));
    };
    let mut plan = super::pi::Pi.plan_gateway(home, &offer_with(super::Token(held.to_string()))?)?;
    // Only what the room changes: Pi's models and settings. Its pi.json (provider and model) does not
    // change with the room, and the installer adds keys of its own to it (`memory`) after the handoff.
    plan.writes.retain(|w| w.path != home.join(".config/yantrik/pi.json"));
    if let Some(w) = plan.writes.iter().find(|w| edited_by_hand(home, &w.path)) {
        return Ok(Planned::Skipped(format!("{} was edited by hand, so it is left as it is", w.path.display())));
    }
    if plan.writes.iter().all(|w| std::fs::read_to_string(&w.path).is_ok_and(|live| live == w.content)) {
        return Ok(Planned::Unchanged);
    }
    plan.restart = None;
    plan.partial = true;
    Ok(Planned::Plan(plan))
}

/// [`plan_refresh`], for the pick: the plan, or nothing to do.
#[cfg(test)]
pub(crate) fn plan_for_new_pick(home: &Path, offer_with: impl FnOnce(super::Token) -> Result<super::Offer, String>) -> Result<Option<super::Plan>, String> {
    Ok(match plan_refresh(home, offer_with)? {
        Planned::Plan(plan) => Some(plan),
        _ => None,
    })
}

/// What a refresh did, for one line in the log.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Written(Vec<String>),
    Unchanged,
    Skipped(String),
}

/// Refresh Pi's room files, or say why not. `readable` is whether the accounts (and so each model's
/// room) can be read yet: the shell starts locked, and until the person unlocks the accounts are an
/// empty list, so every model would be told the room it has with no account (its own context) and
/// the plan would equal what is there. Nothing is decided then, and that is said, not read as "no
/// change".
pub(crate) fn refresh(home: &Path, readable: bool, offer_with: impl FnOnce(super::Token) -> Result<super::Offer, String>) -> Result<Outcome, String> {
    if !readable {
        return Ok(Outcome::Skipped("the accounts are not readable yet, so no model's room is known".into()));
    }
    Ok(match plan_refresh(home, offer_with)? {
        Planned::Skipped(why) => Outcome::Skipped(why),
        Planned::Unchanged => Outcome::Unchanged,
        Planned::Plan(plan) => {
            super::apply(home, &plan)?;
            Outcome::Written(plan.writes.iter().filter_map(|w| w.path.file_name().map(|n| n.to_string_lossy().into_owned())).collect())
        }
    })
}
