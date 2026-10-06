//! AI accounts: the one list of the accounts the person added, and each one's models with what
//! each can do (#673). Settings → AI & Intelligence, the picker and the local model gateway all
//! read it here; none keeps a list of its own.
//!
//! - `account`: the accounts, from the saved providers and the free tiers switched on (pure).
//! - `models`: each account's models and capabilities (pure; capabilities from yantrik_ml).
//! - `keys`: an account's key and address at the moment of use (the vault, or the saved entry).
//! - `store`: the catalogue in memory and its cache file, which holds no key.
//! - `consent`: which accounts may be sent the person's private context (off until switched on).
//!
//! Keys stay where they are kept — the vault for the free tiers, the saved provider's entry for
//! the rest — and are read only to list an account's models and by the gateway as it forwards.
//! No file this writes, and nothing a harness can ask for, carries one.

pub mod account;
pub mod consent;
pub mod keys;
pub mod models;
pub mod store;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use account::Account;
use models::AccountModels;
use store::Catalogue;
use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use crate::bridge::CompanionBridge;
use crate::wire::free_ai::store::{Op, Reply};
use crate::wire::settings::ProviderStore;

static CATALOGUE: RwLock<Option<Catalogue>> = RwLock::new(None);
static ACCOUNTS: RwLock<Vec<Account>> = RwLock::new(Vec::new());
static LISTENERS: Mutex<Vec<Box<dyn Fn() + Send + Sync>>> = Mutex::new(Vec::new());
/// One refresh at a time; a second asked for while one runs is folded into it.
static REFRESHING: Mutex<bool> = Mutex::new(false);

const VAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The catalogue as last built (or as cached from the last run, until then).
pub fn catalogue() -> Catalogue {
    if let Some(c) = CATALOGUE.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return c.clone();
    }
    let cached = store::load(&store::path());
    *CATALOGUE.write().unwrap_or_else(|e| e.into_inner()) = Some(cached.clone());
    cached
}

/// The accounts as last read. No keys.
pub fn accounts() -> Vec<Account> {
    ACCOUNTS.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Called after every rebuild, on the refresh thread.
pub fn on_change(f: impl Fn() + Send + Sync + 'static) {
    LISTENERS.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(f));
}

/// One value from the vault, for the gateway and the listing only. On the UI's own bridge, never
/// on a socket.
pub fn vault_value(bridge: &CompanionBridge, id: &str) -> Option<String> {
    match bridge.provider_keys(Op::Value { id: id.to_string() }, VAULT_TIMEOUT) {
        Ok(Reply::Value(v)) => v,
        _ => None,
    }
}

/// Read the accounts again and ask each for its models, off the UI thread.
pub fn refresh(bridge: Arc<CompanionBridge>) {
    {
        let mut busy = REFRESHING.lock().unwrap_or_else(|e| e.into_inner());
        if *busy {
            return;
        }
        *busy = true;
    }
    let _ = std::thread::Builder::new().name("ai-accounts".into()).spawn(move || {
        let saved = ProviderStore::load().entries;
        // Which free tiers have their values kept: asked of the vault now, since the card's own
        // read may not have come back yet at start.
        let (mut kept, off) = crate::wire::free_ai::kept_and_off();
        if let Ok(Reply::Tails(tails)) = bridge.provider_keys(Op::Tails, VAULT_TIMEOUT) {
            kept = tails.into_keys().collect();
        }
        let list = account::accounts(&saved, FREE_TIERS, &kept, &off);
        *ACCOUNTS.write().unwrap_or_else(|e| e.into_inner()) = list.clone();
        let vault = |id: &str| vault_value(&bridge, id);
        let built: Vec<AccountModels> = list.iter().map(|a| list_one(a, &saved, &vault)).collect();
        let catalogue = Catalogue { accounts: built, at: chrono::Utc::now().timestamp() };
        if let Err(e) = store::save(&store::path(), &catalogue) {
            tracing::warn!(error = %e, "the model catalogue was not cached");
        }
        tracing::info!(accounts = catalogue.accounts.len(), models = catalogue.models().count(), "AI accounts listed");
        *CATALOGUE.write().unwrap_or_else(|e| e.into_inner()) = Some(catalogue);
        *REFRESHING.lock().unwrap_or_else(|e| e.into_inner()) = false;
        for f in LISTENERS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            f();
        }
    });
}

fn list_one(a: &Account, saved: &[crate::wire::settings::ProviderStoreEntry], vault: &dyn Fn(&str) -> Option<String>) -> AccountModels {
    if models::needs_no_listing(a, FREE_TIERS) {
        return models::build(a, FREE_TIERS, None);
    }
    match keys::resolve(a, saved, vault) {
        Ok(r) => {
            let auth = if r.key.is_some() { "bearer" } else { "none" };
            let listed = crate::wire::provider_models::list_models(&r.base_url, r.key.as_deref(), auth);
            models::build(a, FREE_TIERS, Some(listed))
        }
        Err(why) => {
            let mut out = models::build(a, FREE_TIERS, None);
            out.listing = models::Listing::Failed(why.to_string());
            out
        }
    }
}
