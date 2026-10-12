//! Where the person's private context goes, per account: Share, Private lane or Off
//! (design/onboarding-one-model-one-memory-2026-10-10.md, "Private context is a preference").
//!
//! Every mind keeps YantrikDB memory, so every mind's calls carry what it knows about the person.
//! The gateway follows the person's choice for the account a call names (crates/yantrik-gateway,
//! `policy`): Share sends it unchanged, Private lane sends it to the local model kept as the lane
//! ([`Consent::lane`]) instead, Off refuses it with the fix named. An account on this machine or
//! the local network is always Share: nothing leaves. An account with no choice of its own takes
//! [`Consent::default`]: Share once the person agreed to sharing at onboarding, else Off, so adding
//! an account never sends anything private anywhere by itself.
//!
//! `~/.config/yantrik/ai-accounts.json`, beside the other preference files. No key in it. The
//! file's earlier shape (an Allow list, `private_context`) is read and kept as written: an account
//! on it is Share, any other was Stop and is Off.
//!
//! Harnesses run as the person and can edit that file (security review of #675, finding 4), so
//! the shell holds the person's choices in memory, as `memory_grants` holds grants: a widening
//! (toward Share) in the file that this shell did not save itself is ignored and logged once; a
//! narrowing from the file is always honoured. The same limit: the first read after the shell
//! starts has nothing to compare with, so it is accepted as it is.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use yantrik_gateway::Sharing;

use super::account::Account;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Consent {
    /// The earlier Allow list, by account id. Never written to any more and never deleted:
    /// [`load`] reads each as Share.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub private_context: BTreeSet<String>,
    /// The person's choice, by account id.
    pub sharing: BTreeMap<String, Sharing>,
    /// For an account with no choice of its own; `None` is Off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Sharing>,
    /// The private lane, `<account>/<model>` on a local account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
}

impl Consent {
    /// What the person chose for `account`, as the gateway applies it. A local account is Share. A
    /// free tier that may train on what it is sent is never Share (the free card promises nothing
    /// of the person's goes there): it keeps personal things on this computer instead.
    pub fn sharing_for(&self, account: &Account) -> Sharing {
        if account.local {
            return Sharing::Share;
        }
        let chosen = self.sharing.get(&account.id).copied().or(self.default).unwrap_or(Sharing::Off);
        if account.trains_on_prompts && chosen == Sharing::Share {
            return Sharing::Lane;
        }
        chosen
    }
}

pub fn path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config/yantrik/ai-accounts.json")
}

/// The file as it is, with the earlier Allow list read as Share (where no newer choice was made).
fn read(path: &Path) -> Consent {
    let mut c: Consent = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    for id in &c.private_context {
        c.sharing.entry(id.clone()).or_insert(Sharing::Share);
    }
    c
}

/// How far a choice lets personal things go: Off, then the private lane, then Share.
fn reach(s: Option<Sharing>) -> u8 {
    match s {
        None | Some(Sharing::Off) => 0,
        Some(Sharing::Lane) => 1,
        Some(Sharing::Share) => 2,
    }
}

/// Take out of `on_disk` whatever lets an account's personal things go further than `baseline`
/// did. Answers what is left, and the accounts (and `default`) the file tried to widen.
fn settle(baseline: &Consent, mut on_disk: Consent) -> (Consent, Vec<String>) {
    let mut widened = Vec::new();
    let ids: BTreeSet<String> = baseline.sharing.keys().chain(on_disk.sharing.keys()).cloned().collect();
    for id in ids {
        let was = baseline.sharing.get(&id).copied().or(baseline.default);
        let now = on_disk.sharing.get(&id).copied().or(on_disk.default);
        if reach(now) > reach(was) {
            on_disk.sharing.insert(id.clone(), was.unwrap_or(Sharing::Off));
            widened.push(id);
        }
    }
    if reach(on_disk.default) > reach(baseline.default) {
        on_disk.default = baseline.default;
        widened.push("default".into());
    }
    (on_disk, widened)
}

/// The consent file as this shell has seen it: what it last saved or accepted, and what a
/// widening was last logged for, so one edit is one warning.
#[derive(Default)]
struct Keeper {
    baseline: Option<Consent>,
    warned: Vec<String>,
}

/// One keeper per file (the person's, and each test's own).
static KEEPERS: Mutex<BTreeMap<PathBuf, Keeper>> = Mutex::new(BTreeMap::new());

impl Keeper {
    fn load_from(&mut self, path: &Path) -> Consent {
        let on_disk = read(path);
        let (settled, widened) = match &self.baseline {
            Some(baseline) => settle(baseline, on_disk),
            None => (on_disk, Vec::new()),
        };
        if !widened.is_empty() && widened != self.warned {
            tracing::warn!(
                path = %path.display(),
                accounts = ?widened,
                "ai-accounts.json shares more than this desktop saved; the widening is ignored (#675 finding 4)"
            );
        }
        self.warned = widened;
        self.baseline = Some(settled.clone());
        settled
    }
}

/// The person's choices: the file, with any widening the shell did not make taken out.
pub fn load(path: &Path) -> Consent {
    KEEPERS.lock().unwrap_or_else(|e| e.into_inner()).entry(path.to_path_buf()).or_default().load_from(path)
}

/// The person's choices as this shell holds them, without reading the file again (read once if
/// it never was). What the Mind is sent (`mirror`): never a value a harness could write since.
pub fn held(path: &Path) -> Consent {
    let mut keepers = KEEPERS.lock().unwrap_or_else(|e| e.into_inner());
    let keeper = keepers.entry(path.to_path_buf()).or_default();
    match &keeper.baseline {
        Some(c) => c.clone(),
        None => keeper.load_from(path),
    }
}

/// Write `consent` whole; what is saved is what the next read is held to.
pub fn save(path: &Path, consent: &Consent) -> Result<(), String> {
    let text = serde_json::to_string_pretty(consent).map_err(|e| e.to_string())?;
    let mut keepers = KEEPERS.lock().unwrap_or_else(|e| e.into_inner());
    crate::private_file::write(path, format!("{text}\n").as_bytes(), crate::private_file::Publish::Replace, || Ok(()))?;
    let keeper = keepers.entry(path.to_path_buf()).or_default();
    keeper.baseline = Some(consent.clone());
    keeper.warned.clear();
    Ok(())
}

fn change(path: &Path, f: impl FnOnce(&mut Consent)) -> Result<(), String> {
    let mut c = load(path);
    f(&mut c);
    save(path, &c)
}

/// Settings → AI accounts: the person's choice for one account.
pub fn set(path: &Path, account: &str, sharing: Sharing) -> Result<(), String> {
    change(path, |c| {
        c.sharing.insert(account.to_string(), sharing);
    })?;
    tracing::info!(account, sharing = sharing.as_str(), "private context for an account");
    Ok(())
}

/// Onboarding's "Your minds will send what they remember about you … to <provider>": checked,
/// every account with no choice of its own is Share; unchecked, Off. Choices already made stay.
pub fn agree_at_onboarding(path: &Path, share: bool) -> Result<(), String> {
    change(path, |c| c.default = Some(if share { Sharing::Share } else { Sharing::Off }))
}

/// Keep `model` (`<account>/<model>`, a local account's) as the private lane, or none.
pub fn set_lane(path: &Path, model: Option<&str>) -> Result<(), String> {
    change(path, |c| c.lane = model.map(str::to_string).filter(|m| !m.trim().is_empty()))
}

/// The private lane, when the model it names is on an account that is local.
pub fn lane(consent: &Consent, accounts: &[Account]) -> Option<String> {
    let lane = consent.lane.as_deref()?;
    let (account, _) = lane.split_once('/')?;
    accounts.iter().any(|a| a.id == account && a.local).then(|| lane.to_string())
}
