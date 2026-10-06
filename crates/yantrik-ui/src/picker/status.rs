//! What the picker and the mind panel say about whether the mind's model works: "connected ·
//! <model>" only after a real successful call or a cheap probe, and otherwise what is wrong, with
//! the one press that fixes it. Never a configured name dressed as health: the panel used to read
//! "ready · yantrik-4b" on a fresh install with nothing listening.
//!
//! Pure: the mind, its last gateway call, the probe and the gateway's state are handed in.

use yantrik_gateway::log::Call;

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
}

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Fix::None => "",
            Fix::SetUp => "Set up",
            Fix::AddKey => "Add key",
            Fix::Allow => "Allow",
            Fix::Retry => "Try again",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// "connected", "checking", "not-set-up", "key-missing", "unreachable", "not-allowed", "own".
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
}

/// Everything the status is decided from.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub now: i64,
    /// The mind is pointed at the gateway.
    pub on_gateway: bool,
    /// The model it answers with, `<account>/<model>`, or "" when none is picked.
    pub model: String,
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
}

/// The model's own name, for "connected · <model>": the part after the account.
fn short(model: &str) -> &str {
    model.split_once('/').map(|(_, m)| m).unwrap_or(model)
}

pub fn judge(i: &Input) -> Status {
    if !i.on_gateway {
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
    if i.model.is_empty() {
        return Status::new("not-set-up", "not set up · pick a model", Fix::AddKey);
    }
    let short = short(&i.model);
    // The newest real call for this model decides, while it is fresh.
    if let Some(c) = i.last_call.as_ref().filter(|c| format!("{}/{}", c.account, c.model) == i.model) {
        if i.now - c.at <= CALL_FRESH_SECS {
            if c.ok() {
                return Status::new("connected", format!("connected · {short}"), Fix::None);
            }
            match c.outcome.as_str() {
                "key_missing" | "vault_locked" | "upstream_401" | "upstream_403" => {
                    return Status::new("key-missing", "key missing", Fix::AddKey)
                }
                "private_context_not_allowed" => return Status::new("not-allowed", "not allowed · private context", Fix::Allow),
                "account_not_found" | "model_not_found" => return Status::new("not-set-up", "not set up · that account is gone", Fix::AddKey),
                "private_mode" => return Status::new("not-allowed", "Private mode · local models only", Fix::None),
                _ if c.status >= 500 || c.outcome == "unreachable" => return Status::new("unreachable", "unreachable", Fix::Retry),
                _ => {}
            }
        }
    }
    match &i.probe {
        Some(p) if i.now - p.at <= PROBE_FRESH_SECS => match &p.outcome {
            Outcome::Ok => Status::new("connected", format!("connected · {short}"), Fix::None),
            Outcome::KeyRefused | Outcome::KeyMissing => Status::new("key-missing", "key missing", Fix::AddKey),
            Outcome::Unreachable(_) => Status::new("unreachable", "unreachable", Fix::Retry),
        },
        _ => Status::new("checking", format!("checking · {short}"), Fix::None),
    }
}
