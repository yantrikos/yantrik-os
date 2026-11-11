//! `await_app`: wait, with a bounded poll, until an app's `describe` says something became true.
//!
//! Split from `app_ui.rs`, which reads and drives the apps; this is only the waiting.

use std::time::Duration;

use super::{PermissionLevel, Tool, ToolContext};
use crate::app_ui::{beyond_cap, client, describe, describe_outcome, ACT_TIMEOUT};

// ── Waiting for something to become true ──

/// How often an outstanding wait re-reads its target.
///
/// A `describe` costs about 0.4 ms, so twenty reads a second is under one percent of one core for
/// as long as somebody is actually waiting — and nothing at all when nobody is. That is the whole
/// argument for polling our own apps instead of instrumenting them: the cost is bounded, it is
/// paid only during a wait, and there is no setter anywhere that somebody can forget to annotate.
const WAIT_TICK: Duration = Duration::from_millis(50);

/// The longest any single wait may run.
///
/// Not a guess about how long work takes — a bound on how long one tool call may hold a worker.
/// Anything slower than this wants the job board, not a wait.
const MAX_WAIT: Duration = Duration::from_secs(120);

const DEFAULT_WAIT: Duration = Duration::from_secs(10);

/// One bounded question about an app's state.
///
/// Deliberately not a script. A predicate that could run arbitrary code would be a second, worse
/// action surface — and every predicate worth having is one of these five.
#[derive(Debug, Clone, PartialEq)]
enum Until {
    /// Anything at all differs from the baseline.
    Changed,
    /// A dotted path into `state` equals this value.
    Equals { path: String, value: serde_json::Value },
    /// A dotted path into `state` is anything other than this value.
    Differs { path: String, value: serde_json::Value },
    /// The summary, or the value at a path, contains this text. Case-insensitive.
    Contains { path: String, text: String },
    /// A dotted path into `state` exists at all — how a dialog that was not there is waited for.
    Exists { path: String },
}

impl Until {
    fn parse(spec: &serde_json::Value) -> Result<Until, String> {
        let op = spec.get("op").and_then(|v| v.as_str()).unwrap_or("changed");
        let path = spec.get("path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let value = spec.get("value").cloned();

        let needs_path = |what: &str| -> Result<String, String> {
            if path.is_empty() {
                Err(format!("`{what}` needs a `path`, e.g. \"open_note\" or \"dialog.title\""))
            } else {
                Ok(path.clone())
            }
        };
        let needs_value = |what: &str| -> Result<serde_json::Value, String> {
            value.clone().ok_or_else(|| format!("`{what}` needs a `value` to compare against"))
        };

        match op {
            "changed" => Ok(Until::Changed),
            "eq" => Ok(Until::Equals { path: needs_path("eq")?, value: needs_value("eq")? }),
            "ne" => Ok(Until::Differs { path: needs_path("ne")?, value: needs_value("ne")? }),
            "exists" => Ok(Until::Exists { path: needs_path("exists")? }),
            "contains" => {
                let text = needs_value("contains")?;
                let text = text.as_str().map(str::to_string).unwrap_or_else(|| text.to_string());
                // An empty path means the summary, which is the common case: "wait until the
                // window says Saved".
                Ok(Until::Contains { path, text })
            }
            other => Err(format!(
                "unknown `op` {other}; use changed, eq, ne, contains or exists"
            )),
        }
    }

    /// Whether this holds of `view`, given what the same app looked like before.
    fn holds(&self, view: &serde_json::Value, baseline: &serde_json::Value) -> bool {
        match self {
            Until::Changed => {
                revision_of(view) != revision_of(baseline) || revision_of(view).is_empty()
            }
            Until::Equals { path, value } => at(view, path).as_ref() == Some(value),
            Until::Differs { path, value } => at(view, path).as_ref() != Some(value),
            Until::Exists { path } => at(view, path).is_some_and(|v| !v.is_null()),
            Until::Contains { path, text } => {
                let haystack = if path.is_empty() {
                    view.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string()
                } else {
                    match at(view, path) {
                        Some(serde_json::Value::String(s)) => s,
                        Some(other) => other.to_string(),
                        None => String::new(),
                    }
                };
                haystack.to_lowercase().contains(&text.to_lowercase())
            }
        }
    }

    fn describe(&self) -> String {
        match self {
            Until::Changed => "anything changes".to_string(),
            Until::Equals { path, value } => format!("{path} becomes {value}"),
            Until::Differs { path, value } => format!("{path} stops being {value}"),
            Until::Exists { path } => format!("{path} appears"),
            Until::Contains { path, text } => {
                if path.is_empty() {
                    format!("the summary mentions {text:?}")
                } else {
                    format!("{path} mentions {text:?}")
                }
            }
        }
    }
}

fn revision_of(view: &serde_json::Value) -> String {
    view.get("revision").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// A dotted path into a view's `state`.
fn at(view: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let mut here = view.get("state")?;
    for segment in path.split('.').filter(|s| !s.is_empty()) {
        here = here.get(segment)?;
    }
    Some(here.clone())
}

pub struct AwaitAppTool;

impl Tool for AwaitAppTool {
    fn name(&self) -> &'static str {
        "await_app"
    }
    /// Standard rather than Safe, because `then_act` really does act.
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Standard
    }
    fn category(&self) -> &'static str {
        "app"
    }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "await_app",
                "description": "Wait until a Yantrik app reaches some state — a dialog appears, a \
                                save finishes, a list stops being empty. Give `then_act` when you \
                                are waiting for the result of an action: the app is read *before* \
                                the action runs, so a result that arrives immediately is not \
                                missed. Use this instead of acting and then describing, which \
                                cannot tell 'already true' from 'just became true'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "App id, e.g. notes" },
                        "until": {
                            "type": "object",
                            "description": "What to wait for. {op: changed} — anything differs. \
                                            {op: eq|ne, path, value} — a field in the app's state \
                                            reaches (or leaves) a value. {op: exists, path} — a \
                                            field appears. {op: contains, text} — the summary \
                                            mentions text, or a field does if you give a path. \
                                            Paths are dotted, into the `state` from describe_app.",
                            "properties": {
                                "op": { "type": "string" },
                                "path": { "type": "string" },
                                "value": {},
                                "text": { "type": "string" }
                            }
                        },
                        "then_act": {
                            "type": "object",
                            "description": "An action to run once the baseline read is taken: \
                                            {action, args}. Optional.",
                            "properties": {
                                "action": { "type": "string" },
                                "args": { "type": "object" }
                            }
                        },
                        "timeout_ms": {
                            "type": "integer",
                            "description": "How long to wait. Default 10000, maximum 120000."
                        }
                    },
                    "required": ["app", "until"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let app = args["app"].as_str().unwrap_or("").trim();
        if app.is_empty() {
            return "Error: `app` is required. Call list_apps to see which are open.".to_string();
        }
        let until = match Until::parse(args.get("until").unwrap_or(&serde_json::json!({}))) {
            Ok(u) => u,
            Err(e) => return format!("Error: {e}"),
        };
        let budget = args
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_WAIT)
            .min(MAX_WAIT);

        // 1. Baseline, before anything else happens.
        //
        // The ordering is the whole point. Acting first and reading afterwards cannot distinguish
        // "the dialog opened" from "the dialog was already open", and a wait installed after a
        // click misses the result that arrived during the round trip.
        if let Err(said) = crate::app_start::ensure(app, args.get("then_act").is_some_and(|v| v.is_object())) {
            return said;
        }
        let baseline = match describe(app) {
            Ok(v) => v,
            Err(e) => return format!("'{app}' did not answer: {}", e),
        };

        // 2. Act, if this is a wait for the consequences of an action. Guarded on the baseline we
        //    just took, so an app that moved between the read and the dispatch refuses rather
        //    than acting on a state nobody decided about.
        let mut dispatched = String::new();
        if let Some(act) = args.get("then_act").filter(|v| v.is_object()) {
            let action = act.get("action").and_then(|v| v.as_str()).unwrap_or("").trim();
            if action.is_empty() {
                return "Error: `then_act` needs an `action`.".to_string();
            }
            if let Some(refusal) = beyond_cap(&baseline, app, action, ctx.max_permission) {
                return refusal;
            }
            let call = serde_json::json!({
                "action": action,
                "args": act.get("args").cloned().unwrap_or(serde_json::json!({})),
                "expect_revision": revision_of(&baseline),
            });
            match client(app, ACT_TIMEOUT).call("app.act", call) {
                Ok(reply) => dispatched = describe_outcome(app, action, &reply),
                Err(e) => return format!("{app}.{action} failed: {}", e.message),
            }
        }

        // 3. Reconcile, then await. The first check happens immediately: for an action that
        //    settled on return, the condition is already true and waiting 50 ms to notice would
        //    be 50 ms of nothing.
        let started = std::time::Instant::now();
        let mut polls = 0u32;
        let mut latest = baseline.clone();
        loop {
            if let Ok(view) = describe(app) {
                latest = view;
                polls += 1;
                if until.holds(&latest, &baseline) {
                    return held(app, &until, &latest, &dispatched, started.elapsed(), polls);
                }
            }
            if started.elapsed() >= budget {
                return timed_out(app, &until, &latest, &dispatched, budget, polls);
            }
            std::thread::sleep(WAIT_TICK);
        }
    }
}

fn held(
    app: &str,
    until: &Until,
    view: &serde_json::Value,
    dispatched: &str,
    elapsed: Duration,
    polls: u32,
) -> String {
    let summary = view.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let mut out = String::new();
    if !dispatched.is_empty() {
        out.push_str(dispatched);
        out.push('\n');
    }
    out.push_str(&format!(
        "Waited for {app}: {} — held after {} ms ({polls} reads).\nNow: {summary}\nrevision: {}",
        until.describe(),
        elapsed.as_millis(),
        revision_of(view),
    ));
    out
}

fn timed_out(
    app: &str,
    until: &Until,
    view: &serde_json::Value,
    dispatched: &str,
    budget: Duration,
    polls: u32,
) -> String {
    let summary = view.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let mut out = String::new();
    if !dispatched.is_empty() {
        out.push_str(dispatched);
        out.push('\n');
    }
    out.push_str(&format!(
        "Waited for {app}: {} — not observed in {} ms ({polls} reads).\nNow: {summary}\n\
         This is a read every {} ms, so a state that appeared and vanished in between was never \
         visible to it. Not observed is not the same as did not happen.",
        until.describe(),
        budget.as_millis(),
        WAIT_TICK.as_millis(),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

// ── The wait ──

fn view(summary: &str, state: serde_json::Value, revision: &str) -> serde_json::Value {
    serde_json::json!({ "app": "notes", "summary": summary, "state": state, "revision": revision })
}

#[test]
fn a_predicate_is_one_of_five_things_and_never_a_script() {
    assert_eq!(Until::parse(&serde_json::json!({ "op": "changed" })).unwrap(), Until::Changed);
    assert!(Until::parse(&serde_json::json!({ "op": "eval", "code": "1" })).is_err());
    // A comparison with nothing to compare against is a mistake worth naming rather than
    // silently treating as "changed".
    let err = Until::parse(&serde_json::json!({ "op": "eq", "path": "x" })).unwrap_err();
    assert!(err.contains("value"), "{err}");
    let err = Until::parse(&serde_json::json!({ "op": "exists" })).unwrap_err();
    assert!(err.contains("path"), "{err}");
}

#[test]
fn waiting_for_a_dialog_that_was_not_there() {
    // The canonical reflex. `exists` is what distinguishes "the dialog opened" from "some
    // field happens to be null".
    let before = view("Notes", serde_json::json!({ "open_note": "x" }), "aaaa");
    let after = view(
        "Notes — Save changes?",
        serde_json::json!({ "open_note": "x", "dialog": { "title": "Save changes?" } }),
        "bbbb",
    );
    let until = Until::parse(&serde_json::json!({ "op": "exists", "path": "dialog" })).unwrap();

    assert!(!until.holds(&before, &before));
    assert!(until.holds(&after, &before));
}

#[test]
fn a_dotted_path_reaches_into_nested_state() {
    let now = view("Notes", serde_json::json!({ "dialog": { "title": "Save changes?" } }), "b");
    let until = Until::parse(
        &serde_json::json!({ "op": "eq", "path": "dialog.title", "value": "Save changes?" }),
    )
    .unwrap();
    assert!(until.holds(&now, &now));

    // And a path through something that is not an object is absent, not a panic.
    let flat = view("Notes", serde_json::json!({ "dialog": 3 }), "c");
    assert!(!until.holds(&flat, &flat));
}

#[test]
fn changed_compares_against_the_baseline_not_against_nothing() {
    // Why the baseline is taken before the action rather than after: without it, "changed"
    // has nothing to be different from, and every wait would return immediately.
    let before = view("Notes — a", serde_json::json!({ "n": 1 }), "1111");
    let same = view("Notes — a", serde_json::json!({ "n": 1 }), "1111");
    let moved = view("Notes — b", serde_json::json!({ "n": 2 }), "2222");

    assert!(!Until::Changed.holds(&same, &before), "an unchanged app has not changed");
    assert!(Until::Changed.holds(&moved, &before));
}

#[test]
fn contains_reads_the_summary_when_no_path_is_given() {
    let now = view("Notes — Saved", serde_json::json!({}), "z");
    let until = Until::parse(&serde_json::json!({ "op": "contains", "value": "saved" })).unwrap();
    assert!(until.holds(&now, &now), "matching should not depend on capitalisation");

    let until = Until::parse(&serde_json::json!({ "op": "contains", "value": "error" })).unwrap();
    assert!(!until.holds(&now, &now));
}

#[test]
fn a_wait_that_ran_out_says_what_it_could_not_have_seen() {
    // The polling contract, stated where somebody will read it. A model told only "not found"
    // will report that the dialog never appeared.
    let now = view("Notes", serde_json::json!({}), "q");
    let text = timed_out(
        "notes",
        &Until::Exists { path: "dialog".into() },
        &now,
        "",
        Duration::from_millis(4000),
        80,
    );
    assert!(text.contains("Not observed is not the same as did not happen"), "{text}");
    assert!(text.contains("4000 ms"));
}
}
