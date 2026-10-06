//! Which accounts may be sent the person's private context: the per-account switch the Mind keeps
//! as its provider's `private_context` flag (yantrik-mind E.PROV1), kept here for every mind.
//!
//! A mind marked as sending private context (its memory, what the desktop shows it) may use an
//! account only when the person switched this on for that account; the gateway enforces it on
//! every request (crates/yantrik-gateway, `policy`). An account on this machine or the local
//! network needs no switch: nothing leaves. Off by default, for every account, so adding an
//! account never sends anything private anywhere by itself.
//!
//! `~/.config/yantrik/ai-accounts.json`, beside the other preference files. No key in it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Consent {
    /// Accounts the person allowed to receive private context, by account id.
    pub private_context: BTreeSet<String>,
}

pub fn path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config/yantrik/ai-accounts.json")
}

pub fn load(path: &Path) -> Consent {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, consent: &Consent) -> Result<(), String> {
    let text = serde_json::to_string_pretty(consent).map_err(|e| e.to_string())?;
    crate::private_file::write(path, format!("{text}\n").as_bytes(), crate::private_file::Publish::Replace, || Ok(()))
}

/// Switch one account, and say what it now is.
pub fn toggle(path: &Path, account: &str) -> Result<bool, String> {
    let mut c = load(path);
    let now = if c.private_context.remove(account) {
        false
    } else {
        c.private_context.insert(account.to_string());
        true
    };
    save(path, &c)?;
    tracing::info!(account, allowed = now, "private context for an account");
    Ok(now)
}
