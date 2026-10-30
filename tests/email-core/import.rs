//! `email.import_account`'s store side (#1022): bringing in an account the companion kept in
//! `config.yaml`. Every secret here is obviously fake.
//!
//! The rules under test: the address is the identity (one the service has is left exactly as it
//! is, secret and all); a new one has its secret kept in the vault and read back before it is
//! listed; a shut or failing vault imports nothing and leaves the accounts file as it was.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use yantrik_foundation::vault::memory::MemoryVault;
use yantrik_foundation::vault::KeyVault;
use yantrik_ipc_contracts::email::{AccountSettings, ImportAccount};

use super::secrets::{vault_id, MailSecrets};
use super::store::{AccountStore, Doing, StoreError};

const PASSWORD: &str = "fake-import-password-SENTINEL-0101";
const TYPED: &str = "fake-typed-password-SENTINEL-0102";
const ACCESS: &str = "fake-import-access-SENTINEL-0103";
const REFRESH: &str = "fake-import-refresh-SENTINEL-0104";

static N: AtomicUsize = AtomicUsize::new(0);

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("yantrik-mail-import-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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

fn password_import(email: &str) -> ImportAccount {
    ImportAccount {
        email: email.to_string(),
        display_name: "Ada".to_string(),
        provider: "advanced".to_string(),
        imap_server: "imap.example.org".to_string(),
        imap_port: 993,
        smtp_server: "smtp.example.org".to_string(),
        smtp_port: 587,
        password: PASSWORD.to_string(),
        oauth_access_token: None,
        oauth_refresh_token: None,
        oauth_expires_at: 0,
    }
}

fn google_import(email: &str) -> ImportAccount {
    ImportAccount {
        password: String::new(),
        oauth_access_token: Some(ACCESS.to_string()),
        oauth_refresh_token: Some(REFRESH.to_string()),
        oauth_expires_at: 1_790_000_000,
        ..password_import(email)
    }
}

#[test]
fn a_new_account_is_kept_in_the_vault_then_listed_and_the_file_holds_no_secret() {
    let (dir, vault) = (Dir::new(), Arc::new(MemoryVault::new()));
    let store = store_in(&dir, &vault);

    let result = store.import(&password_import("ada@example.org")).unwrap();

    assert!(result.created);
    assert_eq!(result.account.email, "ada@example.org");
    let kept = vault.peek(&vault_id(&result.account.id)).expect("the vault has it");
    assert_eq!(MailSecrets::from_json(&kept).unwrap().password, PASSWORD);
    assert!(!dir.text().contains(PASSWORD), "the file holds the password");
    assert!(dir.text().contains("ada@example.org"));
}

#[test]
fn a_google_account_keeps_its_tokens_in_the_vault_and_no_password() {
    let (dir, vault) = (Dir::new(), Arc::new(MemoryVault::new()));
    let store = store_in(&dir, &vault);

    let result = store.import(&google_import("g@gmail.com")).unwrap();

    assert!(result.created && result.account.uses_oauth);
    let secrets = MailSecrets::from_json(&vault.peek(&vault_id(&result.account.id)).unwrap()).unwrap();
    assert_eq!((secrets.oauth_token.as_deref(), secrets.oauth_refresh_token.as_deref()), (Some(ACCESS), Some(REFRESH)));
    assert!(secrets.password.is_empty());
    let file = dir.text();
    assert!(!file.contains(ACCESS) && !file.contains(REFRESH));
}

#[test]
fn an_address_the_service_already_has_is_left_exactly_as_it_is() {
    let (dir, vault) = (Dir::new(), Arc::new(MemoryVault::new()));
    let store = store_in(&dir, &vault);
    let typed = AccountSettings {
        email: "ada@example.org".into(),
        display_name: "Ada (typed)".into(),
        provider: "advanced".into(),
        imap_server: "imap.typed.example".into(),
        imap_port: 993,
        smtp_server: "smtp.typed.example".into(),
        smtp_port: 587,
        password: TYPED.into(),
    };
    let saved = store.save_settings(&typed).unwrap();
    let (file_before, writes_before) = (dir.text(), vault.writes());

    // Same address, other case, other server, other password.
    let result = store.import(&password_import("Ada@Example.org")).unwrap();

    assert!(!result.created);
    assert_eq!(result.account.id, saved.id);
    assert_eq!(dir.text(), file_before, "the accounts file changed");
    assert_eq!(vault.writes(), writes_before, "the vault was written");
    let kept = MailSecrets::from_json(&vault.peek(&vault_id(&saved.id)).unwrap()).unwrap();
    assert_eq!(kept.password, TYPED, "the person's own sign-in was replaced by an older copy");
}

#[test]
fn a_shut_vault_imports_nothing_and_says_it_is_waiting() {
    let (dir, vault) = (Dir::new(), Arc::new(MemoryVault::new()));
    vault.shut(true);
    let store = store_in(&dir, &vault);

    let refused = store.import(&password_import("ada@example.org")).unwrap_err();

    assert_eq!(refused, StoreError::Waiting(Doing::Saving));
    assert!(!dir.file().exists(), "an account was written with its secret not kept");

    // Opened: the same import goes through.
    vault.shut(false);
    assert!(store.import(&password_import("ada@example.org")).unwrap().created);
}

#[test]
fn a_vault_that_will_not_keep_it_leaves_the_file_alone() {
    let (dir, vault) = (Dir::new(), Arc::new(MemoryVault::new()));
    vault.refuse_writes(true);
    let store = store_in(&dir, &vault);

    let refused = store.import(&password_import("ada@example.org")).unwrap_err();

    assert_eq!(refused, StoreError::Unavailable(Doing::Saving));
    assert!(!dir.file().exists());
}

#[test]
fn an_account_with_no_server_is_refused_before_anything_is_kept() {
    let (dir, vault) = (Dir::new(), Arc::new(MemoryVault::new()));
    let store = store_in(&dir, &vault);
    let mut odd = password_import("odd@corp.example");
    odd.imap_server = String::new();

    let refused = store.import(&odd).unwrap_err();

    assert!(matches!(refused, StoreError::Refused(_)));
    assert_eq!(refused.into_service_error().code, -32602);
    assert_eq!(vault.writes(), 0);
    assert!(!dir.file().exists());
}

#[test]
fn nothing_that_prints_an_import_prints_its_secret() {
    let mut import = google_import("g@gmail.com");
    import.password = PASSWORD.to_string();
    let said = format!("{import:?}");
    for secret in [PASSWORD, ACCESS, REFRESH] {
        assert!(!said.contains(secret), "{said}");
    }

    let (dir, vault) = (Dir::new(), Arc::new(MemoryVault::new()));
    let store = store_in(&dir, &vault);
    vault.shut(true);
    let error = store.import(&password_import("ada@example.org")).unwrap_err().into_service_error();
    assert!(!format!("{error:?} {}", error.message).contains(PASSWORD));
}

#[test]
fn the_import_handler_logs_no_secret_and_never_formats_the_request() {
    let source = include_str!("../../services/email-service/src/import.rs");
    let mut seen = 0;
    for line in source.lines().filter(|l| l.contains("tracing::")) {
        seen += 1;
        let words: Vec<String> = line
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(str::to_lowercase)
            .collect();
        for forbidden in ["password", "token", "tokens", "secret", "secrets", "refresh", "settings", "access", "import"] {
            // `imported` is the word for what happened; the request itself is `import`.
            assert!(!words.iter().any(|w| w == forbidden), "a log line names `{forbidden}`: {line}");
        }
    }
    assert_eq!(seen, 1, "the handler logs one line, by account id only");
    for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
        assert!(!(line.contains("{:?}") || line.contains("{e}") || line.contains("{e:")), "the request or an error is formatted: {line}");
    }
    // A deserialiser's error can quote the text it could not read, which is a secret here.
    assert!(source.contains("map_err(|_|"), "the parse error must be dropped, not formatted");
}
