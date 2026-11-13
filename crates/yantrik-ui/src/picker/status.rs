//! What the picker and the mind panel say about whether the mind's model works: "connected ·
//! <model>" only after a real successful call or a cheap probe, and otherwise what is wrong, with
//! the one press that fixes it. Never a configured name dressed as health: the panel used to read
//! "ready · yantrik-4b" on a fresh install with nothing listening.
//!
//! Pure: the mind, its last gateway call, the probe and the gateway's state are handed in. The model
//! it names is the one `answering` resolves, said in its words.

use yantrik_gateway::log::Call;
use yantrik_gateway::Refusal;

use crate::answering::Answer;

/// How long a probe's answer stands before it is asked again.
pub const PROBE_FRESH_SECS: i64 = 180;
/// How long a successful call says "connected" without another.
pub const CALL_FRESH_SECS: i64 = 600;

/// The one press that moves a status on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    None,
    /// Point the mind at Yantrik models (its row's card).
    SetUp,
    /// Settings → AI & Intelligence: add an account, or its key.
    AddKey,
    /// Allow private context for the account (AI accounts).
    Allow,
    /// Ask again now.
    Retry,
    /// Open the model menu: the pick has gone (the chip itself opens it).
    PickAgain,
}

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Fix::None => "",
            Fix::SetUp => "Set up",
            Fix::AddKey => "Add key",
            Fix::Allow => "Choose",
            Fix::Retry => "Try again",
            Fix::PickAgain => "Pick again",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// "connected", "checking", "not-set-up", "key-missing", "unreachable", "not-allowed", "own",
    /// "pick-gone" (the pick is offered by no account any more), "set" (the built-in companion's
    /// endpoint, not called yet).
    pub state: &'static str,
    pub words: String,
    pub fix: Fix,
}

impl Status {
    fn new(state: &'static str, words: impl Into<String>, fix: Fix) -> Status {
        Status { state, words: words.into(), fix }
    }
}

/// What a probe of an account found, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub at: i64,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    KeyRefused,
    KeyMissing,
    Unreachable(String),
    /// No account lists the model.
    NotOffered,
}

/// Everything the status is decided from.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub now: i64,
    /// The mind is pointed at the gateway.
    pub on_gateway: bool,
    /// The model answering it, or that will (`answering::answering`); `None` when the gateway
    /// would refuse its next call, which `refusal` says.
    pub answer: Option<Answer>,
    pub refusal: Option<Refusal>,
    /// The sentence for a pick that has gone, when the default pick answers in its place.
    pub fallback: Option<String>,
    /// Its newest call through the gateway, if any.
    pub last_call: Option<Call>,
    /// The probe of the model's account, if one was made.
    pub probe: Option<Probe>,
    /// The gateway is not listening, and why.
    pub gateway_down: Option<String>,
    /// A mind with its own models: when it last answered a turn, if it did since the shell
    /// started, and the model it says it runs.
    pub own_answered_at: Option<i64>,
    pub own_model: String,
    /// The built-in companion, off the gateway: what it runs on, as `runs_on` resolves it from the
    /// companion's real backend ("" when nothing is set up). `None` for every other mind.
    pub builtin_runs_on: Option<String>,
}

pub fn judge(i: &Input) -> Status {
    if !i.on_gateway {
        // The built-in companion's provider is the desktop's own configuration: said as it is,
        // never "its own models" (13 Oct: it said so while answering from the endpoint the first
        // run had just put it on).
        if let Some(line) = i.builtin_runs_on.as_deref() {
            if line.is_empty() {
                return Status::new("not-set-up", "not set up · choose who does the thinking", Fix::AddKey);
            }
            return match i.own_answered_at {
                Some(at) if i.now - at <= CALL_FRESH_SECS => Status::new("connected", format!("connected · {line}"), Fix::None),
                _ => Status::new("set", format!("runs on {line}"), Fix::None),
            };
        }
        // Its own models: the desktop cannot see its provider, only whether it answered.
        return match i.own_answered_at {
            Some(at) if i.now - at <= CALL_FRESH_SECS => {
                let m = i.own_model.trim();
                Status::new("connected", if m.is_empty() { "connected · its own model".to_string() } else { format!("connected · {m}") }, Fix::None)
            }
            _ => Status::new("own", "uses its own models", Fix::SetUp),
        };
    }
    if let Some(why) = &i.gateway_down {
        return Status::new("unreachable", format!("unreachable · the model gateway is not running ({why})"), Fix::Retry);
    }
    let Some(answer) = &i.answer else { return refused(i.refusal.as_ref()) };
    if let Some(sentence) = &i.fallback {
        return Status::new("pick-gone", sentence.clone(), Fix::PickAgain);
    }
    let words = answer.words();
    // The newest real call for this model decides, while it is fresh.
    if let Some(c) = i.last_call.as_ref().filter(|c| format!("{}/{}", c.account, c.model) == answer.id()) {
        if i.now - c.at <= CALL_FRESH_SECS {
            if c.ok() {
                return Status::new("connected", words, Fix::None);
            }
            if let Some(failed) = failed_with(c) {
                return failed;
            }
        }
    }
    match &i.probe {
        Some(p) if i.now - p.at <= PROBE_FRESH_SECS => match &p.outcome {
            Outcome::Ok => Status::new("connected", words, Fix::None),
            Outcome::KeyRefused | Outcome::KeyMissing => Status::new("key-missing", "key missing", Fix::AddKey),
            Outcome::Unreachable(_) => Status::new("unreachable", "unreachable", Fix::Retry),
            Outcome::NotOffered => Status::new("pick-gone", format!("{} · {} is not offered any more", answer.account, answer.model), Fix::PickAgain),
        },
        _ => Status::new("checking", words, Fix::None),
    }
}

/// What a call that failed says, by the code the gateway logged for it.
fn failed_with(c: &Call) -> Option<Status> {
    match c.outcome.as_str() {
        "key_missing" | "vault_locked" | "upstream_401" | "upstream_403" => Some(Status::new("key-missing", "key missing", Fix::AddKey)),
        "private_context_not_shared" | "private_context_not_allowed" => Some(not_shared()),
        "account_not_found" | "model_not_found" => Some(Status::new("not-set-up", "not set up · that account is gone", Fix::AddKey)),
        "private_mode" => Some(private_mode()),
        _ if c.status >= 500 || c.outcome == "unreachable" => Some(Status::new("unreachable", "unreachable", Fix::Retry)),
        _ => None,
    }
}

/// What the gateway would refuse the next call for, when it names no model.
fn refused(refusal: Option<&Refusal>) -> Status {
    match refusal.map(|r| (r.code, r.message.as_str())) {
        Some(("private_context_not_shared" | "private_context_not_allowed", _)) => not_shared(),
        Some(("private_mode", _)) => private_mode(),
        Some(("account_not_found", _)) => Status::new("not-set-up", "not set up · that account is gone", Fix::AddKey),
        Some(("pick_gone", sentence)) => Status::new("pick-gone", sentence, Fix::PickAgain),
        _ => Status::new("not-set-up", "not set up · pick a model", Fix::AddKey),
    }
}

fn not_shared() -> Status {
    Status::new("not-allowed", "not shared · set up a private lane or allow sharing", Fix::Allow)
}

fn private_mode() -> Status {
    Status::new("not-allowed", "Private mode · local models only", Fix::None)
}
