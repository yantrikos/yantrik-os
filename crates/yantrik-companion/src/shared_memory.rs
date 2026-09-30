//! The machine's shared memory, which the other minds already use (#31).
//!
//! Hermes and Yantrik Mind remember through one server: Yantrik Mind's memory, served over MCP at
//! `127.0.0.1:7440/mcp` (`yantrik-memory` when no mind owns it), guarded by a bearer token kept in
//! a file beside its database. The built-in companion kept a store of its own, so it could not see
//! what they remembered and they could not see what it learned. Switching the answering mind
//! then lost what the person had said (#245). This client lets the companion read and write the
//! same memory:
//! - recall runs beside the companion's own recall, as a separate section of its prompt;
//! - what it learns about the person from a conversation is written through, in the `default`
//!   namespace every mind shares.
//!
//! The server is optional. It may be down, not installed, or starting. Every call is bounded to
//! about two seconds, and after a failure the client stays quiet for a minute, so a turn is
//! never held up waiting for it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Where the server listens, unless `YANTRIK_MEMORY_URL` says otherwise.
pub const DEFAULT_URL: &str = "http://127.0.0.1:7440/mcp";
/// After a failed call, how long the client leaves the server alone.
const QUIET_AFTER_FAILURE: Duration = Duration::from_secs(60);
/// The protocol version this client speaks; the server says which it agreed to.
const PROTOCOL: &str = "2025-03-26";

/// One thing the shared memory recalled.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// "belief" or "memory".
    pub kind: String,
    pub text: String,
    /// Which mind wrote it, when the server says.
    pub source: String,
}

/// The token file the server keeps beside its database.
fn default_token_file() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("YANTRIK_MEMORY_TOKEN_FILE") {
        return Some(path.into());
    }
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".local/share/yantrik-mind/yantrik-memory.token"))
}

/// A connection to the shared memory, if this machine has one.
pub struct SharedMemory {
    url: String,
    token_file: Option<std::path::PathBuf>,
    agent: ureq::Agent,
    session: Mutex<Option<Session>>,
    next_id: AtomicU64,
    quiet_until: Mutex<Option<Instant>>,
}

#[derive(Clone)]
struct Session {
    id: String,
    token: String,
    protocol: String,
}

impl SharedMemory {
    /// A client for the server at `url`, whose token is read from `token_file` when a session starts.
    pub fn new(url: impl Into<String>, token_file: Option<std::path::PathBuf>) -> Self {
        SharedMemory {
            url: url.into(),
            token_file,
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_millis(500))
                .timeout(Duration::from_millis(2000))
                .build(),
            session: Mutex::new(None),
            next_id: AtomicU64::new(1),
            quiet_until: Mutex::new(None),
        }
    }

    /// The machine's shared memory at its usual address.
    pub fn machine() -> &'static SharedMemory {
        static SHARED: std::sync::OnceLock<SharedMemory> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| {
            SharedMemory::new(
                std::env::var("YANTRIK_MEMORY_URL").unwrap_or_else(|_| DEFAULT_URL.to_string()),
                default_token_file(),
            )
        })
    }

    /// What the shared memory recalls about `query`: beliefs and memories together. Empty when
    /// the server cannot be reached, which is never an error for the caller.
    pub fn recall(&self, query: &str, top_k: usize) -> Vec<Hit> {
        let args = json!({ "query": query, "top_k": top_k.clamp(1, 20), "include": "all" });
        match self.call("recall", args) {
            Ok(result) => hits_from(&result),
            Err(e) => {
                tracing::debug!(error = %e, "shared memory: recall skipped");
                Vec::new()
            }
        }
    }

    /// Write a fact learned about the person, for every mind on this machine to recall.
    pub fn remember(&self, text: &str, importance: f64, domain: &str) -> Result<(), String> {
        let args = json!({
            "text": text,
            "importance": importance.clamp(0.0, 1.0),
            "namespace": "default",
            "domain": domain,
            "source": "yantrik-companion",
        });
        self.call("remember", args).map(|_| ())
    }

    /// Call one of the server's tools and return the JSON its text content holds.
    fn call(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        if let Some(until) = *self.quiet_until.lock().unwrap_or_else(|e| e.into_inner()) {
            if Instant::now() < until {
                return Err("the shared memory failed recently; leaving it alone for now".into());
            }
        }
        let outcome = self.call_once(tool, &arguments).or_else(|first| {
            // A session the server no longer knows (it restarted) is started again, once.
            *self.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.call_once(tool, &arguments).map_err(|second| format!("{first}; then {second}"))
        });
        *self.quiet_until.lock().unwrap_or_else(|e| e.into_inner()) =
            outcome.is_err().then(|| Instant::now() + QUIET_AFTER_FAILURE);
        outcome
    }

    fn call_once(&self, tool: &str, arguments: &Value) -> Result<Value, String> {
        let session = self.session()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments },
        });
        let (_, reply) = self.post(&request, Some(&session))?;
        let reply = reply.ok_or("the server sent no answer")?;
        tool_result(&reply)
    }

    /// The open session, or a new one: `initialize`, then `notifications/initialized`.
    fn session(&self) -> Result<Session, String> {
        if let Some(open) = self.session.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(open);
        }
        let path = self.token_file.clone().ok_or("no home directory to find the memory token in")?;
        let token = std::fs::read_to_string(&path)
            .map_err(|e| format!("no shared memory on this machine ({}: {e})", path.display()))?
            .trim()
            .to_string();
        let init = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "yantrik-companion", "version": env!("CARGO_PKG_VERSION") },
            },
        });
        let bare = Session { id: String::new(), token, protocol: PROTOCOL.to_string() };
        let (id, reply) = self.post(&init, Some(&bare))?;
        let id = id.ok_or("the server started no session")?;
        let protocol = reply
            .as_ref()
            .and_then(|r| r["result"]["protocolVersion"].as_str())
            .unwrap_or(PROTOCOL)
            .to_string();
        let session = Session { id, token: bare.token, protocol };
        self.post(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), Some(&session))?;
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session.clone());
        Ok(session)
    }

    /// POST one JSON-RPC message. Returns the session id the server named, if any, and the
    /// JSON-RPC reply, read from either a JSON body or an event stream.
    fn post(&self, body: &Value, session: Option<&Session>) -> Result<(Option<String>, Option<Value>), String> {
        let mut request = self
            .agent
            .post(&self.url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream");
        if let Some(s) = session {
            request = request.set("Authorization", &format!("Bearer {}", s.token));
            if !s.id.is_empty() {
                request = request.set("Mcp-Session-Id", &s.id).set("Mcp-Protocol-Version", &s.protocol);
            }
        }
        let response = request.send_string(&body.to_string()).map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("the shared memory answered HTTP {code}"),
            other => format!("the shared memory could not be reached: {other}"),
        })?;
        let session_id = response.header("mcp-session-id").map(str::to_string);
        if response.status() == 202 {
            return Ok((session_id, None));
        }
        let streamed = response.content_type().starts_with("text/event-stream");
        let text = response.into_string().map_err(|e| format!("reading the shared memory's answer: {e}"))?;
        let reply = if streamed { reply_in_stream(&text, body.get("id")) } else { serde_json::from_str(&text).ok() };
        Ok((session_id, reply))
    }
}

/// The JSON-RPC reply to request `id` in an event stream: the `data:` of the event carrying it.
fn reply_in_stream(stream: &str, id: Option<&Value>) -> Option<Value> {
    stream
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|message| id.map_or(true, |id| message.get("id") == Some(id)))
}

/// The JSON a tool answered with (its text content), or its error in words.
fn tool_result(reply: &Value) -> Result<Value, String> {
    if let Some(error) = reply.get("error") {
        return Err(format!("the shared memory refused: {}", error["message"].as_str().unwrap_or("no reason given")));
    }
    let result = &reply["result"];
    let text = result["content"]
        .as_array()
        .and_then(|parts| parts.iter().find_map(|p| p["text"].as_str()))
        .unwrap_or_default();
    if result["isError"].as_bool() == Some(true) {
        return Err(format!("the shared memory refused: {text}"));
    }
    Ok(serde_json::from_str(text).unwrap_or(Value::String(text.to_string())))
}

/// The hits in a `recall` answer, beliefs and memories as the server interleaved them.
fn hits_from(result: &Value) -> Vec<Hit> {
    result["results"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let kind = row["kind"].as_str()?.to_string();
                    let text = row["statement"].as_str().or_else(|| row["text"].as_str())?.trim().to_string();
                    (!text.is_empty()).then(|| Hit { kind, text, source: row["source"].as_str().unwrap_or("").to_string() })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The section of the companion's prompt for what the shared memory recalled. None when it
/// recalled nothing, so no empty heading reaches the model.
pub fn prompt_section(hits: &[Hit], already: &[String]) -> Option<String> {
    let fresh: Vec<&Hit> = hits
        .iter()
        .filter(|h| !already.iter().any(|a| a.trim().eq_ignore_ascii_case(&h.text)))
        .collect();
    if fresh.is_empty() {
        return None;
    }
    let mut out = String::from(
        "What this machine's shared memory holds (written by the minds on it, the person's own words among them):\n",
    );
    for hit in fresh {
        let label = if hit.kind == "belief" { "believed" } else { "remembered" };
        out.push_str(&format!("- ({label}) {}\n", hit.text));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_read_from_an_event_stream_by_its_id() {
        let stream = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                      data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        let reply = reply_in_stream(stream, Some(&json!(7))).expect("the reply to 7");
        assert_eq!(reply["result"]["ok"], true);
        assert_eq!(reply_in_stream(stream, Some(&json!(8))), None);
    }

    /// The server's recall answer, as `mind-memory-mcp` shapes it: beliefs carry `statement`,
    /// memories carry `text`, each says its kind.
    #[test]
    fn recall_hits_are_read_whichever_half_they_come_from() {
        let text = json!({
            "query": "town model", "count": 2,
            "results": [
                { "kind": "belief", "statement": "The person is building a town model in Blender", "confidence": 0.8 },
                { "kind": "memory", "text": "Made ~/Documents/Town/renders", "source": "hermes" },
                { "kind": "memory", "text": "   " },
            ],
        })
        .to_string();
        let reply = json!({ "jsonrpc": "2.0", "id": 3, "result": { "content": [{ "type": "text", "text": text }] } });
        let hits = hits_from(&tool_result(&reply).unwrap());
        assert_eq!(hits.len(), 2, "an empty text is no memory");
        assert_eq!((hits[0].kind.as_str(), hits[1].source.as_str()), ("belief", "hermes"));
    }

    #[test]
    fn a_refusal_is_an_error_in_words() {
        let refused = json!({ "jsonrpc": "2.0", "id": 1, "result": { "isError": true, "content": [{ "type": "text", "text": "recall beliefs: locked" }] } });
        assert_eq!(tool_result(&refused).unwrap_err(), "the shared memory refused: recall beliefs: locked");
        let rpc = json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32602, "message": "bad params" } });
        assert_eq!(tool_result(&rpc).unwrap_err(), "the shared memory refused: bad params");
    }

    /// What the companion already recalled from its own store is not said twice.
    #[test]
    fn the_prompt_section_skips_what_is_already_there_and_is_absent_when_empty() {
        let hits = vec![
            Hit { kind: "memory".into(), text: "The person prefers dark mode".into(), source: "hermes".into() },
            Hit { kind: "belief".into(), text: "The person lives in Bentonville".into(), source: String::new() },
        ];
        let section = prompt_section(&hits, &["the person prefers dark mode".into()]).unwrap();
        assert!(section.contains("- (believed) The person lives in Bentonville"), "{section}");
        assert!(!section.contains("dark mode"), "{section}");
        assert_eq!(prompt_section(&hits[..1], &["The person prefers dark mode".into()]), None);
        assert_eq!(prompt_section(&[], &[]), None);
    }

    /// Against a real `yantrik-memory` (Yantrik Mind's `mind-memory-mcp`): what the companion
    /// shares, the next recall finds. Run by hand with a server up:
    /// `YANTRIK_MEMORY_IT_URL=http://127.0.0.1:17440/mcp YANTRIK_MEMORY_IT_TOKEN=<its token file>
    /// cargo test -p yantrik-companion shared_memory -- --ignored`.
    #[test]
    #[ignore = "needs a running yantrik-memory server"]
    fn what_the_companion_shares_a_real_server_recalls() {
        let (Some(url), Some(token)) =
            (std::env::var_os("YANTRIK_MEMORY_IT_URL"), std::env::var_os("YANTRIK_MEMORY_IT_TOKEN"))
        else {
            panic!("set YANTRIK_MEMORY_IT_URL and YANTRIK_MEMORY_IT_TOKEN");
        };
        let shared = SharedMemory::new(url.to_string_lossy(), Some(token.into()));
        shared.remember("The person keeps the town model in ~/Documents/Town", 0.7, "work").expect("stored");
        let hits = shared.recall("where is the town model kept", 5);
        assert!(
            hits.iter().any(|h| h.text == "The person keeps the town model in ~/Documents/Town" && h.source == "yantrik-companion"),
            "{hits:?}"
        );
    }

    /// No server at the address: the call fails fast, and the client then stays quiet rather
    /// than trying again on every turn.
    #[test]
    fn an_unreachable_server_costs_one_short_try_then_nothing() {
        let dir = std::env::temp_dir().join(format!("shared-memory-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let token = dir.join("yantrik-memory.token");
        std::fs::write(&token, "t").unwrap();
        // Port 9 (discard) on loopback: nothing answers.
        let shared = SharedMemory::new("http://127.0.0.1:9/mcp", Some(token));
        let started = Instant::now();
        assert!(shared.recall("anything", 5).is_empty());
        assert!(started.elapsed() < Duration::from_secs(5), "bounded: {:?}", started.elapsed());
        let again = Instant::now();
        assert!(shared.recall("anything", 5).is_empty());
        assert!(again.elapsed() < Duration::from_millis(50), "quiet after a failure: {:?}", again.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
