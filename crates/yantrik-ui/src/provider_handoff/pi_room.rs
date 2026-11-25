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

/// The room Pi is told: the account's per-request room, else the model's own context, else 128K.
pub(super) fn told(room: Room, context: u32) -> Room {
    match room.per_request() {
        Some(_) => room,
        None => Room { context: Some(if context > 0 { u64::from(context) } else { UNKNOWN_WINDOW }), ..room },
    }
}

/// `(contextWindow, maxTokens)` for models.json.
pub(super) fn window_and_answer(told: Room) -> (u64, u64) {
    let window = told.per_request().unwrap_or(UNKNOWN_WINDOW);
    (window, if window >= ROOMY { ANSWER_CEILING } else { told.answer_room(ANSWER_CEILING) })
}

/// `(reserveTokens, keepRecentTokens)` for a room too small for Pi's defaults, `None` for a room
/// they suit. Pi compacts when the context (its prompt and every tool's schema included, about
/// 3,600 tokens in production) passes `contextWindow - reserveTokens`. The reserve is a quarter of
/// the room: what the next call may add (the person's message, a tool's result) before it would
/// not fit, and the answer's room. The part kept after a compaction is an eighth: the last two or
/// three exchanges. Measured on Pi 0.99.1 at 8,000 with the production extension: three eighths
/// compacted every second turn (the tools alone are nearly half the room), a quarter and an eighth
/// every fifth, with every call under 6,200 tokens (#1402).
pub(super) fn compaction(told: Room) -> Option<(u64, u64)> {
    let window = told.per_request()?;
    (window < ROOMY).then(|| (window / 4, window / 8))
}

/// settings.json with `compaction.modelOverrides["yantrik/<id>"]` for each model in `models` that
/// needs one. The overrides of ours that are no longer needed are dropped, and nothing else of
/// the person's file is touched.
pub(super) fn settings_text(current: Option<&str>, provider: &str, models: &[(String, Room)]) -> Result<Option<String>, String> {
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
    for (id, room) in models {
        if let Some((reserve, keep)) = compaction(*room) {
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

/// The ask bar's pick changed: Pi's files again with the rooms of the model now picked, under the
/// token Pi already holds (a running Pi keeps working with it until it is restarted). `None` when
/// Pi is not on Yantrik models, or when nothing Pi reads would change. The plan has no restart:
/// a running Pi is made to read the files again by its harness, between turns (`/yantrik-room`).
pub(crate) fn plan_for_new_pick(home: &Path, offer_with: impl FnOnce(super::Token) -> Result<super::Offer, String>) -> Result<Option<super::Plan>, String> {
    use super::Handoff as _;
    if !super::uses_gateway(home, super::pi::Pi.harness()) {
        return Ok(None);
    }
    let models = super::read_json_object(&home.join(".pi/agent/models.json"))?;
    let Some(held) = models.get("providers").and_then(|p| p.get(super::pi::PROVIDER)).and_then(|p| p.get("apiKey")).and_then(Value::as_str) else {
        return Ok(None);
    };
    let mut plan = super::pi::Pi.plan_gateway(home, &offer_with(super::Token(held.to_string()))?)?;
    let unchanged = plan.writes.iter().all(|w| std::fs::read_to_string(&w.path).is_ok_and(|live| live == w.content));
    plan.restart = None;
    Ok((!unchanged).then_some(plan))
}
