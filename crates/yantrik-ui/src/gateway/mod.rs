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

pub(crate) use accounts::{room_of, sharing};

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
    TOKENS
        .get_or_init(|| {
            let tokens = Tokens::open(&tokens_path());
            // Every mind has YantrikDB memory: a token from before that carries private context too.
            if let Err(e) = tokens.mark_all_private() {
                tracing::warn!(error = %e, "gateway tokens were not marked as carrying private context");
            }
            Arc::new(tokens)
        })
        .clone()
}

/// The call log the server writes. The same one whether or not the server started.
pub fn log() -> Arc<CallLog> {
    static LOG: OnceLock<Arc<CallLog>> = OnceLock::new();
    #[cfg(test)]
    return LOG.get_or_init(|| Arc::new(CallLog::in_memory())).clone();
    #[cfg(not(test))]
    LOG.get_or_init(|| Arc::new(CallLog::at(log_path()))).clone()
}

/// Where a call from `harness` asking for `asked` ([`yantrik_gateway::PICKED`], or the model its
/// config names) goes, decided by the gateway's own routing (`server::route_call`) over the same
/// accounts, pick, consent and Private mode it serves with: what the chat composer's line says for
/// a mind on the gateway. No key is read. Every grant carries private context (`tokens()` marks
/// them all so), and so does this one.
pub(crate) fn route_for(harness: &str, asked: &str) -> Result<yantrik_gateway::server::Routed, yantrik_gateway::Refusal> {
    let accounts = accounts::ShellAccounts { keys: accounts::Keys::NotRead };
    let grant = yantrik_gateway::Grant { harness: harness.to_string(), private_context: true };
    yantrik_gateway::server::route_call(&accounts, &grant, asked, |_| {})
}

/// Whether what the built-in companion starts by itself is kept from the account its calls go to:
/// its calls go through the gateway now, and the gateway would send them to a tight account. The
/// same routing the gateway applies ([`route_for`]), so the companion does not start what the
/// gateway would refuse (`yantrik_ml::asked`).
///
/// "Now" is what it was last reloaded onto, not what its file says: a companion put on a saved
/// provider by "Use for the companion" answers from that provider while the file still names
/// Yantrik models, and it is not behind the gateway (machine test of #1299: it stopped learning
/// on a provider that has no limit at all).
fn companion_is_kept_for_asked() -> bool {
    kept_for_asked(crate::provider_handoff::companion::armed_on_gateway(), || {
        route_for(crate::wire::harness::BUILTIN_ID, yantrik_gateway::PICKED).is_ok_and(|routed| routed.target.tight)
    })
}

/// The decision of [`companion_is_kept_for_asked`]: the route is asked only for a companion that
/// is behind the gateway.
fn kept_for_asked(behind_the_gateway: bool, routed_to_a_tight_account: impl FnOnce() -> bool) -> bool {
    behind_the_gateway && routed_to_a_tight_account()
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
    yantrik_ml::asked::on_tight_check(companion_is_kept_for_asked);
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
        accounts: Arc::new(accounts::ShellAccounts { keys: accounts::Keys::Read(bridge) }),
        upstream: Arc::new(Http::default()),
        log: log(),
        template_refused: Default::default(),
        params_refused: Default::default(), health: Default::default(), holds: Default::default(),
    });
    if GATEWAY.set(gateway.clone()).is_ok() {
        let _ = std::thread::Builder::new().name("model-gateway".into()).spawn(move || gateway.serve(listener));
        tracing::info!(addr = yantrik_gateway::ADDR, "the model gateway is listening");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_companion_behind_the_gateway_and_routed_to_a_tight_account_is_kept_for_what_is_asked() {
        assert!(kept_for_asked(true, || true));
        assert!(!kept_for_asked(true, || false), "the account its calls go to is not tight");
        assert!(
            !kept_for_asked(false, || panic!("the route is not asked for a companion on a provider of its own")),
            "on a saved provider it goes on learning, whatever its pick on Yantrik models says"
        );
    }

    #[test]
    fn the_check_reads_what_the_companion_was_last_reloaded_onto_and_not_its_file() {
        let src = include_str!("mod.rs").split("#[cfg(test)]\nmod tests").next().unwrap();
        let check = &src[src.find("fn companion_is_kept_for_asked").unwrap()..src.find("fn kept_for_asked").unwrap()];
        assert!(check.contains("companion::armed_on_gateway()"));
        assert!(!check.contains("companion_on_gateway("), "the file names Yantrik models while a saved provider answers");
    }
}
