//! The accounts the person added, in one list: the providers saved in Settings → AI & Intelligence
//! (`providers.yaml`, custom endpoints among them) and the free tiers switched on in the "Free AI
//! accounts" card. Pure: what is read from disk and the vault is passed in.
//!
//! An account's id is what goes in front of a model in the gateway's `<account>/<model>`, so it is
//! a word a person can read in a log and type: the saved provider's name as a slug (`ollama-cloud`,
//! `nvidia-nim`), and `free-<tier>` for a free tier. No key is in an [`Account`]: it says where the
//! key is kept ([`KeyRef`]), and only the gateway, at the moment it forwards, reads it.

use std::collections::BTreeSet;

use yantrik_ml::provider::pool::tiers::FreeTier;

use crate::wire::settings::ProviderStoreEntry;

/// Where an account's key is kept. Never the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyRef {
    /// None needed (a local runtime, a free tier open to any address).
    None,
    /// The saved provider's own entry, by its id.
    Saved { entry: String },
    /// The vault, under this key-store id; `fill` is the vault id of a part of the address only the
    /// person knows (Cloudflare's account id), put where the address has `{account_id}`.
    Vault { id: String, fill: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Saved under Settings → AI & Intelligence (a preset or a custom endpoint).
    Saved,
    /// A free tier from the Free AI accounts card.
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub name: String,
    /// The catalogue's id (`openai`, `ollama`, …), or `custom`.
    pub provider_type: String,
    /// The OpenAI-compatible base URL requests go to (`…/v1`). May hold `{account_id}`.
    pub base_url: String,
    pub key: KeyRef,
    /// On this machine or the local network: no private context leaves it.
    pub local: bool,
    pub source: Source,
    /// The provider's free tier may train on what is sent (the free card's label).
    pub trains_on_prompts: bool,
    /// The model the person picked for this provider in Settings, if any.
    pub saved_model: String,
}

/// The gateway's own address. A saved provider pointing here is the gateway itself, not an
/// account: listing it would loop.
pub fn is_gateway(base_url: &str) -> bool {
    let host = crate::runs_on::identity::host(base_url).to_ascii_lowercase();
    let port = yantrik_gateway::PORT;
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// A name as an account id: lowercase letters, digits and single dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Every account, saved providers first in the order they were saved, then the free tiers that are
/// switched on and have what they need (`kept`: the vault ids that hold a value; `off`: the tiers
/// switched off on the card).
pub fn accounts(saved: &[ProviderStoreEntry], free: &[FreeTier], kept: &BTreeSet<String>, off: &BTreeSet<String>) -> Vec<Account> {
    let mut out: Vec<Account> = Vec::new();
    let mut taken = BTreeSet::new();
    let mut unique = |wanted: String| {
        let base = if wanted.is_empty() { "account".to_string() } else { wanted };
        let mut id = base.clone();
        let mut n = 2;
        while !taken.insert(id.clone()) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    };
    for e in saved {
        if is_gateway(&e.base_url) {
            continue;
        }
        let base_url = crate::wire::provider_models::openai_base(&e.base_url);
        let has_key = e.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
        let slugged = slug(&e.name);
        out.push(Account {
            id: unique(if slugged.is_empty() { slug(&e.provider_type) } else { slugged }),
            name: e.name.clone(),
            provider_type: e.provider_type.clone(),
            local: crate::runs_on::identity::is_local(&base_url),
            base_url,
            key: if has_key && e.auth_type != "none" { KeyRef::Saved { entry: e.id.clone() } } else { KeyRef::None },
            source: Source::Saved,
            trains_on_prompts: false,
            saved_model: e.model.clone(),
        });
    }
    for t in free {
        if off.contains(t.id) {
            continue;
        }
        let signup = yantrik_ml::provider::pool::signup::signup(t.id);
        let values: Vec<&str> = signup.map(|s| s.values.iter().map(|v| v.id).collect()).unwrap_or_default();
        if t.needs_key && (values.is_empty() || !values.iter().all(|v| kept.contains(*v))) {
            continue;
        }
        // The provider's own value is the last; any before it fill the address.
        let key = match values.split_last() {
            Some((key, rest)) => KeyRef::Vault { id: key.to_string(), fill: rest.first().map(|v| v.to_string()) },
            None => KeyRef::None,
        };
        out.push(Account {
            id: unique(format!("free-{}", slug(t.id))),
            name: format!("{} (free)", t.name),
            provider_type: t.id.to_string(),
            base_url: t.base_url.to_string(),
            key,
            local: false,
            source: Source::Free,
            trains_on_prompts: t.trains_on_prompts,
            saved_model: String::new(),
        });
    }
    out
}
