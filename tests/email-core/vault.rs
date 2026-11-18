//! Mail account secrets and the credential vault. Every secret here is obviously fake.
//!
//! The faults these cover were each invisible from one side alone. A password moved into the
//! vault and the file still had it; a vault that was shut at start and was never asked again; a
//! removed account whose password stayed behind; a log line that formatted the account it was
//! about. So each one is built with a sentinel secret and the file, the vault, the errors and the
//! `Debug` text are searched for it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use yantrik_foundation::vault::memory::MemoryVault;
use yantrik_foundation::vault::opening::{self, Pass};
use yantrik_foundation::vault::{KeyVault, VaultError};
use yantrik_ipc_contracts::credentials::ERR_LOCKED;
use yantrik_ipc_contracts::email::{AccountSettings, WAITING_FOR_VAULT};

use super::accounts::{self, Account};
use super::secrets::{vault_id, MailSecrets};
use super::store::{AccountStore, Doing, StoreError};

const PASSWORD: &str = "fake-mail-password-SENTINEL-0001";
const OTHER_PASSWORD: &str = "fake-mail-password-SENTINEL-0002";
const ACCESS: &str = "fake-access-token-SENTINEL-0003";
const REFRESH: &str = "fake-refresh-token-SENTINEL-0004";
const NEW_ACCESS: &str = "fake-new-access-token-SENTINEL-0005";

static N: AtomicUsize = AtomicUsize::new(0);

/// A directory of its own with an accounts file path in it, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("yantrik-mail-vault-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
    fn file(&self) -> PathBuf {
        self.0.join("yantrik/email.json")
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.file()).unwrap_or_default()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn store_in(dir: &Dir, vault: &Arc<MemoryVault>) -> AccountStore {
    let vault: Arc<dyn KeyVault> = vault.clone();
    AccountStore::new(dir.file(), vault)
}

/// An older service's file: a password account and a Google account, secrets in clear text.
fn legacy_file(dir: &Dir) {
    std::fs::create_dir_all(dir.file().parent().unwrap()).unwrap();
    let text = serde_json::json!([
        {
            "id": "a-example-org", "email": "a@example.org", "provider": "advanced",
            "password": PASSWORD, "imap_server": "imap.example.org", "imap_port": 993,
            "smtp_server": "smtp.example.org", "smtp_port": 587
        },
        {
            "id": "g-gmail-com", "email": "g@gmail.com", "provider": "gmail", "use_oauth": true,
            "imap_server": "imap.gmail.com", "imap_port": 993, "smtp_server": "smtp.gmail.com", "smtp_port": 587,
            "oauth_token": ACCESS, "oauth_refresh_token": REFRESH, "oauth_expires_at": 1_900_000_000i64
        }
    ]);
    std::fs::write(dir.file(), serde_json::to_string_pretty(&text).unwrap()).unwrap();
}

fn settings(email: &str, password: &str) -> AccountSettings {
    AccountSettings {
        email: email.into(),
        display_name: "Someone".into(),
        provider: "advanced".into(),
        imap_server: "imap.example.org".into(),
        imap_port: 993,
        smtp_server: "smtp.example.org".into(),
        smtp_port: 587,
        password: password.into(),
    }
}

fn kept(vault: &MemoryVault, id: &str) -> MailSecrets {
    MailSecrets::from_json(&vault.peek(&vault_id(id)).expect("the vault holds the account")).expect("what this service wrote")
}

/// A vault that refuses to keep one account's secrets and takes the rest.
struct Picky {
    inner: Arc<MemoryVault>,
    refuse: &'static str,
}

impl KeyVault for Picky {
    fn store(&self, id: &str, value: &str) -> Result<(), VaultError> {
        if id == vault_id(self.refuse) {
            return Err(VaultError::Failed);
        }
        self.inner.store(id, value)
    }
    fn value(&self, id: &str) -> Result<Option<String>, VaultError> {
        self.inner.value(id)
    }
    fn remove(&self, id: &str) -> Result<(), VaultError> {
        self.inner.remove(id)
    }
}

// ── Moving an older service's clear-text secrets ──

#[test]
fn migration_copies_into_the_vault_and_leaves_no_secret_in_the_file() {
    let dir = Dir::new();
    legacy_file(&dir);
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    assert!(store.accounts().unwrap().iter().all(Account::held_in_file), "both accounts start with their secrets in the file");

    let report = store.migrate();
    assert_eq!((report.moved, report.waiting, report.failed), (2, 0, 0));

    let text = dir.text();
    for secret in [PASSWORD, ACCESS, REFRESH] {
        assert!(!text.contains(secret), "a secret is still in the file:\n{text}");
    }
    assert!(text.contains("imap.example.org") && text.contains("1900000000"), "the account itself is kept:\n{text}");
    assert!(accounts::load_file(&dir.file()).unwrap().iter().all(|a| !a.held_in_file()));

    assert_eq!(kept(&vault, "a-example-org").password, PASSWORD);
    let google = kept(&vault, "g-gmail-com");
    assert_eq!((google.oauth_token.as_deref(), google.oauth_refresh_token.as_deref()), (Some(ACCESS), Some(REFRESH)));
}

#[test]
fn a_migrated_account_signs_in_with_what_the_vault_holds() {
    let dir = Dir::new();
    legacy_file(&dir);
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    store.migrate();

    let loaded = accounts::pick(&store.accounts().unwrap(), Some("a-example-org")).cloned().unwrap();
    assert_eq!(loaded.password, "", "the file gives no password");
    let resolved = store.resolve(loaded).unwrap();
    assert_eq!(resolved.password, PASSWORD);
    assert!(!resolved.held_in_file(), "a resolved account is never written back");
}

#[test]
fn a_second_pass_does_nothing_and_asks_the_vault_nothing() {
    let dir = Dir::new();
    legacy_file(&dir);
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    store.migrate();
    let writes = vault.writes();
    let before = dir.text();

    assert_eq!(store.migrate(), Default::default());
    assert_eq!(vault.writes(), writes);
    assert_eq!(dir.text(), before);
}

#[test]
fn a_shut_vault_leaves_the_file_exactly_as_it_was() {
    let dir = Dir::new();
    legacy_file(&dir);
    let before = std::fs::read(dir.file()).unwrap();
    let vault = Arc::new(MemoryVault::new());
    vault.shut(true);
    let store = store_in(&dir, &vault);

    let report = store.migrate();
    assert_eq!((report.moved, report.waiting, report.failed), (0, 2, 0));
    assert!(report.left_some());
    assert_eq!(std::fs::read(dir.file()).unwrap(), before, "not one byte changed");
    assert!(!vault.has(&vault_id("a-example-org")));
}

#[test]
fn a_vault_that_fails_or_garbles_leaves_the_file_exactly_as_it_was() {
    for how in ["refuses", "garbles", "is silent"] {
        let dir = Dir::new();
        legacy_file(&dir);
        let before = std::fs::read(dir.file()).unwrap();
        let vault = Arc::new(MemoryVault::new());
        match how {
            "refuses" => vault.refuse_writes(true),
            "garbles" => vault.garble(true),
            _ => vault.go_silent(true),
        }
        let report = store_in(&dir, &vault).migrate();
        assert_eq!((report.moved, report.waiting, report.failed), (0, 0, 2), "a vault that {how}");
        assert_eq!(std::fs::read(dir.file()).unwrap(), before, "a vault that {how} changed the file");
    }
}

#[test]
fn an_account_the_vault_would_not_keep_keeps_its_text_while_the_others_move() {
    let dir = Dir::new();
    legacy_file(&dir);
    let inner = Arc::new(MemoryVault::new());
    let picky: Arc<dyn KeyVault> = Arc::new(Picky { inner: inner.clone(), refuse: "a-example-org" });
    let store = AccountStore::new(dir.file(), picky);

    let report = store.migrate();
    assert_eq!((report.moved, report.waiting, report.failed), (1, 0, 1));
    let text = dir.text();
    assert!(text.contains(PASSWORD), "the refused account still has its password where it was:\n{text}");
    assert!(!text.contains(ACCESS) && !text.contains(REFRESH), "the account that moved has none");
    let after = store.accounts().unwrap();
    assert!(after.iter().find(|a| a.id == "a-example-org").unwrap().held_in_file());
    assert!(!after.iter().find(|a| a.id == "g-gmail-com").unwrap().held_in_file());
}

#[test]
fn a_migration_that_started_with_the_vault_shut_finishes_when_it_opens() {
    // As `main` runs it: one pass at start, and the same pass again at each opening.
    let dir = Dir::new();
    legacy_file(&dir);
    let vault = Arc::new(MemoryVault::new());
    vault.shut(true);
    let store = store_in(&dir, &vault);
    let pass: Pass = {
        let store = store.clone();
        Arc::new(move |_| store.migrate().left_some())
    };

    // These tests share the process-wide queue, so they take turns.
    static TURN: Mutex<()> = Mutex::new(());
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    opening::forget_waiting();

    assert!(opening::begin(&pass), "the first pass left secrets behind");
    assert!(dir.text().contains(PASSWORD));
    assert_eq!(opening::waiting(), 1, "and asked to be run when the vault opens");

    vault.shut(false);
    opening::opened();
    let text = dir.text();
    assert!(!text.contains(PASSWORD) && !text.contains(REFRESH), "the unlock moved them:\n{text}");
    assert_eq!(opening::waiting(), 0, "and nothing waits for another opening");
    assert_eq!(kept(&vault, "a-example-org").password, PASSWORD);
}

// ── Saving ──

#[test]
fn a_saved_account_puts_its_password_in_the_vault_and_not_in_the_file() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);

    let saved = store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();
    assert_eq!(saved.email, "a@example.org");
    assert!(!dir.text().contains(PASSWORD), "the password is in the file:\n{}", dir.text());
    assert!(!format!("{saved:?}").contains(PASSWORD));
    assert_eq!(kept(&vault, &saved.id).password, PASSWORD);

    let resolved = store.resolve(store.accounts().unwrap().remove(0)).unwrap();
    assert_eq!(resolved.password, PASSWORD);
}

#[test]
fn saving_an_account_with_the_vault_shut_writes_nothing() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    vault.shut(true);
    let store = store_in(&dir, &vault);

    assert_eq!(store.check_open(Doing::Saving), Err(StoreError::Waiting(Doing::Saving)));
    let refused = store.save_settings(&settings("a@example.org", PASSWORD)).unwrap_err();
    assert_eq!(refused, StoreError::Waiting(Doing::Saving));
    assert!(!dir.file().exists(), "no file was made for an account whose password could not be kept");
    let said = refused.into_service_error();
    assert_eq!(said.code, ERR_LOCKED);
    assert!(!said.message.contains(PASSWORD));
}

#[test]
fn a_vault_that_does_not_keep_the_password_leaves_the_file_alone() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();
    let before = std::fs::read(dir.file()).unwrap();

    vault.garble(true);
    let refused = store.save_settings(&settings("b@example.org", OTHER_PASSWORD)).unwrap_err();
    assert_eq!(refused, StoreError::Unavailable(Doing::Saving));
    assert_eq!(std::fs::read(dir.file()).unwrap(), before, "the second account was not written");
}

#[test]
fn a_google_sign_in_keeps_its_tokens_in_the_vault_and_drops_an_old_password() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    store.save_settings(&settings("g@gmail.com", PASSWORD)).unwrap();

    let saved = store.save_google("g@gmail.com", ACCESS, REFRESH, 1_900_000_000).unwrap();
    assert!(saved.uses_oauth);
    let text = dir.text();
    for secret in [PASSWORD, ACCESS, REFRESH] {
        assert!(!text.contains(secret), "{secret} is in the file:\n{text}");
    }
    assert!(text.contains("1900000000"), "when the token stops working is not secret:\n{text}");
    let held = kept(&vault, &saved.id);
    assert_eq!(held.password, "", "the app password is no longer kept for anything");
    assert_eq!(held.oauth_token.as_deref(), Some(ACCESS));
    assert_eq!(held.oauth_refresh_token.as_deref(), Some(REFRESH));
}

#[test]
fn a_renewed_token_goes_to_the_vault_and_its_expiry_to_the_file() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    store.save_google("g@gmail.com", ACCESS, REFRESH, 100).unwrap();
    let account = store.resolve(store.accounts().unwrap().remove(0)).unwrap();

    assert_eq!(store.store_refreshed(&account, NEW_ACCESS, REFRESH, 7_200).unwrap(), true);
    let text = dir.text();
    for secret in [ACCESS, REFRESH, NEW_ACCESS] {
        assert!(!text.contains(secret), "{secret} is in the file:\n{text}");
    }
    assert!(text.contains("7200"));
    assert_eq!(kept(&vault, &account.id).oauth_token.as_deref(), Some(NEW_ACCESS));

    store.remove(&account.id).unwrap();
    assert_eq!(store.store_refreshed(&account, NEW_ACCESS, REFRESH, 9_000).unwrap(), false, "removed meanwhile");
    assert!(!vault.has(&vault_id(&account.id)), "a renewal does not bring a removed account's secrets back");
}

// ── Removing ──

#[test]
fn removing_an_account_forgets_its_vault_entry() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    let a = store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();
    let b = store.save_settings(&settings("b@example.org", OTHER_PASSWORD)).unwrap();

    assert_eq!(store.remove("a@example.org"), Ok(true), "by address");
    assert!(!vault.has(&vault_id(&a.id)), "the password is gone from the vault");
    assert!(vault.has(&vault_id(&b.id)), "and only that one");
    let left = store.accounts().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, b.id);
    assert_eq!(store.remove("a@example.org"), Ok(false), "there is nothing left to remove");
}

#[test]
fn removing_with_the_vault_shut_removes_nothing() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    let a = store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();
    vault.shut(true);

    assert_eq!(store.remove(&a.id), Err(StoreError::Waiting(Doing::Removing)));
    vault.shut(false);
    assert_eq!(store.accounts().unwrap().len(), 1, "the account is still listed, so its entry can still be found");
    assert!(vault.has(&vault_id(&a.id)));
    assert_eq!(store.remove(&a.id), Ok(true), "and it goes once the vault is open");
    assert!(!vault.has(&vault_id(&a.id)));
}

#[test]
fn removal_asks_for_an_account_by_name_and_never_falls_back_to_the_first() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    let a = store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();

    assert_eq!(store.remove("default"), Ok(false), "`default` picks the first account for reading, not for removing");
    assert_eq!(store.remove(""), Ok(false));
    assert!(vault.has(&vault_id(&a.id)));
}

// ── Mail waits for the vault ──

#[test]
fn mail_is_not_checked_while_the_vault_is_shut_and_resumes_when_it_opens() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();
    let account = store.accounts().unwrap().remove(0);

    vault.shut(true);
    assert!(store.waiting_for_vault(&store.accounts().unwrap()), "`email.accounts` says so");
    let refused = store.resolve(account.clone()).unwrap_err();
    assert_eq!(refused, StoreError::Waiting(Doing::Reading));
    let said = refused.into_service_error();
    assert_eq!((said.code, said.message.as_str()), (ERR_LOCKED, WAITING_FOR_VAULT), "the plain sentence, on its own code");
    assert!(WAITING_FOR_VAULT.starts_with("Mail is waiting for the vault"));

    vault.shut(false);
    assert!(!store.waiting_for_vault(&store.accounts().unwrap()));
    assert_eq!(store.resolve(account).unwrap().password, PASSWORD, "the same call works once the vault is open");
}

#[test]
fn a_machine_with_no_accounts_is_not_waiting_for_anything() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    vault.shut(true);
    let store = store_in(&dir, &vault);
    assert!(!store.waiting_for_vault(&store.accounts().unwrap()));
}

#[test]
fn an_account_left_in_clear_text_waits_for_the_vault_like_any_other() {
    let dir = Dir::new();
    legacy_file(&dir);
    let vault = Arc::new(MemoryVault::new());
    vault.shut(true);
    let store = store_in(&dir, &vault);
    let account = accounts::pick(&store.accounts().unwrap(), Some("a-example-org")).cloned().unwrap();

    assert_eq!(store.resolve(account.clone()).unwrap_err(), StoreError::Waiting(Doing::Reading), "not used from the file");
    assert!(dir.text().contains(PASSWORD), "and not moved while the vault is shut");

    vault.shut(false);
    let resolved = store.resolve(account).unwrap();
    assert_eq!(resolved.password, PASSWORD);
    assert!(!dir.text().contains(PASSWORD), "using it moved it");
}

#[test]
fn an_open_vault_that_holds_nothing_for_the_account_says_to_enter_it_again() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    let saved = store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();
    vault.remove(&vault_id(&saved.id)).unwrap();

    let gone = store.resolve(store.accounts().unwrap().remove(0)).unwrap_err();
    assert!(matches!(gone, StoreError::Missing { google: false, .. }), "{gone:?}");
    let said = gone.into_service_error();
    assert!(said.message.contains("no longer in the credential vault") && said.message.contains("a@example.org"), "{}", said.message);
    assert!(!said.message.contains(PASSWORD));
}

#[test]
fn an_account_from_the_environment_needs_no_vault() {
    let from_env = Account { id: "e".into(), email: "e@example.org".into(), password: PASSWORD.into(), from_env: true, ..Account::default() };
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    vault.shut(true);
    let store = store_in(&dir, &vault);
    assert_eq!(store.resolve(from_env.clone()).unwrap().password, PASSWORD);
    assert!(!store.waiting_for_vault(&[from_env]));
}

// ── No secret in a log, a Debug, an error or a file ──

#[test]
fn nothing_that_prints_an_account_prints_a_secret() {
    let account = Account {
        id: "a".into(),
        email: "a@example.org".into(),
        password: PASSWORD.into(),
        oauth_token: Some(ACCESS.into()),
        oauth_refresh_token: Some(REFRESH.into()),
        in_file: true,
        ..Account::default()
    };
    let secrets = MailSecrets::from_account(&account);
    let errors = [
        StoreError::Waiting(Doing::Reading),
        StoreError::Waiting(Doing::Saving),
        StoreError::Waiting(Doing::Removing),
        StoreError::Unavailable(Doing::Reading),
        StoreError::Missing { email: "a@example.org".into(), google: true },
        StoreError::Missing { email: "a@example.org".into(), google: false },
    ];
    let mut shown = vec![format!("{account:?}"), format!("{secrets:?}"), format!("{:?}", settings("a@example.org", PASSWORD))];
    for e in errors {
        shown.push(format!("{e:?}"));
        shown.push(e.into_service_error().message);
    }
    for text in shown {
        for secret in [PASSWORD, ACCESS, REFRESH] {
            assert!(!text.contains(secret), "{secret} reached: {text}");
        }
    }
}

#[test]
fn a_file_saved_from_memory_never_holds_a_secret_that_was_not_already_in_it() {
    let dir = Dir::new();
    let mut all = Vec::new();
    accounts::upsert(&mut all, &settings("a@example.org", PASSWORD));
    accounts::upsert_google(&mut all, "g@gmail.com", ACCESS, REFRESH, 5);
    accounts::save(&dir.file(), &all).unwrap();

    let text = dir.text();
    for secret in [PASSWORD, ACCESS, REFRESH] {
        assert!(!text.contains(secret), "{secret} was written:\n{text}");
    }
}

#[test]
fn an_account_in_the_environment_is_not_written_into_the_file_by_a_change_to_another() {
    let dir = Dir::new();
    let vault = Arc::new(MemoryVault::new());
    let store = store_in(&dir, &vault);
    // `load_file` is what every change starts from, and it never includes the environment's.
    assert!(accounts::load_file(&dir.file()).unwrap().is_empty());
    store.save_settings(&settings("a@example.org", PASSWORD)).unwrap();
    assert_eq!(accounts::load_file(&dir.file()).unwrap().len(), 1);
}

/// The text of every `tracing::…!(…)` call in `source`, with its string literals taken out, so what
/// is left is the names of the values it logs.
fn tracing_calls(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("tracing::") {
        let from = &rest[at..];
        let Some(open) = from.find('(') else { break };
        let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
        let mut call = String::new();
        for c in from[open..].chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                continue;
            }
            match c {
                '"' => in_string = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            call.push(c);
        }
        found.push(call);
        rest = &from[open..];
    }
    found
}

#[test]
fn no_log_line_in_the_mail_service_names_a_secret() {
    let sources = [
        ("main.rs", include_str!("../../services/email-service/src/main.rs")),
        ("store.rs", include_str!("../../services/email-service/src/store.rs")),
        ("secrets.rs", include_str!("../../services/email-service/src/secrets.rs")),
        ("accounts.rs", include_str!("../../services/email-service/src/accounts.rs")),
        ("oauth.rs", include_str!("../../services/email-service/src/oauth.rs")),
        ("google.rs", include_str!("../../services/email-service/src/google.rs")),
    ];
    let mut seen = 0;
    for (file, source) in sources {
        for call in tracing_calls(source) {
            seen += 1;
            let words: Vec<String> = call
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map(str::to_lowercase)
                .collect();
            for forbidden in ["password", "token", "tokens", "secret", "secrets", "refresh", "settings", "access", "code"] {
                assert!(!words.iter().any(|w| w == forbidden), "{file}: a log line names `{forbidden}`:\n{call}");
            }
        }
    }
    assert!(seen >= 5, "the scan found only {seen} log lines; it is not reading the sources");
}

#[test]
fn the_mail_service_writes_no_secret_with_serde_and_never_formats_an_account() {
    // `Account` is not `Serialize`: the only way it reaches the file is `Account::stored`.
    let accounts_source = include_str!("../../services/email-service/src/accounts.rs");
    assert!(!accounts_source.contains("derive(Clone, Default, serde::Serialize"), "Account must not derive Serialize");
    let main = include_str!("../../services/email-service/src/main.rs");
    for line in main.lines().filter(|l| !l.trim_start().starts_with("//")) {
        assert!(!(line.contains("{:?}") && (line.contains("account") || line.contains("settings") || line.contains("secrets"))), "{line}");
    }
}
