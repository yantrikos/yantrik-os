//! The listener and the two routes. One thread per connection; a connection is one request.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use yantrik_ml::model_caps::ModelCaps;

use crate::body;
use crate::http::{self, error_body, write_json, Request};
use crate::log::{Call, CallLog};
use crate::policy::{self, Decision, Refusal, Sharing, Target};
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
    /// What the person chose for this account and private context ([`Sharing`]).
    pub sharing: Sharing,
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
    /// The private lane, `<account>/<model>`: the model on this machine or network that a call
    /// carrying private context goes to when the person keeps it on this computer for the account
    /// the call names. `None` when none is set up.
    fn private_lane(&self) -> Option<String> {
        None
    }
}

pub struct Gateway {
    pub tokens: Arc<Tokens>,
    pub accounts: Arc<dyn Accounts>,
    pub upstream: Arc<dyn Upstream>,
    pub log: Arc<CallLog>,
    /// Accounts whose server answered 400 to the template thinking switch (`chat_template_kwargs`).
    /// A `custom` account is any endpoint the person typed, so a hosted service serving a Qwen3 can
    /// sit behind one; it is asked once more without the switch, and never sent it again this session.
    pub template_refused: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Sampling parameters ([`body::DROPPABLE`]) a model's provider answered 400 to, by
    /// `<account>/<model>`: "`temperature` is deprecated for this model". The call goes again
    /// without it, once, and later calls to that model are sent none for as long as the gateway runs.
    pub params_refused: Arc<Mutex<std::collections::HashMap<String, Vec<&'static str>>>>,
}

/// How much of a refusing upstream's body is read to see which parameter it names. An error body
/// is a few hundred bytes; past this the rest is still passed through to the harness, unread.
const ERROR_PEEK: u64 = 16 * 1024;

/// The most connections served at once. The mind account can reach this port (its kernel loopback
/// set opens it), and a thread per connection with no bound would let it, or anything else on this
/// machine, grow the shell until it falls over. One past this is closed unread.
pub const MAX_CONNECTIONS: usize = 64;

/// How long a request may take to arrive, head and body, however slowly it drips in.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(60);

/// A connection being served, counted until it ends.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Whether a request's `Host` names this machine's loopback: what a harness sends. A web page that
/// rebinds its own name to 127.0.0.1 still sends its own name, and is refused.
pub fn loopback_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    let name = match h.strip_prefix('[') {
        Some(inside) => inside.split(']').next().unwrap_or(""),
        None => match h.rsplit_once(':') {
            Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => name,
            _ => h.as_str(),
        },
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
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
        let open = Arc::new(AtomicUsize::new(0));
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                open.fetch_sub(1, Ordering::SeqCst);
                drop(stream);
                continue;
            }
            let slot = Slot(open.clone());
            let me = self.clone();
            let _ = std::thread::Builder::new().name("gateway-conn".into()).spawn(move || {
                let _slot = slot;
                me.handle(stream)
            });
        }
    }

    pub fn handle(&self, stream: TcpStream) {
        self.handle_within(stream, REQUEST_DEADLINE)
    }

    /// [`Self::handle`], with the time the request has to arrive.
    pub fn handle_within(&self, mut stream: TcpStream, deadline: Duration) {
        // Loopback only, whatever the socket was bound to.
        if !stream.peer_addr().is_ok_and(|p| p.ip().is_loopback()) {
            return;
        }
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30).min(deadline)));
        let read = http::read_request(http::Deadline { inner: &mut stream, until: Instant::now() + deadline });
        let req = match read {
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
        // A page that rebound its own name to 127.0.0.1 still names itself in Host.
        if req.header("host").is_some_and(|h| !loopback_host(h)) {
            let _ = write_json(out, 403, &error_body("host_refused", "The model gateway answers 127.0.0.1 and localhost only."));
            return;
        }
        let Some(grant) = req.bearer().and_then(|t| self.tokens.verify(t)) else {
            let _ = write_json(
                out,
                401,
                &error_body("invalid_token", "This needs the token the desktop gave this mind (Settings → Minds → Use Yantrik models)."),
            );
            return;
        };
        let path = req.path.split('?').next().unwrap_or("");
        match (req.method.as_str(), path) {
            ("GET", "/v1/models") => {
                let _ = write_json(out, 200, &self.models_for(&grant));
            }
            ("POST", "/v1/chat/completions") => self.chat(&grant, req, out),
            // Which harness this token is (E.SOCK2). The Mind asks it of a token it is offered
            // before it saves it, and takes only its own: a process running as the person could
            // otherwise hand the Mind that process's token, and the Mind's calls would then run
            // under another harness's grant and pick. It names only the holder's own harness,
            // which the holder of the token already is.
            ("GET", "/v1/whoami") => {
                let _ = write_json(out, 200, &serde_json::json!({ "harness": grant.harness }));
            }
            (_, "/v1/models") | (_, "/v1/chat/completions") | (_, "/v1/whoami") => {
                let _ = write_json(out, 405, &error_body("method_not_allowed", "Not with that method."));
            }
            _ => {
                let _ = write_json(out, 404, &error_body("not_found", "The gateway serves /v1/models, /v1/chat/completions and /v1/whoami."));
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
            // An account set to Off would refuse this harness's every call; Private lane answers
            // them on this computer, so it stays listed.
            .filter(|m| !grant.private_context || m.local || m.sharing != Sharing::Off)
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
            lane_for: None,
            dropped: Vec::new(),
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
        let asked = body.get("model").and_then(Value::as_str).unwrap_or("").to_string();
        let routed = route_call(self.accounts.as_ref(), grant, &asked, |r| {
            call.account = r.account.clone();
            call.model = r.model.clone();
        })?;
        let (asked, mut target) = (routed.asked, routed.target);
        if let Some(meant) = routed.lane_for {
            // The log keeps which model answered, and which account it was meant for.
            call.lane_for = Some(meant);
            call.account = target.account.clone();
            call.model = target.model.clone();
        }
        if let Some(map) = body.as_object_mut() {
            if !map.contains_key("effort") && !map.contains_key("reasoning_effort") {
                if let Some(e) = self.accounts.effort_for(&grant.harness, &asked) {
                    map.insert("effort".into(), Value::String(e.as_str().into()));
                }
            }
        }
        let switch = yantrik_ml::model_caps::Reasoning::ChatTemplateThinking;
        if target.reasoning == switch && self.template_refused.lock().is_ok_and(|s| s.contains(&target.account)) {
            target.reasoning = yantrik_ml::model_caps::Reasoning::None;
            target.efforts = Vec::new();
        }
        let effort = body::rewrite(&mut body, &target)?;
        call.effort = effort.map(|e| e.as_str().to_string());
        // What this model is known to refuse goes before the first send: from the model's caps, or
        // from its provider's own 400 earlier in this run.
        let model_id = format!("{}/{}", target.account, target.model);
        let mut known: Vec<&'static str> = body::known_refused(&target).to_vec();
        if let Ok(refused) = self.params_refused.lock() {
            known.extend(refused.get(&model_id).into_iter().flatten().filter(|p| !known.contains(p)).collect::<Vec<_>>());
        }
        note_dropped(call, body::drop_params(&mut body, &known));
        // Thinking, once the effort mapping turned it on, narrows what sampling the provider takes.
        if let Some(map) = body.as_object_mut() {
            note_dropped(call, body::fit_sampling_to_thinking(map));
        }
        let bytes = serde_json::to_vec(&body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
        let mut reply = self
            .upstream
            .chat(&target.base_url, target.key.as_deref(), &bytes)
            .map_err(|u| Refusal::new(502, u.code, u.message))?;
        if reply.status == 400 {
            // The 400 arrives whole before any stream starts, so a streamed call is asked again the
            // same way. Its start is read to see what it refuses, and passed on as it was if not this.
            let mut peek = Vec::new();
            let _ = (&mut reply.body).take(ERROR_PEEK).read_to_end(&mut peek);
            let refused = body::refused_params(&peek, &body);
            let dropped = body::drop_params(&mut body, &refused);
            if dropped.is_empty() {
                let rest = std::mem::replace(&mut reply.body, Box::new(std::io::empty()));
                reply.body = Box::new(std::io::Cursor::new(peek).chain(rest));
            } else {
                note_dropped(call, dropped.clone());
                let bytes = serde_json::to_vec(&body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
                reply = self
                    .upstream
                    .chat(&target.base_url, target.key.as_deref(), &bytes)
                    .map_err(|u| Refusal::new(502, u.code, u.message))?;
                // Remembered for every harness only when dropping it was the difference. The error
                // text can echo what a caller sent, so a 400 alone is one caller's word; a success
                // without the parameter is the provider's.
                if (200..300).contains(&reply.status) {
                    if let Ok(mut remembered) = self.params_refused.lock() {
                        let entry = remembered.entry(model_id.clone()).or_default();
                        entry.extend(dropped.iter().filter(|p| !entry.contains(p)).collect::<Vec<_>>());
                    }
                    tracing::info!(model = %model_id, params = ?dropped, "the model's provider refused these parameters; they are sent no more this run");
                }
            }
        }
        if reply.status == 400 && target.reasoning == switch && effort.is_some() {
            // The switch was the gateway's, not the caller's: the call goes again without it, once.
            body::drop_template_switch(&mut body);
            let bytes = serde_json::to_vec(&body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
            reply = self
                .upstream
                .chat(&target.base_url, target.key.as_deref(), &bytes)
                .map_err(|u| Refusal::new(502, u.code, u.message))?;
            // Remembered only when the switch was the difference: a 400 for another reason (a context
            // too long, a bad tool schema) fails again without it, and leaves the switch on.
            if (200..300).contains(&reply.status) {
                call.effort = None;
                if let Ok(mut refused) = self.template_refused.lock() {
                    refused.insert(target.account.clone());
                }
                tracing::info!(account = %target.account, "the account's server refused chat_template_kwargs; it is sent no switch from now on");
            }
        }
        call.status = reply.status;
        if !(200..300).contains(&reply.status) {
            call.outcome = format!("upstream_{}", reply.status);
        }
        let _ = http::write_head(out, reply.status, &reply.content_type);
        pump(reply.body, out, call);
        Ok(())
    }
}

/// Where a call goes, as the gateway decides it before any byte is sent: `asked` resolved
/// ([`crate::PICKED`] to the harness's pick), the account's target, and the private lane in its
/// place when the person keeps private context on this computer for that account. A refusal is
/// what the harness is shown, and means nothing is sent.
#[derive(Debug)]
pub struct Routed {
    /// `<account>/<model>`, with [`crate::PICKED`] resolved.
    pub asked: String,
    /// Where the call goes: the account it names, or the private lane.
    pub target: Target,
    /// When it goes to the private lane: the `<account>/<model>` it was meant for.
    pub lane_for: Option<String>,
}

/// The gateway's one routing decision: what [`Gateway`] does with each call, and what the desktop
/// says about where words typed to a mind on the gateway go (the chat composer's line), so the two
/// cannot disagree. `parsed` sees the account and model the call names before its target is
/// looked up (the call log keeps them even when that refuses). `accounts.target` may hand back a
/// target with no key: nothing here reads it.
pub fn route_call(accounts: &dyn Accounts, grant: &Grant, asked: &str, parsed: impl FnOnce(&route::Route)) -> Result<Routed, Refusal> {
    let mut asked = asked.to_string();
    if asked == crate::PICKED {
        asked = accounts.picked_model(&grant.harness).ok_or_else(no_pick)?;
    }
    let r = route::parse(&asked).ok_or_else(|| {
        Refusal::new(404, "model_not_found", format!("`{asked}` is not a model here: name one as <account>/<model>, from /v1/models."))
    })?;
    parsed(&r);
    let target = accounts.target(&r.account, &r.model)?;
    if policy::decide(grant, &target, accounts.private_mode())? == Decision::Lane {
        let lane = lane(accounts, &target)?;
        return Ok(Routed { asked, target: lane, lane_for: Some(format!("{}/{}", r.account, r.model)) });
    }
    Ok(Routed { asked, target, lane_for: None })
}

/// The refusal for [`crate::PICKED`] when nothing is picked and nothing usable is there to pick.
pub const NO_PICK: &str = "No model is picked for this mind yet: pick one in the ask bar, or add an account under Settings → AI & Intelligence.";

fn no_pick() -> Refusal {
    Refusal::new(404, "model_not_found", NO_PICK)
}

/// The private lane's account and model, in place of `meant` for a call that carries private
/// context. Refused, with the fix named, when there is none or it is not on this machine or
/// network: a lane never sends anything off it.
fn lane(accounts: &dyn Accounts, meant: &Target) -> Result<Target, Refusal> {
    let lane = accounts.private_lane().and_then(|id| route::parse(&id)).ok_or_else(|| policy::not_shared(&meant.provider))?;
    let target = accounts.target(&lane.account, &lane.model)?;
    if !target.local {
        return Err(policy::not_shared(&meant.provider));
    }
    Ok(target)
}

/// Add what was taken out of a call's body to its log line: the parameter's name, never its value.
fn note_dropped(call: &mut Call, dropped: Vec<&'static str>) {
    for p in dropped {
        if !call.dropped.iter().any(|d| d == p) {
            call.dropped.push(p.to_string());
        }
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
