//! The vault being opened: what waited for it runs. Every key here is obviously fake.

use super::sources::{code_of, sources};
use super::*;
use crate::wire::provider_panel::{save_form_with, start_or_defer, Reload};

/// What waits for the vault to open is one list for the whole process, so the tests that put
/// work on it or run it take turns.
fn deferred_tests() -> std::sync::MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    turn
}

fn waiting_for_the_vault() -> usize {
    DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).len()
}

// ── The reload at start, replayed when the vault opens ──

#[test]
fn a_start_reload_refused_for_a_shut_vault_is_replayed_when_it_opens() {
    let _turn = deferred_tests();
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    vault.store("saved:p1", KEY).unwrap();
    vault.shut(true);
    let mut e = entry("p1", "Mine");
    e.api_key_ref = Some(reference("p1"));
    e.is_primary = true;

    let sent: Arc<Mutex<Vec<(Option<String>, String)>>> = Arc::default();
    let log = sent.clone();
    let send: crate::wire::provider_panel::Sender = Arc::new(move |r: Reload| log.lock().unwrap().push((r.key, r.model)));
    let primary = e.clone();
    let outcome = start_or_defer(&e, send, Arc::new(move || Some(primary.clone())), Arc::new(|| false));
    assert_eq!(outcome, Err(KeyError::Locked));
    assert!(sent.lock().unwrap().is_empty(), "nothing is sent with the vault shut");

    // Opened: the reload is done, once.
    vault.shut(false);
    opened();
    assert_eq!(*sent.lock().unwrap(), [(Some(KEY.to_string()), "m".to_string())]);
    opened();
    assert_eq!(sent.lock().unwrap().len(), 1, "once");
}

#[test]
fn a_replayed_start_reload_leaves_a_companion_on_yantrik_models_alone() {
    let _turn = deferred_tests();
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    vault.store("saved:p1", KEY).unwrap();
    vault.shut(true);
    let mut e = entry("p1", "Mine");
    e.api_key_ref = Some(reference("p1"));
    let sent: Arc<Mutex<usize>> = Arc::default();
    let count = sent.clone();
    let send: crate::wire::provider_panel::Sender = Arc::new(move |_| *count.lock().unwrap() += 1);
    let primary = e.clone();
    let _ = start_or_defer(&e, send, Arc::new(move || Some(primary.clone())), Arc::new(|| true));
    vault.shut(false);
    opened();
    assert_eq!(*sent.lock().unwrap(), 0);
}

#[test]
fn a_start_reload_that_got_no_answer_is_replayed_when_the_vault_opens() {
    let _turn = deferred_tests();
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    vault.store("saved:p1", KEY).unwrap();
    *vault.silent.lock().unwrap() = true;
    let mut e = entry("p1", "Mine");
    e.api_key_ref = Some(reference("p1"));
    e.is_primary = true;

    let sent: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let log = sent.clone();
    let send: crate::wire::provider_panel::Sender = Arc::new(move |r: Reload| log.lock().unwrap().push(r.key));
    let primary = e.clone();
    let outcome = start_or_defer(&e, send, Arc::new(move || Some(primary.clone())), Arc::new(|| false));
    assert_eq!(outcome, Err(KeyError::Unavailable));
    assert!(sent.lock().unwrap().is_empty());

    *vault.silent.lock().unwrap() = false;
    opened();
    assert_eq!(*sent.lock().unwrap(), [Some(KEY.to_string())]);
}

#[test]
fn a_key_that_is_missing_from_an_open_vault_is_not_replayed() {
    let _turn = deferred_tests();
    let vault = Fake::new();
    let _using = Using::vault(&vault);
    let mut e = entry("p1", "Mine");
    e.api_key_ref = Some(reference("p1"));
    let send: crate::wire::provider_panel::Sender = Arc::new(|_| {});
    let outcome = start_or_defer(&e, send, Arc::new(|| None), Arc::new(|| false));
    assert_eq!(outcome, Err(KeyError::Missing));
    assert_eq!(waiting_for_the_vault(), 0);
}

// ── The move into the vault, after a start that did not reach it ──

fn start(vault: &Arc<Fake>, store: &Arc<Mutex<ProviderStore>>, path: &PathBuf) -> Migration {
    let at = p(path);
    let save: SaveFile = Arc::new(move |s| s.save_at(&at));
    let vault: Arc<dyn KeyVault> = vault.clone();
    begin_migration(&vault, store, &save)
}

#[test]
fn a_start_pass_that_failed_moves_the_key_when_the_vault_opens() {
    let _turn = deferred_tests();
    let vault = Fake::new();
    let path = file("failed-start", &legacy(&[("a", KEY)]));
    let store = Arc::new(shared(&path));
    // The worker is still starting: the vault does not take the key.
    *vault.refuse_writes.lock().unwrap() = true;
    let first = start(&vault, &store, &path);
    assert_eq!(first, Migration { moved: 0, waiting: 0, failed: 1 });
    assert!(std::fs::read_to_string(&path).unwrap().contains(KEY));
    assert_eq!(waiting_for_the_vault(), 1);

    // It answers, and the person unlocks: the key is moved without another start.
    *vault.refuse_writes.lock().unwrap() = false;
    opened();
    assert_eq!(vault.get("saved:a").as_deref(), Some(KEY));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(KEY), "the key left the file:\n{text}");
    assert_eq!(load(&path).entries[0].api_key_ref.as_deref(), Some("vault:saved:a"));
    assert_eq!(waiting_for_the_vault(), 0);

    // Once: another opening finds nothing to do.
    let writes = vault.writes.load(Ordering::SeqCst);
    opened();
    assert_eq!(vault.writes.load(Ordering::SeqCst), writes);
}

#[test]
fn a_pass_at_an_opening_that_still_cannot_reach_the_vault_asks_for_the_next_one() {
    let _turn = deferred_tests();
    let vault = Fake::new();
    let path = file("shut-twice", &legacy(&[("a", KEY)]));
    let store = Arc::new(shared(&path));
    vault.shut(true);
    assert_eq!(start(&vault, &store, &path).waiting, 1);
    opened();
    assert_eq!(waiting_for_the_vault(), 1, "still waiting");
    assert!(!vault.has("saved:a"));
    vault.shut(false);
    opened();
    assert_eq!(vault.get("saved:a").as_deref(), Some(KEY));
    assert_eq!(waiting_for_the_vault(), 0);
}

#[test]
fn a_start_pass_with_nothing_left_waits_for_nothing() {
    let _turn = deferred_tests();
    let vault = Fake::new();
    let path = file("nothing-left", &legacy(&[("a", KEY)]));
    let store = Arc::new(shared(&path));
    assert_eq!(start(&vault, &store, &path).moved, 1);
    assert_eq!(start(&vault, &store, &path), Migration::default(), "a second pass does nothing");
    assert_eq!(waiting_for_the_vault(), 0);
    assert_eq!(vault.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_save_that_completes_during_a_migration_keeps_the_new_key() {
    let vault = Fake::new();
    let path = file("save-during-migration", &legacy(&[("a", KEY)]));
    let providers = Arc::new(shared(&path));
    let at = p(&path);

    // While the pass is writing the old key, the person saves a new one for the same provider.
    // Without `key_writes` that save finishes inside the wait, and the old key lands over it.
    type Saving = std::thread::JoinHandle<Result<ProviderStoreEntry, String>>;
    let saver: Arc<Mutex<Option<Saving>>> = Arc::default();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let (done_tx, done_rx) = (Mutex::new(done_tx), Mutex::new(done_rx));
    let (live, v, at2, handle) = (providers.clone(), vault.clone(), at.clone(), saver.clone());
    *vault.before_write.lock().unwrap() = Some(Arc::new(move || {
        let (live, v, at2) = (live.clone(), v.clone(), at2.clone());
        let done = done_tx.lock().unwrap().clone();
        *handle.lock().unwrap() = Some(std::thread::spawn(move || {
            let form = Form {
                name: "A".into(),
                provider_type: "openrouter".into(),
                base_url: "https://openrouter.ai/api/v1".into(),
                api_key: Some(OTHER.into()),
                auth_type: "bearer".into(),
                model: "m".into(),
            };
            let saved = save_form_with(&live, "a", form, Some(v), &mut |s| s.save_at(&at2));
            let _ = done.send(());
            saved
        }));
        let _ = done_rx.lock().unwrap().recv_timeout(std::time::Duration::from_millis(500));
    }));

    let report = run_migration(&vault, &providers, &path);
    assert_eq!(report.moved, 1);
    let handle = saver.lock().unwrap().take().expect("the save started");
    handle.join().unwrap().expect("saved");

    assert_eq!(vault.get("saved:a").as_deref(), Some(OTHER), "the new key is the one kept");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(KEY) && !text.contains(OTHER), "no plaintext in the file:\n{text}");
    assert_eq!(load(&path).entries[0].api_key_ref.as_deref(), Some("vault:saved:a"));
}

#[test]
fn a_provider_removed_during_a_migration_leaves_no_key_in_the_vault() {
    let vault = Fake::new();
    let path = file("remove-during-migration", &legacy(&[("a", KEY)]));
    let providers = Arc::new(shared(&path));
    let at = p(&path);

    // While the pass is writing the copied key, the person removes the provider. Without
    // `key_writes` the remove finishes inside the wait, finds no vault reference yet, and the
    // pass then keeps the key for an entry that is gone.
    type Removing = std::thread::JoinHandle<Result<(), String>>;
    let remover: Arc<Mutex<Option<Removing>>> = Arc::default();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let (done_tx, done_rx) = (Mutex::new(done_tx), Mutex::new(done_rx));
    let (live, v, at2, handle) = (providers.clone(), vault.clone(), at.clone(), remover.clone());
    *vault.before_write.lock().unwrap() = Some(Arc::new(move || {
        let (live, v, at2) = (live.clone(), v.clone(), at2.clone());
        let done = done_tx.lock().unwrap().clone();
        *handle.lock().unwrap() = Some(std::thread::spawn(move || {
            let v: Arc<dyn KeyVault> = v;
            let removed = remove_provider_with(&live, "a", Some(v), &mut |s| s.save_at(&at2));
            let _ = done.send(());
            removed
        }));
        let _ = done_rx.lock().unwrap().recv_timeout(std::time::Duration::from_millis(500));
    }));

    run_migration(&vault, &providers, &path);
    let handle = remover.lock().unwrap().take().expect("the remove started");
    handle.join().unwrap().expect("removed");

    assert!(!vault.has("saved:a"), "the removed provider's key is not left in the vault");
    assert!(load(&path).entries.is_empty(), "the provider is gone");
    assert!(!std::fs::read_to_string(&path).unwrap().contains(KEY), "no plaintext in the file");
}

#[test]
fn every_place_the_vault_is_opened_runs_what_waited_for_it() {
    // The sign-in, the unlock card, and the PIN that unlocks the screen each open the vault; what
    // waited for it (saved providers' keys, the companion's reload) runs only if they say so.
    let places = [("/wire/login.rs", 2), ("/wire/vault.rs", 1), ("/wire/callbacks.rs", 1)];
    let all = sources();
    for (file, arms) in places {
        let (_, text) = all.iter().find(|(n, _)| n.ends_with(file)).unwrap_or_else(|| panic!("{file} is gone"));
        let lines: Vec<&str> = code_of(text).lines().collect();
        let mut found = 0;
        for (i, line) in lines.iter().enumerate() {
            let l = line.trim_start();
            let arm = !l.starts_with("//") && l.contains("=>") && (l.contains("Outcome::Unlocked") || l.contains("Outcome::Protected"));
            if !arm {
                continue;
            }
            found += 1;
            // The arm's body: the lines up to the next arm.
            let mut body = lines[i + 1..].iter().take(12).take_while(|l| !l.contains("=>"));
            assert!(
                body.any(|l| l.contains("provider_vault::opened()")),
                "{file}: the arm `{l}` does not call provider_vault::opened()"
            );
        }
        assert_eq!(found, arms, "{file}: the arms that open the vault are not the ones this test knows");
    }
}
