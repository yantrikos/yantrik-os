//! The local model gateway, served by the shell (crates/yantrik-gateway, #673): every mind pointed
//! at it once reaches the person's AI accounts, and the keys stay here.
//!
//! - `accounts`: what the gateway asks the shell — the models, an account's key at the moment of
//!   a call (from `ai_accounts`), and whether Private mode is on.
//! - this file: starting it on 127.0.0.1:7460, and where its tokens and its call log live.
//!
//! It lives as long as the shell does: a mind's call while the shell restarts fails, and the
//! harness says so, the same as its turn would.

mod accounts;

pub(crate) use accounts::private_ok;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use yantrik_gateway::log::CallLog;
use yantrik_gateway::upstream::Http;
use yantrik_gateway::{Gateway, Tokens};

use crate::bridge::CompanionBridge;

static GATEWAY: OnceLock<Arc<Gateway>> = OnceLock::new();
/// Why the gateway is not listening, when it is not.
static DOWN: OnceLock<String> = OnceLock::new();

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"))
}

/// The gateway's tokens, as SHA-256 hashes: `~/.config/yantrik/gateway/tokens.json`.
pub fn tokens_path() -> PathBuf {
    home().join(".config/yantrik/gateway/tokens.json")
}

/// Every call, without content: `~/.local/state/yantrik/gateway-calls.jsonl`.
pub fn log_path() -> PathBuf {
    home().join(".local/state/yantrik/gateway-calls.jsonl")
}

/// The tokens it accepts. Opened once; the same set the server checks.
pub fn tokens() -> Arc<Tokens> {
    static TOKENS: OnceLock<Arc<Tokens>> = OnceLock::new();
    // Tests never touch the person's own file.
    #[cfg(test)]
    return TOKENS.get_or_init(|| Arc::new(Tokens::in_memory())).clone();
    #[cfg(not(test))]
    TOKENS.get_or_init(|| Arc::new(Tokens::open(&tokens_path()))).clone()
}

/// The call log the server writes. The same one whether or not the server started.
pub fn log() -> Arc<CallLog> {
    static LOG: OnceLock<Arc<CallLog>> = OnceLock::new();
    #[cfg(test)]
    return LOG.get_or_init(|| Arc::new(CallLog::in_memory())).clone();
    #[cfg(not(test))]
    LOG.get_or_init(|| Arc::new(CallLog::at(log_path()))).clone()
}

/// `Ok` while listening; else why not ("127.0.0.1:7460 is in use").
pub fn state() -> Result<(), String> {
    match (GATEWAY.get(), DOWN.get()) {
        (Some(_), _) => Ok(()),
        (None, Some(why)) => Err(why.clone()),
        (None, None) => Err("not started yet".into()),
    }
}

/// Start listening, once. A port someone else holds is logged and said, never retried in a loop.
pub fn start(bridge: Arc<CompanionBridge>) {
    if GATEWAY.get().is_some() {
        return;
    }
    // The companion given Yantrik models goes back onto them at every start.
    crate::provider_handoff::start_companion(&home(), bridge.clone());
    let listener = match yantrik_gateway::server::bind(yantrik_gateway::ADDR) {
        Ok(l) => l,
        Err(why) => {
            tracing::warn!(error = %why, "the model gateway is not listening");
            let _ = DOWN.set(why);
            return;
        }
    };
    let gateway = Arc::new(Gateway {
        tokens: tokens(),
        accounts: Arc::new(accounts::ShellAccounts { bridge }),
        upstream: Arc::new(Http::default()),
        log: log(),
    });
    if GATEWAY.set(gateway.clone()).is_ok() {
        let _ = std::thread::Builder::new().name("model-gateway".into()).spawn(move || gateway.serve(listener));
        tracing::info!(addr = yantrik_gateway::ADDR, "the model gateway is listening");
    }
}
