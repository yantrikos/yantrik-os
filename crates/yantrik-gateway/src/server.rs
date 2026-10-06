//! The listener and the two routes. One thread per connection; a connection is one request.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use yantrik_ml::model_caps::ModelCaps;

use crate::body;
use crate::http::{self, error_body, write_json, Request};
use crate::log::{Call, CallLog};
use crate::policy::{self, Refusal, Target};
use crate::route;
use crate::tokens::{Grant, Tokens};
use crate::upstream::Upstream;

/// One model the gateway can route to. No key.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelInfo {
    /// `<account>/<model>`.
    pub id: String,
    pub account: String,
    pub name: String,
    pub caps: ModelCaps,
    /// On this machine or the local network.
    pub local: bool,
    /// The person allowed this account to receive private context.
    pub private_ok: bool,
}

/// What the gateway asks the OS: its models, an account's key and address at the moment of a
/// call, and whether Private mode is on. Implemented by the shell (`crates/yantrik-ui/src/gateway`).
pub trait Accounts: Send + Sync {
    fn models(&self) -> Vec<ModelInfo>;
    /// The account and its key for this model. `Err` is a refusal the harness is shown (unknown
    /// account, key missing, vault locked).
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal>;
    fn private_mode(&self) -> bool;
    /// The effort the person picked for this harness and model, applied when a request names
    /// none: so a harness that never reads the turn's options still thinks as hard as was picked.
    fn effort_for(&self, _harness: &str, _model: &str) -> Option<yantrik_ml::model_caps::Effort> {
        None
    }
    /// The model the person picked for this harness, `<account>/<model>`: what [`crate::PICKED`]
    /// means for it. `None` when nothing it may use is there to pick.
    fn picked_model(&self, _harness: &str) -> Option<String> {
        None
    }
}

pub struct Gateway {
    pub tokens: Arc<Tokens>,
    pub accounts: Arc<dyn Accounts>,
    pub upstream: Arc<dyn Upstream>,
    pub log: Arc<CallLog>,
}

/// Bind the gateway's address. Anything but a loopback address is refused: the gateway adds the
/// person's keys to what it forwards, and nothing off this machine may ask it to.
pub fn bind(addr: &str) -> Result<TcpListener, String> {
    let parsed: SocketAddr = addr.parse().map_err(|_| format!("{addr} is not an address"))?;
    if !parsed.ip().is_loopback() {
        return Err(format!("the model gateway listens on loopback only, not {addr}"));
    }
    TcpListener::bind(parsed).map_err(|e| format!("could not listen on {addr}: {e}"))
}

impl Gateway {
    /// Serve until the listener fails. Each connection on a thread of its own.
    pub fn serve(self: Arc<Self>, listener: TcpListener) {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let me = self.clone();
            let _ = std::thread::Builder::new().name("gateway-conn".into()).spawn(move || me.handle(stream));
        }
    }

    pub fn handle(&self, mut stream: TcpStream) {
        // Loopback only, whatever the socket was bound to.
        if !stream.peer_addr().is_ok_and(|p| p.ip().is_loopback()) {
            return;
        }
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let req = match http::read_request(&mut stream) {
            Ok(r) => r,
            Err((status, why)) => {
                let _ = write_json(&mut stream, status, &error_body("bad_request", why));
                return;
            }
        };
        let _ = stream.set_read_timeout(None);
        self.respond(&req, &mut stream);
    }

    /// Answer one request into `out`. Split from the socket so it is tested without one.
    pub fn respond(&self, req: &Request, out: &mut impl Write) {
        // A browser page on this machine can reach loopback; no web page has any business here.
        if req.header("origin").is_some() {
            let _ = write_json(out, 403, &error_body("origin_refused", "The model gateway does not answer web pages."));
            return;
        }
        let Some(grant) = req.bearer().and_then(|t| self.tokens.verify(t)) else {
            let _ = write_json(
                out,
                401,
                &error_body("invalid_token", "This needs the token the desktop gave this mind (Settings → Harnesses → Use Yantrik models)."),
            );
            return;
        };
        let path = req.path.split('?').next().unwrap_or("");
        match (req.method.as_str(), path) {
            ("GET", "/v1/models") => {
                let _ = write_json(out, 200, &self.models_for(&grant));
            }
            ("POST", "/v1/chat/completions") => self.chat(&grant, req, out),
            (_, "/v1/models") | (_, "/v1/chat/completions") => {
                let _ = write_json(out, 405, &error_body("method_not_allowed", "Not with that method."));
            }
            _ => {
                let _ = write_json(out, 404, &error_body("not_found", "The gateway serves /v1/models and /v1/chat/completions."));
            }
        }
    }

    /// The models this harness may use, in OpenAI's shape, with what each can do beside it.
    pub fn models_for(&self, grant: &Grant) -> Value {
        let private_mode = self.accounts.private_mode();
        let mut data: Vec<Value> = vec![json!({
            "id": crate::PICKED,
            "object": "model",
            "owned_by": "yantrik",
            "yantrik": { "name": "The model picked in the ask bar", "picked": self.accounts.picked_model(&grant.harness) },
        })];
        let listed: Vec<Value> = self
            .accounts
            .models()
            .into_iter()
            .filter(|m| !(private_mode && !m.local))
            .filter(|m| !grant.private_context || m.local || m.private_ok)
            .map(|m| {
                json!({
                    "id": m.id,
                    "object": "model",
                    "owned_by": m.account,
                    "yantrik": {
                        "name": m.name,
                        "efforts": m.caps.efforts,
                        "vision": m.caps.vision,
                        "context": m.caps.context,
                        "tools": m.caps.tools,
                    },
                })
            })
            .collect();
        data.extend(listed);
        json!({ "object": "list", "data": data })
    }

    fn chat(&self, grant: &Grant, req: &Request, out: &mut impl Write) {
        let started = Instant::now();
        let mut call = Call {
            at: now(),
            harness: grant.harness.clone(),
            account: String::new(),
            model: String::new(),
            effort: None,
            status: 0,
            outcome: String::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
            ms: 0,
        };
        let result = self.forward(grant, req, out, &mut call);
        if let Err(refusal) = result {
            call.status = refusal.status;
            call.outcome = refusal.code.to_string();
            let _ = write_json(out, refusal.status, &error_body(refusal.code, &refusal.message));
        }
        call.ms = started.elapsed().as_millis() as u64;
        self.log.record(call);
    }

    fn forward(&self, grant: &Grant, req: &Request, out: &mut impl Write, call: &mut Call) -> Result<(), Refusal> {
        let mut body: Value = serde_json::from_slice(&req.body)
            .map_err(|_| Refusal::new(400, "bad_request", "The request body is not JSON."))?;
        let mut asked = body.get("model").and_then(Value::as_str).unwrap_or("").to_string();
        if asked == crate::PICKED {
            asked = self.accounts.picked_model(&grant.harness).ok_or_else(|| {
                Refusal::new(404, "model_not_found", "No model is picked for this mind yet: pick one in the ask bar, or add an account under Settings → AI & Intelligence.")
            })?;
        }
        let r = route::parse(&asked).ok_or_else(|| {
            Refusal::new(404, "model_not_found", format!("`{asked}` is not a model here: name one as <account>/<model>, from /v1/models."))
        })?;
        call.account = r.account.clone();
        call.model = r.model.clone();
        let target = self.accounts.target(&r.account, &r.model)?;
        policy::decide(grant, &target, self.accounts.private_mode())?;
        if let Some(map) = body.as_object_mut() {
            if !map.contains_key("effort") && !map.contains_key("reasoning_effort") {
                if let Some(e) = self.accounts.effort_for(&grant.harness, &asked) {
                    map.insert("effort".into(), Value::String(e.as_str().into()));
                }
            }
        }
        let effort = body::rewrite(&mut body, &target)?;
        call.effort = effort.map(|e| e.as_str().to_string());
        let bytes = serde_json::to_vec(&body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
        let reply = self
            .upstream
            .chat(&target.base_url, target.key.as_deref(), &bytes)
            .map_err(|u| Refusal::new(502, u.code, u.message))?;
        call.status = reply.status;
        if !(200..300).contains(&reply.status) {
            call.outcome = format!("upstream_{}", reply.status);
        }
        let _ = http::write_head(out, reply.status, &reply.content_type);
        pump(reply.body, out, call);
        Ok(())
    }
}

/// Copy the answer through as it arrives, counting the tokens it reports.
fn pump(mut body: Box<dyn Read + Send>, out: &mut impl Write, call: &mut Call) {
    let mut buf = [0u8; 16 * 1024];
    let mut tail = Vec::new();
    loop {
        match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if out.write_all(&buf[..n]).and_then(|_| out.flush()).is_err() {
                    call.outcome = "harness_left".into();
                    break;
                }
                // Usage arrives at the end (the last event of a stream, or the whole body), so the
                // last bytes are enough; nothing else of the content is kept.
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > 64 * 1024 {
                    tail.drain(..tail.len() - 32 * 1024);
                }
            }
            Err(_) => {
                call.outcome = "upstream_cut_off".into();
                break;
            }
        }
    }
    if let Some((p, c)) = usage(&tail) {
        call.prompt_tokens = p;
        call.completion_tokens = c;
    }
}

/// The last `usage` an answer reports: a JSON body, or the data lines of a stream.
pub fn usage(bytes: &[u8]) -> Option<(u64, u64)> {
    let text = String::from_utf8_lossy(bytes);
    let read = |v: &Value| {
        let u = v.get("usage")?;
        Some((u.get("prompt_tokens")?.as_u64().unwrap_or(0), u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0)))
    };
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return read(&v);
    }
    text.lines()
        .rev()
        .filter_map(|l| l.trim().strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
        .find_map(|v| read(&v))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
