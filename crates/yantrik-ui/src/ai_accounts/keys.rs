//! An account's key and address at the moment of use, read from where they are kept. Only two
//! callers: listing an account's models, and the gateway forwarding a request. Nothing returned
//! here is written to a file, a log or a harness; a [`Resolved`] prints without its key.

use super::account::{Account, KeyRef};
use crate::wire::settings::ProviderStoreEntry;

pub struct Resolved {
    pub base_url: String,
    pub key: Option<String>,
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("base_url", &self.base_url)
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Why an account cannot be used now, in words for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The saved provider is gone or holds no key.
    Key,
    /// The vault is locked or did not answer.
    Vault,
    /// The address still has a part to fill in.
    Address,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Missing::Key => write!(f, "key missing"),
            Missing::Vault => write!(f, "the vault is locked"),
            Missing::Address => write!(f, "its address is not complete"),
        }
    }
}

/// The key and the filled address. `vault` reads one value by its key-store id (None when it is
/// not kept or the vault is shut).
pub fn resolve(
    account: &Account,
    saved: &[ProviderStoreEntry],
    vault: &dyn Fn(&str) -> Option<String>,
) -> Result<Resolved, Missing> {
    let (key, fill) = match &account.key {
        KeyRef::None => (None, None),
        KeyRef::Saved { entry } => {
            let key = saved
                .iter()
                .find(|e| &e.id == entry)
                .and_then(|e| e.api_key.clone())
                .filter(|k| !k.trim().is_empty())
                .ok_or(Missing::Key)?;
            (Some(key), None)
        }
        KeyRef::Vault { id, fill } => {
            let key = vault(id).filter(|k| !k.is_empty()).ok_or(Missing::Vault)?;
            let filled = match fill {
                Some(part) => Some(vault(part).filter(|v| !v.is_empty()).ok_or(Missing::Vault)?),
                None => None,
            };
            (Some(key), filled)
        }
    };
    let mut base_url = account.base_url.clone();
    if let Some(value) = fill {
        // Only a plain id may be put into an address.
        if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(Missing::Address);
        }
        base_url = base_url.replace("{account_id}", &value);
    }
    if base_url.contains('{') {
        return Err(Missing::Address);
    }
    Ok(Resolved { base_url, key })
}
