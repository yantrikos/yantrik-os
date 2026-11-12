//! What changed in what is shared, and what the gateway checks at the moment of a call.
//!
//! A free tier switched Off, a key removed, a saved account removed: each takes an account out of
//! the list the gateway and the picker read. [`sharing_changed`] is the one call every such place
//! makes. It drops what is no longer usable from the list and catalogue at once (no vault, no
//! network, so no wait), then lists again properly in the background. [`refusal`] is the backstop
//! the gateway asks at routing: a list that missed a change still cannot send words to a tier the
//! person turned off.

use std::collections::BTreeSet;
use std::sync::Arc;

use yantrik_gateway::Refusal;
use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use super::account::{withdrawn, Account, Source, Withdrawn};
use crate::bridge::CompanionBridge;
use crate::wire::settings::ProviderStore;

/// After anything that changes what is shared. One function, called from every place.
pub fn sharing_changed(bridge: Arc<CompanionBridge>) {
    prune_to_shared();
    super::refresh(bridge);
}

/// Drop, now, every free account that is switched off or has lost its key, and every removed saved account, from the list and the
/// catalogue, and tell the listeners (the picker, the runs-on card). Never adds an account: that
/// is the listing's work.
pub fn prune_to_shared() {
    let (kept, off) = crate::wire::free_ai::kept_and_off();
    prune(&kept, &off);
}

fn prune(kept: &BTreeSet<String>, off: &BTreeSet<String>) {
    let saved: BTreeSet<String> = ProviderStore::load().entries.into_iter().map(|e| e.id).collect();
    prune_with(kept, off, &saved);
}

/// `saved`: the ids of the saved providers still there; an account made from one that is not has
/// been removed.
fn prune_with(kept: &BTreeSet<String>, off: &BTreeSet<String>, saved: &BTreeSet<String>) {
    let list: Vec<Account> = super::accounts()
        .into_iter()
        .filter(|a| usable(a, kept, off) && a.entry.as_ref().is_none_or(|e| saved.contains(e)))
        .collect();
    let mut catalogue = super::catalogue();
    catalogue.accounts.retain(|g| list.iter().any(|a| a.id == g.account));
    super::install(list, catalogue);
}

fn usable(a: &Account, kept: &BTreeSet<String>, off: &BTreeSet<String>) -> bool {
    withdrawn_now(a, kept, off).is_none()
}

fn withdrawn_now(a: &Account, kept: &BTreeSet<String>, off: &BTreeSet<String>) -> Option<Withdrawn> {
    if a.source != Source::Free {
        return None;
    }
    withdrawn(FREE_TIERS.iter().find(|t| t.id == a.provider_type)?, kept, off)
}

/// The gateway's check at routing, against what the card says now and not what the list says:
/// `Some` when the account is a free tier the person switched Off, or whose key is gone.
pub fn refusal(a: &Account) -> Option<Refusal> {
    let (kept, off) = crate::wire::free_ai::kept_and_off();
    refusal_with(a, &kept, &off)
}

fn refusal_with(a: &Account, kept: &BTreeSet<String>, off: &BTreeSet<String>) -> Option<Refusal> {
    Some(match withdrawn_now(a, kept, off)? {
        Withdrawn::Off => Refusal::new(
            403,
            "tier_off",
            format!("{} isn't offered any more: it was switched Off in Settings → AI → Free AI accounts. Nothing was sent to it. Switch it On there to use it again.", a.name),
        ),
        Withdrawn::NoKey => Refusal::new(
            503,
            "key_missing",
            format!("{} isn't offered any more: its key was removed. Nothing was sent to it. Add a key in Settings → AI → Free AI accounts to use it again.", a.name),
        ),
    })
}

#[cfg(test)]
pub(super) fn prune_with_for_test(saved: &BTreeSet<String>) {
    let (kept, off) = crate::wire::free_ai::kept_and_off();
    prune_with(&kept, &off, saved);
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
