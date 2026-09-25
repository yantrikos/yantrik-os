//! Reach the companion from an app.
//!
//! The LLM, the memory and the bond live in the shell process. An app talks to them the same way
//! it talks to any service — over the socket bus — so an AI action is a normal RPC call and not
//! a stub.
//!
//! Every call answers `Err` when the shell is not running, which is a state an app must handle
//! rather than hide: these binaries are meant to run on their own too.

use std::time::Duration;

use yantrik_ipc_transport::SyncRpcClient;

/// How long to wait for an answer. A local model on a cold cache is slow, and a caller that
/// gives up early leaves the user with nothing.
const ASK_TIMEOUT: Duration = Duration::from_secs(90);

fn client() -> SyncRpcClient {
    SyncRpcClient::for_service("companion").with_timeout(ASK_TIMEOUT)
}

/// Why an ask did not produce an answer.
///
/// One type and one sentence per case, because a dozen apps used to wrap a raw string in a
/// dozen different sentences of their own — and three of them ended with "Is the Yantrik shell
/// running?", which is the wrong question when the shell is running and just said it has no
/// model.
#[derive(Debug)]
pub enum AskError {
    /// The shell answered, and no model did — none set up, or the one set up did not answer.
    /// Its canned fallback text stays inside the shell: show [`NO_MODEL_HINT`], and write
    /// nothing into a document, a note or a draft.
    NoModel,
    /// No answer at all: no shell, a timeout, or a refusal.
    Failed(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::NoModel => f.write_str(NO_MODEL_HINT),
            AskError::Failed(reason) => write!(f, "The companion did not answer: {reason}"),
        }
    }
}

impl AskError {
    /// What the agent rail should say after this failure.
    ///
    /// A shell that answered "no model" is running, and telling the person to start it is the
    /// wrong hint — which is the one this app-facing sentence can still get wrong.
    pub fn hint(&self) -> &'static str {
        match self {
            AskError::NoModel => NO_MODEL_HINT,
            AskError::Failed(_) => OFFLINE_HINT,
        }
    }
}

/// Ask the companion something and get the finished answer.
///
/// `Ok` is always a model's words: a turn the shell's offline responder served comes back as
/// [`AskError::NoModel`], never as the canned text it produced.
pub fn ask(prompt: &str) -> Result<String, AskError> {
    let response = client()
        .call(
            "companion.ask",
            serde_json::json!({ "prompt": prompt, "timeout_ms": ASK_TIMEOUT.as_millis() as u64 }),
        )
        .map_err(|e| ask_error(e.code, &e.message))?;
    response
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AskError::Failed("companion returned no text".to_string()))
}

/// The wire error to the one typed error, split out so a test can pin the mapping with no
/// socket and no shell.
fn ask_error(code: i32, message: &str) -> AskError {
    if code == yantrik_ipc_contracts::ERR_NO_MODEL {
        AskError::NoModel
    } else {
        AskError::Failed(format!("[{code}] {message}"))
    }
}

/// One memory the companion recalled.
#[derive(Debug, Clone)]
pub struct Recalled {
    pub rid: String,
    pub text: String,
    pub score: f64,
}

/// Search the companion's memory.
pub fn recall(query: &str, limit: usize) -> Result<Vec<Recalled>, String> {
    let response = client()
        .call("companion.recall", serde_json::json!({ "query": query, "limit": limit }))
        .map_err(|e| format!("[{}] {}", e.code, e.message))?;
    let rows = response
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "companion returned no results".to_string())?;
    Ok(rows
        .iter()
        .map(|r| Recalled {
            rid: r.get("rid").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            text: r.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            score: r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
        })
        .collect())
}

/// Recall, filtered to what is actually relevant.
///
/// `recall` returns its best `limit` results, and "best" is not "relevant": on a fresh machine
/// the best match for a note about quarterly planning was the companion's own telemetry --
/// "App opened: yantrik-notes", scoring 9%. An agent rail that shows that is not surfacing
/// context, it is surfacing noise with a number on it, and one junk row costs more trust than
/// three good rows earn.
///
/// So every caller that puts recall results in front of a person goes through here. Over-fetch,
/// filter, then take: asking for `want` directly and filtering afterwards leaves you with two
/// rows when three were available.
pub fn recall_relevant(query: &str, floor: f64, want: usize) -> Vec<Recalled> {
    recall(query, (want * 3).max(6))
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.score >= floor)
        .take(want)
        .collect()
}

/// The floor every app uses, so "relevant" means the same thing in all of them.
pub const RELEVANCE_FLOOR: f64 = 0.35;

/// What to tell someone when the shell is not there.
///
/// One sentence, said once at the top of the rail rather than by every row failing separately.
/// Running an app on its own is a supported thing to do, not an error.
pub const OFFLINE_HINT: &str = "Not connected. Start the Yantrik shell for memory and suggestions.";

/// What to tell someone when the shell is there and no model answered.
///
/// Its own sentence because "start the Yantrik shell" — what every AI control said whenever the
/// backend had not answered — sends the person to start the one thing that is already running.
/// It says only what is known, that nothing answered: the machine that reported the bug had a
/// model set up (Ollama on the host) that was not running, so "no model is set up" — what this
/// said first — told that person something false.
pub const NO_MODEL_HINT: &str = "No AI model answered. Check Settings \u{2192} AI.";

/// What to tell someone when an AI control exists in the markup but nothing is built behind it.
///
/// Shelved apps used to open the panel and leave it empty — the handlers wrote a log line and
/// returned — so a press looked like a request in flight that never landed. One sentence shared
/// by every shelved app, because "not yet" should mean the same thing everywhere it appears.
pub const NOT_BUILT_YET: &str = "Not available yet — the AI in this app is still being built.";

/// Run one of the companion's tools by name, without a model in the loop.
///
/// The companion carries ~178 of them — files, windows, browser, containers, packages — behind
/// the permission ceiling in its config. An app that wants one thing done should ask for that
/// thing rather than describe it in a prompt and hope: this is a function call, and it works
/// when the model is unavailable.
///
/// The result is the tool's own prose, which is what tools return; a caller wanting structure
/// parses it.
pub fn tool(name: &str, args: serde_json::Value) -> Result<String, String> {
    let response = client()
        .call(
            "companion.tool",
            serde_json::json!({
                "name": name,
                "args": args,
                "timeout_ms": ASK_TIMEOUT.as_millis() as u64,
            }),
        )
        .map_err(|e| format!("[{}] {}", e.code, e.message))?;
    response
        .get("result")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "companion returned no result".to_string())
}

/// Whether the shell can answer at all, and when it cannot, which of two different situations
/// the person is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The shell is running and its model answered the last time it was asked.
    Ready,
    /// The shell is running and no model answered — none set up, or the one set up did not
    /// answer and the last ask fell back to canned text.
    NoModel,
    /// No shell is running to ask.
    NoShell,
}

impl Reach {
    /// The one sentence to show in place of an AI action, or `None` when the action can go
    /// ahead.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Reach::Ready => None,
            Reach::NoModel => Some(NO_MODEL_HINT),
            Reach::NoShell => Some(OFFLINE_HINT),
        }
    }
}

/// Ask the shell which situation this is, before offering an AI action or saying why one
/// cannot run.
///
/// Cheap — one status call, the one `is_online` made. It replaces that bool, which could not
/// tell "no shell" from "shell with no model" and so had a dozen apps answer both with "start
/// the Yantrik shell".
pub fn reach() -> Reach {
    reach_of(client().call("companion.status", serde_json::json!({})))
}

/// The reading of a status reply, split out so a test can pin the distinction with no socket
/// and no shell.
fn reach_of(status: Result<serde_json::Value, yantrik_ipc_transport::RpcError>) -> Reach {
    match status {
        // A reply at all is the shell running; the flag inside says whether its model answered
        // last time.
        Ok(v) if v.get("online").and_then(|o| o.as_bool()) == Some(true) => Reach::Ready,
        Ok(_) => Reach::NoModel,
        Err(_) => Reach::NoShell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc_error(code: i32) -> yantrik_ipc_transport::RpcError {
        yantrik_ipc_transport::RpcError { code, message: "no".into(), data: None }
    }

    /// The core of the defect: a no-model refusal used to arrive as an ordinary string, and
    /// every app showed it — or something wrapped around it — as the answer.
    #[test]
    fn the_no_model_code_maps_to_the_one_sentence() {
        let e = ask_error(yantrik_ipc_contracts::ERR_NO_MODEL, "no AI model is set up");
        assert!(matches!(e, AskError::NoModel));
        assert_eq!(e.to_string(), "No AI model answered. Check Settings \u{2192} AI.");
        assert_eq!(e.hint(), NO_MODEL_HINT);
    }

    #[test]
    fn any_other_failure_keeps_its_reason_and_the_old_hint() {
        let e = ask_error(-32000, "companion timed out");
        assert_eq!(e.to_string(), "The companion did not answer: [-32000] companion timed out");
        assert_eq!(e.hint(), OFFLINE_HINT);
    }

    /// A shell that answers "not online" is running: the hint must not send the person to
    /// start it. That wrong sentence is what every AI control showed after the first offline
    /// reply flipped the flag.
    #[test]
    fn reach_tells_a_missing_shell_apart_from_a_shell_with_no_model() {
        assert_eq!(reach_of(Ok(serde_json::json!({ "online": true }))), Reach::Ready);
        assert_eq!(reach_of(Ok(serde_json::json!({ "online": false }))), Reach::NoModel);
        assert_eq!(reach_of(Err(rpc_error(-1))), Reach::NoShell);

        assert_eq!(Reach::Ready.hint(), None);
        assert_eq!(Reach::NoModel.hint(), Some(NO_MODEL_HINT));
        assert_eq!(Reach::NoShell.hint(), Some(OFFLINE_HINT));
    }
}
