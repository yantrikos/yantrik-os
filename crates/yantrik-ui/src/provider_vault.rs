//! Saved AI providers' API keys, kept in the credential vault and not in `providers.yaml`.
//!
//! A saved provider's key goes into the vault under `saved:<entry id>`, through the same worker
//! and the same operations the free tiers' keys use (`wire::free_ai::store::Op`). The file keeps
//! only a reference, `api_key_ref: vault:saved:<entry id>`. Every reader asks this module for the
//! key at the moment it needs it; a shut vault gives a refusal and nothing is sent.
//!
//! An entry carries a key in three ways, and only the second is ever written to the file:
//!
//! - `api_key`: in memory only. A key the person just typed, or one resolved for a reader. It is
//!   never serialised. `ProviderStore::save` moves it into the vault first.
//! - `api_key_ref`: where the vault keeps it.
//! - `unmigrated_key`: a key an older version left in the file as `api_key:`. It is kept as it
//!   is until the vault is open, copied then, and removed from the file by [`migrate`].

use std::sync::{Arc, Mutex, RwLock};

use crate::bridge::CompanionBridge;
use crate::wire::free_ai::store::{Op, Reply};
use crate::wire::settings::{ProviderStore, ProviderStoreEntry};

#[cfg(test)]
mod tests;

/// What a reference starts with: `vault:` and then the vault id.
pub const REF_SCHEME: &str = "vault:";

const VAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The vault id of a saved provider's key.
pub fn vault_id(entry_id: &str) -> String {
    format!("saved:{entry_id}")
}

/// The reference the file keeps for a saved provider's key.
pub fn reference(entry_id: &str) -> String {
    format!("{REF_SCHEME}{}", vault_id(entry_id))
}

/// The vault id a reference names, or `None` when it is not a vault reference.
pub fn id_in(reference: &str) -> Option<&str> {
    reference.strip_prefix(REF_SCHEME).filter(|id| !id.is_empty())
}

/// Why the vault did not do what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultError {
    /// The vault has a passphrase and is shut.
    Locked,
    /// The vault refused, did not answer, or gave back something other than what was stored.
    Failed,
}

/// The vault as this module needs it. The shell's implementation is [`BridgeVault`]; tests use a
/// fake.
pub trait KeyVault: Send + Sync {
    fn store(&self, id: &str, value: &str) -> Result<(), VaultError>;
    /// The value kept under `id`, or `None` when nothing is.
    fn value(&self, id: &str) -> Result<Option<String>, VaultError>;
    fn remove(&self, id: &str) -> Result<(), VaultError>;
}

/// The credential vault, reached through the companion's worker like the free tiers' keys.
pub struct BridgeVault {
    bridge: Arc<CompanionBridge>,
}

impl BridgeVault {
    pub fn new(bridge: Arc<CompanionBridge>) -> Self {
        Self { bridge }
    }

    fn ask(&self, op: Op) -> Result<Reply, VaultError> {
        self.bridge.provider_keys(op, VAULT_TIMEOUT).map_err(|_| VaultError::Failed)
    }
}

impl KeyVault for BridgeVault {
    fn store(&self, id: &str, value: &str) -> Result<(), VaultError> {
        match self.ask(Op::Store { id: id.to_string(), value: value.to_string() })? {
            Reply::Done => Ok(()),
            Reply::Locked => Err(VaultError::Locked),
            _ => Err(VaultError::Failed),
        }
    }

    fn value(&self, id: &str) -> Result<Option<String>, VaultError> {
        match self.ask(Op::Value { id: id.to_string() })? {
            Reply::Value(v) => Ok(v),
            Reply::Locked => Err(VaultError::Locked),
            _ => Err(VaultError::Failed),
        }
    }

    fn remove(&self, id: &str) -> Result<(), VaultError> {
        match self.ask(Op::Remove { id: id.to_string() })? {
            Reply::Done => Ok(()),
            Reply::Locked => Err(VaultError::Locked),
            _ => Err(VaultError::Failed),
        }
    }
}

static INSTALLED: RwLock<Option<Arc<dyn KeyVault>>> = RwLock::new(None);

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_VAULT: std::cell::RefCell<Option<Arc<dyn KeyVault>>> = const { std::cell::RefCell::new(None) };
}

/// Make `vault` the one every reader and writer of a saved key uses. Called once at start.
pub fn install(vault: Arc<dyn KeyVault>) {
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = Some(vault);
}

/// Make the companion's vault the one in use, if none is yet. Every place that wires a screen
/// that saves providers calls this, so the order they are wired in does not matter.
pub fn install_for(bridge: &Arc<CompanionBridge>) {
    let mut slot = INSTALLED.write().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(Arc::new(BridgeVault::new(bridge.clone())));
    }
}

/// The vault in use, or `None` before [`install`].
pub fn current() -> Option<Arc<dyn KeyVault>> {
    #[cfg(test)]
    if let Some(v) = TEST_VAULT.with(|v| v.borrow().clone()) {
        return Some(v);
    }
    INSTALLED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Why a saved provider's key could not be had, in words for the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    Locked,
    /// The vault is open and holds nothing under the reference.
    Missing,
    /// The vault did not answer, or is not there yet.
    Unavailable,
}

impl KeyError {
    /// The sentence for using the provider named `name`.
    pub fn to_use(self, name: &str) -> String {
        match self {
            KeyError::Locked => format!("Unlock the credential vault to use {name}."),
            KeyError::Missing => format!("The key for {name} is no longer in the credential vault. Enter it again."),
            KeyError::Unavailable => format!("The credential vault did not answer, so {name} was not used. Try again."),
        }
    }

    /// The sentence for saving or removing the provider named `name`.
    pub fn to_change(self, name: &str) -> String {
        match self {
            KeyError::Locked => format!("Unlock the credential vault to change {name}."),
            KeyError::Missing | KeyError::Unavailable => {
                format!("The credential vault did not answer, so {name} was not changed. Try again.")
            }
        }
    }
}

impl From<VaultError> for KeyError {
    fn from(e: VaultError) -> Self {
        match e {
            VaultError::Locked => KeyError::Locked,
            VaultError::Failed => KeyError::Unavailable,
        }
    }
}

/// Keep `key` under `id` and read it back: Ok only when the vault gives the same text.
pub fn keep(vault: &dyn KeyVault, id: &str, key: &str) -> Result<(), VaultError> {
    vault.store(id, key)?;
    match vault.value(id)? {
        Some(back) if back == key => Ok(()),
        _ => Err(VaultError::Failed),
    }
}

/// A key as a reader finds it: in memory, or by a vault id, or not at all. Resolved by [`read`].
#[derive(Clone, PartialEq, Eq, Default)]
pub struct KeySource {
    pub plain: Option<String>,
    pub vault_id: Option<String>,
}

// By hand, so a key never reaches a log through `{:?}`.
impl std::fmt::Debug for KeySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeySource")
            .field("plain", &self.plain.as_ref().map(|_| "<redacted>"))
            .field("vault_id", &self.vault_id)
            .finish()
    }
}

/// The key `source` names. A vault id is read from the vault now and never from a copy; the vault
/// being shut or silent is an error, so no caller goes on with an empty key.
pub fn read(source: &KeySource, vault: Option<&dyn KeyVault>) -> Result<Option<String>, KeyError> {
    if let Some(key) = source.plain.as_ref().filter(|k| !k.is_empty()) {
        return Ok(Some(key.clone()));
    }
    let Some(id) = &source.vault_id else { return Ok(None) };
    let vault = vault.ok_or(KeyError::Unavailable)?;
    match vault.value(id)? {
        Some(key) if !key.is_empty() => Ok(Some(key)),
        _ => Err(KeyError::Missing),
    }
}

/// [`read`] against the vault in use.
pub fn read_now(source: &KeySource) -> Result<Option<String>, KeyError> {
    let vault = current();
    read(source, vault.as_deref())
}

impl ProviderStoreEntry {
    /// Whether a key is held for this provider, wherever it is kept.
    pub fn has_key(&self) -> bool {
        let some = |k: &Option<String>| k.as_deref().is_some_and(|k| !k.trim().is_empty());
        some(&self.api_key) || some(&self.unmigrated_key) || self.vault_key_id().is_some()
    }

    /// The vault id this entry's key is kept under, when it is in the vault. The reference is
    /// trusted only when it names this entry's own key: one that names another entry, or a free
    /// tier's, is no key, so it is never read, sent to this entry's address, or removed.
    pub fn vault_key_id(&self) -> Option<String> {
        self.api_key_ref.as_deref().filter(|r| *r == reference(&self.id)).and_then(id_in).map(str::to_string)
    }

    /// Where a reader finds this entry's key: a key in memory first, then the vault, then a key
    /// an older version left in the file.
    pub fn key_source(&self) -> KeySource {
        let held = |k: &Option<String>| k.clone().filter(|k| !k.is_empty());
        if let Some(key) = held(&self.api_key) {
            return KeySource { plain: Some(key), vault_id: None };
        }
        if let Some(id) = self.vault_key_id() {
            return KeySource { plain: None, vault_id: Some(id) };
        }
        KeySource { plain: held(&self.unmigrated_key), vault_id: None }
    }

    /// The key, resolved now. `Ok(None)` for a provider with no key; an error when it has one
    /// that cannot be had, so the caller refuses rather than sending without it.
    pub fn resolve_key(&self) -> Result<Option<String>, KeyError> {
        read_now(&self.key_source())
    }

    /// A copy with the key in `api_key`, for a caller that hands the entry to code that reads it
    /// there. The copy is for the moment and is never saved.
    pub fn with_resolved_key(&self) -> Result<ProviderStoreEntry, KeyError> {
        let mut copy = self.clone();
        copy.api_key = self.resolve_key()?;
        Ok(copy)
    }
}

/// A key that could not be kept: which provider, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct KeepError {
    pub name: String,
    pub why: KeyError,
}

impl std::fmt::Display for KeepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.why.to_change(&self.name))
    }
}

impl ProviderStore {
    /// Move every key held in memory into the vault, verified by reading it back, and leave the
    /// entry with a reference only. Stops at the first key that cannot be kept; the entries
    /// before it are done and the rest are untouched.
    pub fn keep_keys(&mut self, vault: &dyn KeyVault) -> Result<(), KeepError> {
        for e in &mut self.entries {
            let Some(key) = e.api_key.clone() else { continue };
            if key.trim().is_empty() {
                e.api_key = None;
                continue;
            }
            let id = vault_id(&e.id);
            keep(vault, &id, &key).map_err(|why| KeepError { name: e.name.clone(), why: why.into() })?;
            e.api_key_ref = Some(reference(&e.id));
            e.api_key = None;
            e.unmigrated_key = None;
        }
        Ok(())
    }

    /// Whether any entry holds a key in memory that the vault does not have yet.
    pub fn has_unkept_keys(&self) -> bool {
        self.entries.iter().any(|e| e.api_key.is_some())
    }
}

/// What a migration pass did. Counts only.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Keys now in the vault and gone from the file.
    pub moved: usize,
    /// Keys left in the file because the vault is shut.
    pub waiting: usize,
    /// Keys left in the file because the vault or the file refused.
    pub failed: usize,
}

static KEY_WRITES: Mutex<()> = Mutex::new(());

/// Held while a key is kept in the vault and the entry that names it is committed, by a migration
/// pass and by a save of a provider. Without it a pass that read an old key and a save that kept a
/// new one for the same entry could interleave, and the old key would be written over the new.
/// Taken before the providers' lock and never inside it; the vault is asked with this held and
/// the providers' lock not.
pub(crate) fn key_writes() -> std::sync::MutexGuard<'static, ()> {
    KEY_WRITES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Move the keys older versions left in `providers.yaml` into the vault, once the vault is open.
///
/// - Each key is stored and read back; only a key that reads back equal is dropped from the file.
/// - The file is rewritten once, atomically, by the store's own save, and only if the vault took
///   at least one key. A vault that refuses leaves the file exactly as it was.
/// - Idempotent: with nothing left in the file it does nothing.
/// - The vault is asked outside the store's lock; under it only memory and the file are touched.
/// - Not while a provider is being saved, and not beside another pass ([`key_writes`]).
pub fn migrate(vault: &dyn KeyVault, store: &Mutex<ProviderStore>) -> Migration {
    migrate_with(vault, store, &mut |s: &mut ProviderStore| s.save())
}

/// [`migrate`], with the way the file is written given (the tests write to a file of their own).
pub fn migrate_with(
    vault: &dyn KeyVault,
    store: &Mutex<ProviderStore>,
    save: &mut dyn FnMut(&mut ProviderStore) -> Result<(), String>,
) -> Migration {
    let _writing = key_writes();
    let pending: Vec<(String, String)> = match store.lock() {
        Ok(s) => s
            .entries
            .iter()
            .filter_map(|e| e.unmigrated_key.clone().filter(|k| !k.trim().is_empty()).map(|k| (e.id.clone(), k)))
            .collect(),
        Err(_) => return Migration::default(),
    };
    let mut report = Migration::default();
    let mut verified: Vec<(String, String)> = Vec::new();
    for (id, key) in pending {
        match keep(vault, &vault_id(&id), &key) {
            Ok(()) => verified.push((id, key)),
            Err(VaultError::Locked) => report.waiting += 1,
            Err(VaultError::Failed) => report.failed += 1,
        }
    }
    if verified.is_empty() {
        return report;
    }
    let Ok(mut live) = store.lock() else {
        report.failed += verified.len();
        return report;
    };
    let before = live.clone();
    let mut changed = 0;
    for (id, key) in &verified {
        // Only an entry that still holds the key that was copied: one the person changed
        // meanwhile is left for the next pass.
        if let Some(e) = live.entries.iter_mut().find(|e| &e.id == id && e.unmigrated_key.as_deref() == Some(key.as_str())) {
            e.api_key_ref = Some(reference(id));
            e.unmigrated_key = None;
            changed += 1;
        }
    }
    if changed == 0 {
        return report;
    }
    match save(&mut live) {
        Ok(()) => report.moved = changed,
        Err(_) => {
            *live = before;
            report.failed += changed;
        }
    }
    report
}

type Deferred = Box<dyn FnOnce() + Send>;

static DEFERRED: Mutex<Vec<Deferred>> = Mutex::new(Vec::new());

/// Run `work` the next time the vault is opened. For something refused because the vault was
/// shut, such as the companion's reload at start, which happens before the person has signed in.
pub fn after_open(work: impl FnOnce() + Send + 'static) {
    DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(work));
}

/// The vault was just opened (at sign-in, at a screen unlock, or from the unlock card): run what
/// was waiting for it, once.
pub fn opened() {
    let waiting = std::mem::take(&mut *DEFERRED.lock().unwrap_or_else(|e| e.into_inner()));
    for work in waiting {
        work();
    }
}

/// Forget the vault entry of a provider being removed. A shut vault is an error, so the key is
/// not left behind with the provider gone from the list.
pub fn forget(vault: &dyn KeyVault, entry: &ProviderStoreEntry) -> Result<(), KeyError> {
    match entry.vault_key_id() {
        Some(id) => vault.remove(&id).map_err(KeyError::from),
        None => Ok(()),
    }
}

/// How often, and how many times, a migration that left keys behind asks again.
const RETRY_EVERY: std::time::Duration = std::time::Duration::from_secs(60);
const RETRIES: usize = 60;

/// How a pass writes the file.
type SaveFile = Arc<dyn Fn(&mut ProviderStore) -> Result<(), String> + Send + Sync>;

/// Whether a pass left keys in the file.
fn keys_left(report: &Migration) -> bool {
    report.waiting + report.failed > 0
}

/// One pass, logged when it did anything. Counts only. A pass with nothing to do is silent.
fn pass(vault: &dyn KeyVault, store: &Mutex<ProviderStore>, save: &SaveFile, when: &str) -> Migration {
    let report = migrate_with(vault, store, &mut |s: &mut ProviderStore| save(s));
    if report != Migration::default() {
        tracing::info!(
            moved = report.moved,
            waiting_for_the_vault = report.waiting,
            failed = report.failed,
            when,
            "Saved providers' keys moved into the credential vault"
        );
    }
    report
}

/// One more pass when the vault opens. It asks again for the next opening if keys are still left,
/// so an unlock at any time, even after the timer has given up, reaches the vault.
fn migrate_when_opened(vault: Arc<dyn KeyVault>, store: Arc<Mutex<ProviderStore>>, save: SaveFile) {
    after_open(move || {
        if keys_left(&pass(&*vault, &store, &save, "unlock")) {
            migrate_when_opened(vault, store, save);
        }
    });
}

/// The first pass, and the one that waits for the vault to open when it left keys behind,
/// whether the vault was shut or did not answer (the companion's worker may still be starting).
fn begin_migration(vault: &Arc<dyn KeyVault>, store: &Arc<Mutex<ProviderStore>>, save: &SaveFile) -> Migration {
    let report = pass(&**vault, store, save, "start");
    if keys_left(&report) {
        migrate_when_opened(vault.clone(), store.clone(), save.clone());
    }
    report
}

/// [`migrate`] on a thread of its own, at start. The vault may be shut at start (its passphrase
/// is typed on the shell's unlock card) or the worker not yet answering, so a pass that left keys
/// behind asks again every minute for an hour, and once more whenever the vault is opened
/// ([`opened`]). Logs counts only.
pub fn migrate_in_background(bridge: &Arc<CompanionBridge>, store: Arc<Mutex<ProviderStore>>) {
    install_for(bridge);
    let Some(vault) = current() else { return };
    let save: SaveFile = Arc::new(|s| s.save());
    let spawned = std::thread::Builder::new().name("provider-keys-migrate".into()).spawn(move || {
        let mut report = begin_migration(&vault, &store, &save);
        for _ in 0..RETRIES {
            if !keys_left(&report) {
                return;
            }
            std::thread::sleep(RETRY_EVERY);
            report = pass(&*vault, &store, &save, "retry");
        }
    });
    if spawned.is_err() {
        tracing::warn!("no thread to move saved providers' keys into the vault");
    }
}
