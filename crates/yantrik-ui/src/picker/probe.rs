//! Asking a model's account whether it answers, so the status moves on from "checking": off the UI
//! thread, at most one at a time per account, and every ask ends in an answer, including "this
//! account does not offer that model" (it used to return without one, and "checking" never ended).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use super::status::{Outcome, Probe};
use super::{lock, now};
use crate::ai_accounts;
use crate::bridge::CompanionBridge;

static PROBES: LazyLock<Mutex<HashMap<String, Probe>>> = LazyLock::new(Default::default);
static PROBING: LazyLock<Mutex<std::collections::HashSet<String>>> = LazyLock::new(Default::default);

/// What was last found for an account, if anything.
pub(super) fn found(account: &str) -> Option<Probe> {
    lock(&PROBES).get(account).cloned()
}

/// Forget every answer, so the next draw asks again ("Try again").
pub(super) fn forget() {
    lock(&PROBES).clear();
}

/// The account part of `<account>/<model>`.
pub(crate) fn account_of(model: &str) -> &str {
    model.split_once('/').map(|(a, _)| a).unwrap_or_default()
}

/// Record what was found for an account.
fn settle(account: &str, outcome: Outcome) {
    lock(&PROBES).insert(account.to_string(), Probe { at: now(), outcome });
}

/// A model no account lists is answered at once, without a network call.
fn settle_not_offered(model: &str) {
    settle(account_of(model), Outcome::NotOffered);
}

/// Ask the model's account whether it answers; the answer stands for a few minutes
/// (status::PROBE_FRESH_SECS).
pub(super) fn ask(bridge: &Arc<CompanionBridge>, model: &str) {
    let Some(m) = ai_accounts::catalogue().find(model).cloned() else {
        // A tier switched Off or without its key leaves the catalogue: that is the reason said, not
        // "not offered". Asked of the gateway's own check; nothing is sent to the account.
        match ai_accounts::shared::refusal_for_id(account_of(model)).map(|r| r.code) {
            Some("tier_off") => settle(account_of(model), Outcome::SwitchedOff),
            Some("key_missing") => settle(account_of(model), Outcome::KeyMissing),
            _ => settle_not_offered(model),
        }
        return;
    };
    let account = m.account.clone();
    if !lock(&PROBING).insert(account.clone()) {
        return;
    }
    let bridge = bridge.clone();
    let _ = std::thread::Builder::new().name("model-probe".into()).spawn(move || {
        if let Some(outcome) = probe_outcome(&bridge, &account, &m.model) {
            lock(&PROBES).insert(account.clone(), Probe { at: now(), outcome });
        }
        lock(&PROBING).remove(&account);
    });
}

/// What the account answers, or `None` when nothing is known yet (the keys were not read). It
/// asks the gateway's own check first: a tier switched Off, or without its key, is not sent even a
/// one-token request.
pub(crate) fn probe_outcome(bridge: &Arc<CompanionBridge>, account: &str, model: &str) -> Option<Outcome> {
    if let Some(refused) = ai_accounts::shared::refusal_for_id(account) {
        return match refused.code {
            "tier_off" => Some(Outcome::SwitchedOff),
            "keys_not_known_yet" => None,
            _ => Some(Outcome::KeyMissing),
        };
    }
    let accounts = ai_accounts::accounts();
    let Some(a) = accounts.iter().find(|a| a.id == account) else { return Some(Outcome::Gone) };
    let saved = crate::wire::settings::ProviderStore::load().entries;
    let vault = |id: &str| ai_accounts::vault_value(bridge, id);
    let Ok(r) = ai_accounts::keys::resolve(a, &saved, &vault) else { return Some(Outcome::KeyMissing) };
    if ai_accounts::models::needs_no_listing(a, yantrik_ml::provider::pool::tiers::FREE_TIERS) {
        // No list to ask: the smallest request that needs the key, one token.
        return Some(match crate::wire::provider_models::check_key(&r.base_url, r.key.as_deref().unwrap_or(""), model) {
            crate::wire::provider_models::KeyCheck::Works => Outcome::Ok,
            crate::wire::provider_models::KeyCheck::Refused => Outcome::KeyRefused,
            crate::wire::provider_models::KeyCheck::Unconfirmed(why) => Outcome::Unreachable(why),
        });
    }
    Some(match crate::wire::provider_models::list_models(&r.base_url, r.key.as_deref(), ai_accounts::keys::auth_for(a, r.key.as_deref())) {
        Ok(_) => Outcome::Ok,
        Err(crate::wire::provider_models::ListError::KeyRefused) => Outcome::KeyRefused,
        Err(e) => Outcome::Unreachable(e.to_string()),
    })
}

/// Forget what was found about every account not in `kept`: one that left the list (a tier
/// switched Off, an account removed) must not keep saying "connected" from an older probe.
pub(crate) fn keep_probes_of(kept: &[String]) {
    lock(&PROBES).retain(|account, _| kept.contains(account));
}

#[cfg(test)]
pub(crate) fn probe_for_test(account: &str, found: Option<Outcome>) -> bool {
    match found {
        Some(outcome) => {
            lock(&PROBES).insert(account.to_string(), Probe { at: now(), outcome });
            true
        }
        None => lock(&PROBES).contains_key(account),
    }
}

#[cfg(test)]
mod tests {
    use super::super::status::{judge, Input};
    use super::*;
    use crate::answering::Answer;

    #[test]
    fn a_model_no_account_offers_is_answered_at_once_so_checking_cannot_loop() {
        let answer = Answer { account_id: "probe-test".into(), account: "Probe Test".into(), model: "gone".into(), lane: false, called_at: None };
        let input = Input { now: 1_000, on_gateway: true, answer: Some(answer.clone()), ..Input::default() };
        assert_eq!(judge(&input).state, "checking", "nothing has asked yet");
        settle_not_offered(&answer.id());
        let probe = found("probe-test").expect("the ask ended in an answer");
        assert_eq!(probe.outcome, Outcome::NotOffered);
        let after = judge(&Input { now: probe.at, probe: Some(probe), ..input });
        assert_ne!(after.state, "checking");
        assert!(after.words.contains("Probe Test"), "{}", after.words);
    }
}
