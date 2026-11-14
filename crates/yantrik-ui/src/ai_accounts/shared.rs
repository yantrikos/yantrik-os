//! What changed in what is shared, and what the gateway checks at the moment of a call.
//!
//! A free tier switched Off, a key removed, a saved account removed: each takes an account out of
//! the list the gateway and the picker read. [`sharing_changed`] is the one call every such place
//! makes. It drops what is no longer usable from the list and catalogue at once (no vault, no
//! network, so no wait), moves the built-in companion off a saved provider that is gone, then
//! lists again properly in the background. A listing already running when this happens is
//! corrected by a second pass; until then, [`refusal`] and [`refusal_for_id`] hold at each call:
//! the gateway checks them at routing against what the card says now, so a list that missed a
//! change still cannot send words to a tier the person turned off.

use std::collections::BTreeSet;
use std::sync::Arc;

use yantrik_gateway::Refusal;
use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use super::account::{free_id, free_name, withdrawn, Account, Source, Withdrawn};
use crate::bridge::CompanionBridge;
use crate::wire::settings::ProviderStore;

/// After anything that changes what is shared. One function, called from every place.
pub fn sharing_changed(bridge: Arc<CompanionBridge>) {
    prune_to_shared();
    crate::picker::keep_probes_of(&super::accounts().into_iter().map(|a| a.id).collect::<Vec<_>>());
    companion_off_a_removed_provider(&bridge, &home());
    super::refresh(bridge);
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("/root"))
}

/// The built-in companion, when it is not on Yantrik models, holds its provider's key in memory
/// and talks to it itself: the gateway's check never sees that. When the saved provider it runs on
/// is gone, it is stopped at once, then moved to the new primary if there is one: a move that
/// cannot get its key leaves it stopped, so its next turn reaches no one.
fn companion_off_a_removed_provider(bridge: &Arc<CompanionBridge>, home: &std::path::Path) {
    let saved = ProviderStore::load();
    if crate::provider_handoff::companion::primary_gone(home, &saved) {
        crate::provider_handoff::companion::stop_then_move(bridge, saved.primary());
    }
}

/// Drop, now, every free account that is switched off or has lost its key, and every removed saved account, from the list and the
/// catalogue, and tell the listeners (the picker, the runs-on card). Never adds an account: that
/// is the listing's work.
pub fn prune_to_shared() {
    let (kept, off) = crate::wire::free_ai::kept_and_off();
    let saved: BTreeSet<String> = ProviderStore::load().entries.into_iter().map(|e| e.id).collect();
    prune_with(&kept, &off, &saved, crate::wire::free_ai::kept_known());
}

/// `saved`: the ids of the saved providers still there; an account made from one that is not has
/// been removed. `known`: the vault was read into `kept`; if it was not, a tier missing from it
/// is not known to have lost its key, so only a tier switched Off is dropped.
fn prune_with(kept: &BTreeSet<String>, off: &BTreeSet<String>, saved: &BTreeSet<String>, known: bool) {
    let list: Vec<Account> = super::accounts()
        .into_iter()
        .filter(|a| usable(a, kept, off, known) && a.entry.as_ref().is_none_or(|e| saved.contains(e)))
        .collect();
    let mut catalogue = super::catalogue();
    catalogue.accounts.retain(|g| list.iter().any(|a| a.id == g.account));
    super::install(list, catalogue);
}

fn usable(a: &Account, kept: &BTreeSet<String>, off: &BTreeSet<String>, known: bool) -> bool {
    match withdrawn_now(a, kept, off) {
        None => true,
        Some(Withdrawn::NoKey) => !known,
        Some(Withdrawn::Off) => false,
    }
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
    refusal_with(&a.name, withdrawn_now(a, &kept, &off), crate::wire::free_ai::kept_known())
}

/// The same check for an account id that is no longer in the list: a free tier switched Off, or
/// one that lost its key, has left the list, and the person is owed that reason, not "there is no
/// such account". `None` for an id that is not a free tier's, or a tier that is usable now.
pub fn refusal_for_id(id: &str) -> Option<Refusal> {
    let t = FREE_TIERS.iter().find(|t| free_id(t) == id)?;
    let (kept, off) = crate::wire::free_ai::kept_and_off();
    refusal_with(&free_name(t), withdrawn(t, &kept, &off), crate::wire::free_ai::kept_known())
}

/// `known`: the vault was read. A key not found in a vault that was never read is not "removed".
fn refusal_with(name: &str, why: Option<Withdrawn>, known: bool) -> Option<Refusal> {
    Some(match why? {
        Withdrawn::Off => Refusal::new(
            403,
            "tier_off",
            format!("{name} isn't offered any more: it was switched Off in Settings → AI → Free AI accounts. Nothing was sent to it. Switch it On there to use it again."),
        ),
        Withdrawn::NoKey if !known => Refusal::new(
            503,
            "keys_not_known_yet",
            format!("{name} cannot be used just yet: the shell has not finished reading which keys are kept. Nothing was sent to it. Try again in a moment."),
        ),
        Withdrawn::NoKey => Refusal::new(
            503,
            "key_missing",
            format!("{name} isn't offered any more: its key was removed. Nothing was sent to it. Add a key in Settings → AI → Free AI accounts to use it again."),
        ),
    })
}

#[cfg(test)]
pub(super) fn prune_with_for_test(saved: &BTreeSet<String>) {
    let (kept, off) = crate::wire::free_ai::kept_and_off();
    prune_with(&kept, &off, saved, crate::wire::free_ai::kept_known());
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
