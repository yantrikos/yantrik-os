//! Saved providers' keys and the credential vault. Every key here is obviously fake.

use std::collections::HashMap;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::ai_accounts::account::{accounts, KeyRef};
use crate::ai_accounts::keys::{resolve, Missing};
use crate::wire::provider_panel::{save_form_with, Form};
use crate::wire::settings::remove_provider_with;

mod opening;
mod sources;

const KEY: &str = "fake-key-not-a-secret-0001";
const OTHER: &str = "fake-key-not-a-secret-0002";

/// A vault in memory that can be shut, can refuse to write, and can hand back the wrong text.
#[derive(Default)]
struct Fake {
    held: Mutex<HashMap<String, String>>,
    locked: Mutex<bool>,
    refuse_writes: Mutex<bool>,
    garble: Mutex<bool>,
    writes: AtomicUsize,
    /// Run in the middle of a store, as something else happening while the vault answers.
    during_store: Mutex<Option<Box<dyn Fn() + Send>>>,
    /// Run once, ahead of the next store's write, with the vault's own locks free.
    before_write: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Does not answer reads, as a worker still starting does not.
    silent: Mutex<bool>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn shut(&self, on: bool) {
        *self.locked.lock().unwrap() = on;
    }
    fn has(&self, id: &str) -> bool {
        self.held.lock().unwrap().contains_key(id)
    }
    fn get(&self, id: &str) -> Option<String> {
        self.held.lock().unwrap().get(id).cloned()
    }
}

impl KeyVault for Fake {
    fn store(&self, id: &str, value: &str) -> Result<(), VaultError> {
        if *self.locked.lock().unwrap() {
            return Err(VaultError::Locked);
        }
        if *self.refuse_writes.lock().unwrap() {
            return Err(VaultError::Failed);
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        let first = self.before_write.lock().unwrap().take();
        if let Some(hook) = first {
            hook();
        }
        if let Some(hook) = self.during_store.lock().unwrap().as_ref() {
            hook();
        }
        let kept = if *self.garble.lock().unwrap() { format!("{value}-changed") } else { value.to_string() };
        self.held.lock().unwrap().insert(id.to_string(), kept);
        Ok(())
    }
    fn value(&self, id: &str) -> Result<Option<String>, VaultError> {
        if *self.locked.lock().unwrap() {
            return Err(VaultError::Locked);
        }
        if *self.silent.lock().unwrap() {
            return Err(VaultError::Failed);
        }
        Ok(self.held.lock().unwrap().get(id).cloned())
    }
    fn remove(&self, id: &str) -> Result<(), VaultError> {
        if *self.locked.lock().unwrap() {
            return Err(VaultError::Locked);
        }
        self.held.lock().unwrap().remove(id);
        Ok(())
    }
}

/// Run with `vault` as the one in use on this thread.
struct Using;
impl Using {
    fn vault(vault: &Arc<Fake>) -> Using {
        let v: Arc<dyn KeyVault> = vault.clone();
        TEST_VAULT.with(|slot| *slot.borrow_mut() = Some(v));
        Using
    }
}
impl Drop for Using {
    fn drop(&mut self) {
        TEST_VAULT.with(|slot| *slot.borrow_mut() = None);
    }
}

fn entry(id: &str, name: &str) -> ProviderStoreEntry {
    ProviderStoreEntry {
        id: id.into(),
        name: name.into(),
        provider_type: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: None,
        api_key_ref: None,
        unmigrated_key: None,
        auth_type: "bearer".into(),
        is_primary: false,
        is_fallback: false,
        model: "m".into(),
    }
}

/// A file of its own in a directory of its own, at mode 600 like the real one.
fn file(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yantrik-provider-vault-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("providers.yaml");
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();
    std::io::Write::write_all(&mut f, text.as_bytes()).unwrap();
    path
}

fn legacy(ids: &[(&str, &str)]) -> String {
    let mut out = String::from("entries:\n");
    for (id, key) in ids {
        out.push_str(&format!(
            "- id: {id}\n  name: {id} name\n  provider_type: openrouter\n  base_url: https://openrouter.ai/api/v1\n  api_key: {key}\n  auth_type: bearer\n  is_primary: false\n  is_fallback: false\n  model: m\n"
        ));
    }
    out
}

fn load(path: &PathBuf) -> ProviderStore {
    serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn shared(path: &PathBuf) -> Mutex<ProviderStore> {
    Mutex::new(load(path))
}

fn p(path: &PathBuf) -> String {
    path.to_str().unwrap().to_string()
}

fn run_migration(vault: &Fake, store: &Mutex<ProviderStore>, path: &PathBuf) -> Migration {
    let at = p(path);
    migrate_with(vault, store, &mut |s| s.save_at(&at))
}

// ── Store ──

#[test]
fn saving_puts_the_key_in_the_vault_and_the_file_holds_a_reference_and_no_key() {
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    let path = file("save", "entries: []\n");
    let mut store = ProviderStore::default();
    let mut e = entry("openrouter-1", "OpenRouter");
    e.api_key = Some(KEY.into());
    store.entries.push(e);
    store.save_at(&p(&path)).expect("saved");

    assert_eq!(vault.get("saved:openrouter-1").as_deref(), Some(KEY), "the key is in the vault");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(KEY), "the key is not in the file:\n{text}");
    assert!(!text.contains("api_key:"), "no api_key line is written:\n{text}");
    assert!(text.contains("api_key_ref: vault:saved:openrouter-1"), "{text}");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(store.entries[0].api_key.is_none(), "the plaintext is not kept in memory either");
}

#[test]
fn replacing_a_key_replaces_the_vault_entry() {
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    let path = file("replace", "entries: []\n");
    let providers = Mutex::new(ProviderStore::default());
    let providers = Arc::new(providers);
    let at = p(&path);
    let form = |key: Option<&str>| Form {
        name: "OpenRouter".into(),
        provider_type: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: key.map(str::to_string),
        auth_type: "bearer".into(),
        model: "m".into(),
    };
    let saved = save_form_with(&providers, "", form(Some(KEY)), Some(vault.clone()), &mut |s| s.save_at(&at)).unwrap();
    let again = save_form_with(&providers, &saved.id, form(Some(OTHER)), Some(vault.clone()), &mut |s| s.save_at(&at)).unwrap();
    assert_eq!(vault.get(&vault_id(&again.id)).as_deref(), Some(OTHER));
    assert!(!std::fs::read_to_string(&path).unwrap().contains(OTHER));
    // Left empty on an edit, the saved key stays.
    save_form_with(&providers, &saved.id, form(None), Some(vault.clone()), &mut |s| s.save_at(&at)).unwrap();
    assert_eq!(vault.get(&vault_id(&again.id)).as_deref(), Some(OTHER));
}

#[test]
fn a_shut_vault_refuses_the_save_and_the_file_and_the_list_stay_as_they_were() {
    let vault = Fake::new();
    vault.shut(true);
    let path = file("shut-save", &legacy(&[("a", "fake-key-a")]));
    let before = std::fs::read_to_string(&path).unwrap();
    let providers = Arc::new(shared(&path));
    let at = p(&path);
    let form = Form {
        name: "Second".into(),
        provider_type: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: Some(KEY.into()),
        auth_type: "bearer".into(),
        model: "m".into(),
    };
    let said = save_form_with(&providers, "", form, Some(vault.clone()), &mut |s| s.save_at(&at)).unwrap_err();
    assert_eq!(said, "Unlock the credential vault to change Second.");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "the file is untouched");
    assert_eq!(providers.lock().unwrap().entries.len(), 1, "the list is as it was");
    assert!(!said.contains(KEY));
}

#[test]
fn a_save_while_the_vault_is_shut_keeps_a_key_not_yet_moved() {
    let vault = Fake::new();
    vault.shut(true);
    let _using = Using::vault(&vault);
    let path = file("keep-legacy", &legacy(&[("a", "fake-key-a")]));
    let mut store = load(&path);
    store.entries[0].is_primary = true;
    store.save_at(&p(&path)).expect("a change that needs no key still saves");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("api_key: fake-key-a"), "the key is not lost while the vault is shut:\n{text}");
    assert!(text.contains("is_primary: true"));
}

// ── Read ──

#[test]
fn every_reader_resolves_the_reference_through_the_vault() {
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    vault.store("saved:p1", KEY).unwrap();
    let mut e = entry("p1", "Mine");
    e.api_key_ref = Some(reference("p1"));

    // Test, list models, the model check: the entry's own resolution.
    assert_eq!(e.resolve_key(), Ok(Some(KEY.to_string())));
    // The companion reload and the harness hand-off: a copy that holds it, for the moment.
    assert_eq!(e.with_resolved_key().unwrap().api_key.as_deref(), Some(KEY));
    assert!(e.api_key.is_none(), "the entry itself still holds none");
    // The card that checks the companion's endpoint.
    let source = e.key_source();
    assert_eq!((source.plain.as_deref(), source.vault_id.as_deref()), (None, Some("saved:p1")));
    assert_eq!(read_now(&source), Ok(Some(KEY.to_string())));
    // AI accounts and the gateway: the account reads it the way a free tier's key is read.
    let list = accounts(std::slice::from_ref(&e), &[], &Default::default(), &Default::default());
    assert_eq!(list[0].key, KeyRef::Vault { id: "saved:p1".into(), fill: None });
    let read = |id: &str| vault.value(id).ok().flatten();
    let resolved = resolve(&list[0], std::slice::from_ref(&e), &read).expect("resolved");
    assert_eq!(resolved.key.as_deref(), Some(KEY));
    // And a key an older version left in the file is still read, until it is moved.
    let mut old = entry("p2", "Old");
    old.unmigrated_key = Some(OTHER.into());
    assert_eq!(old.resolve_key(), Ok(Some(OTHER.to_string())));
    let list = accounts(std::slice::from_ref(&old), &[], &Default::default(), &Default::default());
    assert_eq!(resolve(&list[0], std::slice::from_ref(&old), &read).unwrap().key.as_deref(), Some(OTHER));
    // No key at all is not an error.
    assert_eq!(entry("p3", "None").resolve_key(), Ok(None));
}

#[test]
fn a_shut_vault_gives_the_refusal_and_no_reader_goes_on_without_a_key() {
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    vault.store("saved:p1", KEY).unwrap();
    vault.shut(true);
    let mut e = entry("p1", "Mine");
    e.api_key_ref = Some(reference("p1"));

    let why = e.resolve_key().unwrap_err();
    assert_eq!(why, KeyError::Locked);
    assert_eq!(why.to_use(&e.name), "Unlock the credential vault to use Mine.");
    assert_eq!(e.with_resolved_key().unwrap_err(), KeyError::Locked, "the hand-off and the reload get no entry");
    assert_eq!(read_now(&e.key_source()), Err(KeyError::Locked));
    // AI accounts and the gateway: refused as a free tier is when the vault is shut.
    let list = accounts(std::slice::from_ref(&e), &[], &Default::default(), &Default::default());
    let fetch = |id: &str| vault.value(id).ok().flatten();
    assert_eq!(resolve(&list[0], std::slice::from_ref(&e), &fetch).unwrap_err(), Missing::Vault);
    // No vault at all (before it is installed) is a refusal too, never an empty key.
    assert_eq!(read(&e.key_source(), None), Err(KeyError::Unavailable));
    // Open again and empty: the key is missing, which is said differently.
    vault.shut(false);
    vault.remove("saved:p1").unwrap();
    assert_eq!(e.resolve_key(), Err(KeyError::Missing));
}

// ── Migrate ──

#[test]
fn migration_moves_plaintext_into_the_vault_and_rewrites_the_file_without_it() {
    let vault = Fake::new();
    let path = file("migrate", &legacy(&[("a", KEY), ("b", OTHER)]));
    let store = shared(&path);
    let report = run_migration(&vault, &store, &path);
    assert_eq!(report, Migration { moved: 2, waiting: 0, failed: 0 });
    assert_eq!(vault.get("saved:a").as_deref(), Some(KEY));
    assert_eq!(vault.get("saved:b").as_deref(), Some(OTHER));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(KEY) && !text.contains(OTHER), "no key is left in the file:\n{text}");
    assert!(!text.contains("api_key:"), "{text}");
    assert!(text.contains("api_key_ref: vault:saved:a") && text.contains("api_key_ref: vault:saved:b"));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the mode is kept");
    // The list in memory is the file's.
    assert!(store.lock().unwrap().entries.iter().all(|e| e.unmigrated_key.is_none() && e.api_key_ref.is_some()));
}

#[test]
fn a_shut_vault_leaves_the_plaintext_in_place_to_be_moved_later() {
    let vault = Fake::new();
    vault.shut(true);
    let path = file("migrate-shut", &legacy(&[("a", KEY)]));
    let before = std::fs::read_to_string(&path).unwrap();
    let store = shared(&path);
    assert_eq!(run_migration(&vault, &store, &path), Migration { moved: 0, waiting: 1, failed: 0 });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    vault.shut(false);
    assert_eq!(run_migration(&vault, &store, &path).moved, 1, "moved when it opens");
    assert!(!std::fs::read_to_string(&path).unwrap().contains(KEY));
}

#[test]
fn a_vault_write_that_fails_leaves_the_file_untouched() {
    let vault = Fake::new();
    *vault.refuse_writes.lock().unwrap() = true;
    let path = file("migrate-fail", &legacy(&[("a", KEY)]));
    let before = std::fs::read_to_string(&path).unwrap();
    let store = shared(&path);
    assert_eq!(run_migration(&vault, &store, &path), Migration { moved: 0, waiting: 0, failed: 1 });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "the key is still in the file");
    assert_eq!(store.lock().unwrap().entries[0].unmigrated_key.as_deref(), Some(KEY), "and in memory");
    assert!(store.lock().unwrap().entries[0].api_key_ref.is_none());
}

#[test]
fn a_key_that_does_not_read_back_the_same_is_not_dropped_from_the_file() {
    let vault = Fake::new();
    *vault.garble.lock().unwrap() = true;
    let path = file("migrate-garble", &legacy(&[("a", KEY)]));
    let before = std::fs::read_to_string(&path).unwrap();
    let store = shared(&path);
    assert_eq!(run_migration(&vault, &store, &path), Migration { moved: 0, waiting: 0, failed: 1 });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "only a verified key leaves the file");
}

#[test]
fn a_second_migration_does_nothing() {
    let vault = Fake::new();
    let path = file("migrate-twice", &legacy(&[("a", KEY)]));
    let store = shared(&path);
    assert_eq!(run_migration(&vault, &store, &path).moved, 1);
    let (file_after, writes_after) = (std::fs::read_to_string(&path).unwrap(), vault.writes.load(Ordering::SeqCst));
    assert_eq!(run_migration(&vault, &store, &path), Migration::default());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), file_after);
    assert_eq!(vault.writes.load(Ordering::SeqCst), writes_after, "the vault is not written again");
    // A fresh start reading the rewritten file finds nothing to move either.
    let restarted = shared(&path);
    assert_eq!(run_migration(&vault, &restarted, &path), Migration::default());
}

#[test]
fn migration_never_prints_a_key() {
    let mut e = entry("a", "A");
    e.unmigrated_key = Some(KEY.into());
    e.api_key = Some(OTHER.into());
    let shown = format!("{e:?} {:?} {:?}", e.key_source(), Migration { moved: 1, waiting: 0, failed: 0 });
    assert!(!shown.contains(KEY) && !shown.contains(OTHER), "{shown}");
}

// ── Remove ──

#[test]
fn removing_a_provider_removes_its_vault_entry() {
    let vault = Fake::new();
    let path = file("remove", "entries: []\n");
    let mut store = ProviderStore::default();
    for (id, key) in [("a", KEY), ("b", OTHER)] {
        let mut e = entry(id, id);
        e.api_key = Some(key.into());
        store.entries.push(e);
    }
    store.keep_keys(&*vault).unwrap();
    let at = p(&path);
    store.save_at(&at).unwrap();
    let providers = Arc::new(Mutex::new(store));
    remove_provider_with(&providers, "a", Some(vault.clone()), &mut |s| s.save_at(&at)).expect("removed");
    assert!(!vault.has("saved:a"), "its key is gone from the vault");
    assert!(vault.has("saved:b"), "another provider's key is not touched");
    assert_eq!(providers.lock().unwrap().entries.len(), 1);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("saved:a"));
}

#[test]
fn a_shut_vault_does_not_let_a_key_be_left_behind() {
    let vault = Fake::new();
    let path = file("remove-shut", "entries: []\n");
    let mut store = ProviderStore::default();
    let mut e = entry("a", "Mine");
    e.api_key = Some(KEY.into());
    store.entries.push(e);
    store.keep_keys(&*vault).unwrap();
    let at = p(&path);
    store.save_at(&at).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let providers = Arc::new(Mutex::new(store));
    vault.shut(true);
    let said = remove_provider_with(&providers, "a", Some(vault.clone()), &mut |s| s.save_at(&at)).unwrap_err();
    assert_eq!(said, "Mine was not removed: the credential vault is locked. Unlock it, then remove it again.");
    assert_eq!(providers.lock().unwrap().entries.len(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    assert!(vault.has("saved:a"));
}

// ── Accounts listed before a key was moved ──

#[test]
fn an_account_listed_before_the_move_reads_the_key_from_the_vault_after_it() {
    let vault = Fake::new();
    vault.store("saved:p1", KEY).unwrap();
    let mut before = entry("p1", "Mine");
    before.unmigrated_key = Some(KEY.into());
    let list = accounts(std::slice::from_ref(&before), &[], &Default::default(), &Default::default());
    assert_eq!(list[0].key, KeyRef::Saved { entry: "p1".into() });
    // The file has since been rewritten with a reference only.
    let mut after = entry("p1", "Mine");
    after.api_key_ref = Some(reference("p1"));
    let fetch = |id: &str| vault.value(id).ok().flatten();
    assert_eq!(resolve(&list[0], std::slice::from_ref(&after), &fetch).unwrap().key.as_deref(), Some(KEY));
    vault.shut(true);
    let fetch = |id: &str| vault.value(id).ok().flatten();
    assert_eq!(resolve(&list[0], std::slice::from_ref(&after), &fetch).unwrap_err(), Missing::Vault);
}

// ── A reference is trusted only for its own entry ──

#[test]
fn a_reference_to_another_entry_or_a_free_tier_is_no_key() {
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    vault.store("groq", "fake-free-tier-key").unwrap();
    vault.store("saved:other", OTHER).unwrap();
    for foreign in ["vault:groq", "vault:saved:other", "vault:", "saved:p1", "groq"] {
        let mut e = entry("p1", "Mine");
        e.base_url = "https://attacker.example/v1".into();
        e.api_key_ref = Some(foreign.into());
        assert!(!e.has_key(), "{foreign}");
        assert_eq!(e.vault_key_id(), None, "{foreign}");
        assert_eq!(e.resolve_key(), Ok(None), "{foreign}: nothing is read, so nothing goes to its address");
        let list = accounts(std::slice::from_ref(&e), &[], &Default::default(), &Default::default());
        assert_eq!(list[0].key, KeyRef::None, "{foreign}");
        assert_eq!(forget(&*vault, &e), Ok(()));
    }
    assert!(vault.has("groq") && vault.has("saved:other"), "removing deletes only an entry's own key");
}

// ── A save while something else changes the list ──

#[test]
fn a_migration_and_a_removal_during_a_save_are_kept() {
    let vault = Fake::new();
    let path = file("lost-update", &legacy(&[("a", KEY), ("b", OTHER)]));
    let providers = Arc::new(shared(&path));
    let at = p(&path);

    // While the vault answers the save, the first provider's key is moved and the second removed.
    let live = providers.clone();
    let (v, at2) = (vault.clone(), at.clone());
    *vault.during_store.lock().unwrap() = Some(Box::new(move || {
        let mut s = live.lock().unwrap();
        v.held.lock().unwrap().insert("saved:a".into(), KEY.into());
        let a = s.entries.iter_mut().find(|e| e.id == "a").unwrap();
        a.api_key_ref = Some(reference("a"));
        a.unmigrated_key = None;
        s.entries.retain(|e| e.id != "b");
        s.save_at(&at2).unwrap();
    }));
    let form = Form {
        name: "Third".into(),
        provider_type: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: Some("fake-key-not-a-secret-0003".into()),
        auth_type: "bearer".into(),
        model: "m".into(),
    };
    let saved = save_form_with(&providers, "", form, Some(vault.clone()), &mut |s| s.save_at(&at)).expect("saved");

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(KEY) && !text.contains(OTHER) && !text.contains("0003"), "no plaintext came back:\n{text}");
    let now = load(&path);
    let ids: Vec<_> = now.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["a", saved.id.as_str()], "the removed provider stayed removed");
    assert!(now.entries[0].api_key_ref.is_some() && now.entries[0].unmigrated_key.is_none());
    assert_eq!(vault.get(&vault_id(&saved.id)).as_deref(), Some("fake-key-not-a-secret-0003"));
}

#[test]
fn a_provider_removed_while_it_is_edited_is_not_brought_back() {
    let vault = Fake::new();
    let path = file("edit-removed", "entries: []\n");
    let mut store = ProviderStore::default();
    let mut e = entry("a", "A");
    e.api_key = Some(KEY.into());
    store.entries.push(e);
    store.keep_keys(&*vault).unwrap();
    let providers = Arc::new(Mutex::new(store));
    let at = p(&path);
    let live = providers.clone();
    *vault.during_store.lock().unwrap() = Some(Box::new(move || live.lock().unwrap().entries.clear()));
    let form = Form {
        name: "A".into(),
        provider_type: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: Some(OTHER.into()),
        auth_type: "bearer".into(),
        model: "m".into(),
    };
    let said = save_form_with(&providers, "a", form, Some(vault.clone()), &mut |s| s.save_at(&at)).unwrap_err();
    assert!(said.contains("removed"), "{said}");
    assert!(providers.lock().unwrap().entries.is_empty());
}
