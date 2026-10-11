//! The pool's keys: one `provider=key` line each, in a file only its owner can read
//! (`deploy/free-pool/import-keys.py` writes it). Read here, held in memory, never logged: the
//! `Debug` of `Keys` prints which providers have one, not what it is.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Default)]
pub struct Keys(BTreeMap<String, String>);

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

impl Keys {
    /// The keys in `path`. A missing or unreadable file is no keys, not an error: the providers
    /// that need one then say "Needs a key".
    pub fn load(path: &Path) -> Keys {
        std::fs::read_to_string(path).map(|t| Keys::parse(&t)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Keys {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                if !k.is_empty() && !v.is_empty() {
                    map.insert(k.to_string(), v.to_string());
                }
            }
        }
        Keys(map)
    }

    /// Keys from wherever they are kept (the vault, `provider_keys::load_all`), by provider id.
    pub fn from_map(map: BTreeMap<String, String>) -> Keys {
        Keys(map.into_iter().filter(|(k, v)| !k.is_empty() && !v.is_empty()).collect())
    }

    pub fn get(&self, provider: &str) -> Option<&str> {
        self.0.get(provider).map(String::as_str)
    }

    /// The providers that have a key (Cloudflare's also needs its account id).
    pub fn keyed(&self) -> Vec<String> {
        self.0
            .keys()
            .filter(|k| k.as_str() != "cloudflare_account")
            .filter(|k| k.as_str() != "cloudflare" || self.0.contains_key("cloudflare_account"))
            .cloned()
            .collect()
    }

    /// The address to call for a provider: its own, with the account id filled in for
    /// Cloudflare.
    pub fn base_url(&self, tier: &super::tiers::FreeTier) -> String {
        match self.get("cloudflare_account") {
            Some(account) if tier.base_url.contains("{account_id}") => tier.base_url.replace("{account_id}", account),
            _ => tier.base_url.to_string(),
        }
    }
}
