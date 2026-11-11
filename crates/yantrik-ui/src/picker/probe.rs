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
pub(super) fn account_of(model: &str) -> &str {
    model.split_once('/').map(|(a, _)| a).unwrap_or_default()
}

/// A model no account lists is answered at once, without a network call.
fn settle_not_offered(model: &str) {
    lock(&PROBES).insert(account_of(model).to_string(), Probe { at: now(), outcome: Outcome::NotOffered });
}

/// Ask the model's account whether it answers; the answer stands for a few minutes
/// (status::PROBE_FRESH_SECS).
pub(super) fn ask(bridge: &Arc<CompanionBridge>, model: &str) {
    let Some(m) = ai_accounts::catalogue().find(model).cloned() else {
        settle_not_offered(model);
        return;
    };
    let account = m.account.clone();
    if !lock(&PROBING).insert(account.clone()) {
        return;
    }
    let bridge = bridge.clone();
    let _ = std::thread::Builder::new().name("model-probe".into()).spawn(move || {
        let accounts = ai_accounts::accounts();
        let outcome = match accounts.iter().find(|a| a.id == account) {
            None => Outcome::KeyMissing,
            Some(a) => {
                let saved = crate::wire::settings::ProviderStore::load().entries;
                let vault = |id: &str| ai_accounts::vault_value(&bridge, id);
                match ai_accounts::keys::resolve(a, &saved, &vault) {
                    Err(_) => Outcome::KeyMissing,
                    Ok(r) if ai_accounts::models::needs_no_listing(a, yantrik_ml::provider::pool::tiers::FREE_TIERS) => {
                        // No list to ask: the smallest request that needs the key, one token.
                        match crate::wire::provider_models::check_key(&r.base_url, r.key.as_deref().unwrap_or(""), &m.model) {
                            crate::wire::provider_models::KeyCheck::Works => Outcome::Ok,
                            crate::wire::provider_models::KeyCheck::Refused => Outcome::KeyRefused,
                            crate::wire::provider_models::KeyCheck::Unconfirmed(why) => Outcome::Unreachable(why),
                        }
                    }
                    Ok(r) => match crate::wire::provider_models::list_models(&r.base_url, r.key.as_deref(), ai_accounts::keys::auth_for(a, r.key.as_deref())) {
                        Ok(_) => Outcome::Ok,
                        Err(crate::wire::provider_models::ListError::KeyRefused) => Outcome::KeyRefused,
                        Err(e) => Outcome::Unreachable(e.to_string()),
                    },
                }
            }
        };
        lock(&PROBES).insert(account.clone(), Probe { at: now(), outcome });
        lock(&PROBING).remove(&account);
    });
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
