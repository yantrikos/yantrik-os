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
mod turn_keys;

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

/// What each account has been sent today, counts only: `~/.local/state/yantrik/gateway-day.json`.
pub fn day_path() -> PathBuf {
    home().join(".local/state/yantrik/gateway-day.json")
}

/// Each model's use today against its account's published daily limits, in words, for the
/// account named: "Groq (free), openai/gpt-oss-120b: 150K of 200K tokens and 12 of 1,000 requests
/// today; the companion has used 50K tokens." Empty when the gateway is not up or the account
/// publishes no daily limit.
pub fn today_words(account: &str, provider: &str) -> Vec<String> {
    GATEWAY.get().map(|g| g.day_lines(account).iter().map(|l| l.words(provider)).collect()).unwrap_or_default()
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

/// What a companion call of its own typically takes of a minute's allowance (26 Nov: about 6,300).
const OWN_CALL_TOKENS: u64 = 6_300;

/// Whether what the built-in companion starts by itself should wait now: the gateway's "now" has a
/// turn open, or the key its calls go to lacks room this minute, or the account is tight. The route
/// is the gateway's own ([`route_for`]), so the companion does not start what the gateway would
/// refuse (`yantrik_ml::asked`).
///
/// "Behind the gateway" is what it was last reloaded onto, not what its file says: a companion put
/// on a saved provider by "Use for the companion" answers from that provider while the file still
/// names Yantrik models (machine test of #1299: it stopped learning on a provider with no limit).
fn companion_must_wait() -> yantrik_ml::asked::Wait {
    let now = yantrik_gateway::now::shared();
    let holds = GATEWAY.get().map(|g| g.holds.clone());
    must_wait(
        crate::provider_handoff::companion::armed_on_gateway(),
        // The companion's own turn is not a reason for it to wait on itself: what it does inside
        // that turn (learning from the exchange) is the turn's tail.
        now.any_open_besides(crate::wire::harness::BUILTIN_ID) || (now.is_open_for(crate::wire::harness::BUILTIN_ID) && !crate::turn_source::in_the_persons_turn()),
        || route_for(crate::wire::harness::BUILTIN_ID, yantrik_gateway::PICKED).ok().map(|r| (r.target.tight, format!("{}/{}", r.target.account, r.target.model), room_of(&r.target.account, &r.target.model).per_minute)),
        |key| holds.as_ref().map_or(0, |h| h.used_last_minute(key)),
    )
}

/// The decision of [`companion_must_wait`]; the route is asked only for a companion behind the gateway.
fn must_wait(
    behind_the_gateway: bool,
    turn_open: bool,
    route: impl FnOnce() -> Option<(bool, String, Option<u64>)>,
    used: impl Fn(&str) -> u64,
) -> yantrik_ml::asked::Wait {
    use yantrik_ml::asked::Wait;
    if !behind_the_gateway {
        return Wait::No;
    }
    if turn_open {
        return Wait::TurnOpen;
    }
    match route() {
        Some((true, ..)) => Wait::Tight,
        Some((false, key, Some(per_minute))) if per_minute.saturating_sub(used(&key)) < OWN_CALL_TOKENS.min(per_minute / 2) => Wait::NoRoom,
        _ => Wait::No,
    }
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
    yantrik_ml::asked::on_wait_check(companion_must_wait);
    // One wake-up when the last open turn closes: what was paused for it runs again.
    yantrik_gateway::now::shared().on_close(|| {
        // The keys a turn held through a lock go with it (#1430).
        turn_keys::release();
        let _ = std::thread::Builder::new().name("resume-paused".into()).spawn(yantrik_ml::asked::turn_closed);
    });
    // The companion given Yantrik models goes back onto them at every start.
    crate::provider_handoff::start_companion(&home(), bridge.clone());
    // Pi's model and compaction files follow this build's numbers (an update does not rewrite them): at
    // a start with the accounts readable, and again whenever they are read (the shell starts locked,
    // so that is after the unlock); a running Pi reads them again before its next turn (#1402).
    std::thread::spawn(crate::wire::harness_provider::pick_changed);
    crate::ai_accounts::on_change(|| {
        std::thread::spawn(crate::wire::harness_provider::pick_changed);
    });
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
        params_refused: Default::default(),
        health: Default::default(),
        holds: Arc::new(yantrik_gateway::hold::Holds::with_state(day_path())),
    });
    if GATEWAY.set(gateway.clone()).is_ok() {
        let _ = std::thread::Builder::new().name("model-gateway".into()).spawn(move || gateway.serve(listener));
        tracing::info!(addr = yantrik_gateway::ADDR, "the model gateway is listening");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use yantrik_ml::asked::Wait;

    #[test]
    fn only_a_companion_behind_the_gateway_waits_and_why() {
        let tight = || Some((true, "a/m".to_string(), None));
        assert_eq!(must_wait(true, false, tight, |_| 0), Wait::Tight);
        assert_eq!(must_wait(true, false, || Some((false, "a/m".into(), None)), |_| 0), Wait::No, "not tight");
        assert_eq!(must_wait(true, true, || panic!("a turn is open: no route is asked"), |_| 0), Wait::TurnOpen);
        assert_eq!(
            must_wait(false, true, || panic!("on a saved provider it goes on, whatever its pick says"), |_| 0),
            Wait::No
        );
    }

    #[test]
    fn a_key_without_room_this_minute_waits() {
        let route = || Some((false, "groq/gpt-oss-120b".to_string(), Some(8_000)));
        assert_eq!(must_wait(true, false, route, |_| 6_000), Wait::NoRoom, "2,000 left of 8,000 is under one own call");
        assert_eq!(must_wait(true, false, route, |_| 1_000), Wait::No);
    }

    #[test]
    fn the_check_reads_what_the_companion_was_last_reloaded_onto_and_not_its_file() {
        let src = include_str!("mod.rs").split("#[cfg(test)]\nmod tests").next().unwrap();
        let check = &src[src.find("fn companion_must_wait").unwrap()..src.find("fn must_wait").unwrap()];
        assert!(check.contains("companion::armed_on_gateway()"));
        assert!(!check.contains("companion_on_gateway("), "the file names Yantrik models while a saved provider answers");
    }
}
