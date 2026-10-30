//! The migration of `config.yaml` mail accounts (#1022): into the service and the vault, with the
//! original kept, and the file touched only when a copy of it is safe.
//!
//! Every password here is a made-up sentinel, and the tests search for it everywhere it must not
//! be.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::email::fake::{account, FakeMail, Vault};

const ADA_PASSWORD: &str = "SENTINEL-ada-pw-7f3a9";
const BOB_PASSWORD: &str = "SENTINEL-bob-pw-2c8d1";
const GOOGLE_ACCESS: &str = "SENTINEL-access-token-55aa";
const GOOGLE_REFRESH: &str = "SENTINEL-refresh-token-66bb";

/// A directory of the test's own, removed when it goes out of scope.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("yos-1022-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn config(&self) -> PathBuf {
        self.0.join("config.yaml")
    }

    /// Every file in the directory, by name.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(&self.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config_text() -> String {
    format!(
        "# the person's own notes\nllm:\n  backend: api\nemail:\n  enabled: true\n  poll_interval_minutes: 5\n  accounts:\n    \
         - name: Ada\n      email: ada@example.com\n      provider: gmail\n      password: {ADA_PASSWORD}\n    \
         - name: Bob\n      email: bob@example.com\n      provider: yahoo\n      password: {BOB_PASSWORD}\ncalendar:\n  enabled: true\n"
    )
}

fn write_config(dir: &Scratch, text: &str) -> PathBuf {
    std::fs::write(dir.config(), text).unwrap();
    dir.config()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

const STAMP: &str = "20261030T101500";

// ── Into the service and the vault ──────────────────────────────────────────────────────────

#[test]
fn an_account_the_service_does_not_have_is_created_with_its_secret_in_the_vault_then_dropped() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    let mail = FakeMail::empty();

    let outcome = migrate(&config, &mail, STAMP);

    assert_eq!((outcome.moved, outcome.already_there), (2, 0), "{outcome:?}");
    let state = mail.state.lock().unwrap();
    assert_eq!(state.kept.get("mail:ada-at-example.com").map(String::as_str), Some(ADA_PASSWORD));
    assert_eq!(state.kept.get("mail:bob-at-example.com").map(String::as_str), Some(BOB_PASSWORD));
    drop(state);

    let after = read(&config);
    assert!(!after.contains(ADA_PASSWORD) && !after.contains(BOB_PASSWORD), "a password is still in config.yaml");
    assert!(!after.contains("accounts"), "an empty accounts list is not left behind:\n{after}");
}

#[test]
fn what_else_is_in_the_config_survives_the_rewrite() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    migrate(&config, &FakeMail::empty(), STAMP);

    let after: serde_yaml::Value = serde_yaml::from_str(&read(&config)).unwrap();
    assert_eq!(after["llm"]["backend"], "api");
    assert_eq!(after["email"]["enabled"], true);
    assert_eq!(after["email"]["poll_interval_minutes"], 5);
    assert_eq!(after["calendar"]["enabled"], true);
}

#[test]
fn an_address_the_service_already_has_is_left_alone_and_only_its_password_leaves_the_file() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    let mail = FakeMail::with_accounts(vec![account("ada-at-example.com", "Ada@Example.com")]);

    let outcome = migrate(&config, &mail, STAMP);

    assert_eq!((outcome.moved, outcome.already_there), (1, 1), "{outcome:?}");
    let state = mail.state.lock().unwrap();
    assert!(!state.kept.contains_key("mail:ada-at-example.com"), "the existing account's secret was replaced");
    assert_eq!(state.accounts.len(), 2, "no second Ada");
    drop(state);
    assert!(!read(&config).contains(ADA_PASSWORD));
}

#[test]
fn a_google_account_moves_its_tokens_and_no_password() {
    let dir = Scratch::new();
    let text = format!(
        "email:\n  enabled: true\n  accounts:\n    - name: G\n      email: g@example.com\n      provider: gmail\n      \
         auth_method: oauth2\n      oauth_access_token: {GOOGLE_ACCESS}\n      oauth_refresh_token: {GOOGLE_REFRESH}\n      \
         oauth_token_expiry: 1790000000.0\n"
    );
    let config = write_config(&dir, &text);
    let mail = FakeMail::empty();

    let outcome = migrate(&config, &mail, STAMP);

    assert_eq!(outcome.moved, 1, "{outcome:?}");
    assert_eq!(
        mail.state.lock().unwrap().kept.get("mail:g-at-example.com").map(String::as_str),
        Some(format!("{GOOGLE_ACCESS}|{GOOGLE_REFRESH}").as_str())
    );
    let after = read(&config);
    assert!(!after.contains(GOOGLE_ACCESS) && !after.contains(GOOGLE_REFRESH));
}

#[test]
fn an_entry_that_holds_no_secret_is_left_where_it_is_and_asks_nothing() {
    let dir = Scratch::new();
    let text = "email:\n  accounts:\n    - name: Plain\n      email: p@example.com\n      provider: gmail\n";
    let config = write_config(&dir, text);
    let mail = FakeMail::empty();

    let outcome = migrate(&config, &mail, STAMP);

    assert_eq!(outcome, Outcome::default());
    assert_eq!(read(&config), text);
    assert_eq!(mail.count("import_account"), 0);
    assert_eq!(dir.files(), vec!["config.yaml"], "nothing changed, so nothing was archived");
}

// ── The vault is shut, then failing, then open ───────────────────────────────────────────────

#[test]
fn a_shut_vault_leaves_config_yaml_exactly_as_it_was_and_the_next_pass_moves_it() {
    let dir = Scratch::new();
    let original = config_text();
    let config = write_config(&dir, &original);
    let mail = FakeMail::empty();

    // Shut.
    mail.set_vault(Vault::Shut);
    let outcome = migrate(&config, &mail, STAMP);
    assert!(outcome.left_some() && outcome.waiting == 2 && outcome.moved == 0, "{outcome:?}");
    assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes(), "config.yaml changed while the vault was shut");
    assert_eq!(dir.files(), vec!["config.yaml"], "no archive is made for a file that was not changed");
    assert!(mail.state.lock().unwrap().kept.is_empty());

    // Failing.
    mail.set_vault(Vault::Failing);
    let outcome = migrate(&config, &mail, STAMP);
    assert!(outcome.left_some() && outcome.failed == 2 && outcome.moved == 0, "{outcome:?}");
    assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes(), "config.yaml changed while the vault failed");
    assert_eq!(dir.files(), vec!["config.yaml"]);

    // Unlocked.
    mail.set_vault(Vault::Open);
    let outcome = migrate(&config, &mail, STAMP);
    assert_eq!(outcome.moved, 2, "{outcome:?}");
    assert!(!outcome.left_some());
    assert!(!read(&config).contains(ADA_PASSWORD));
}

#[test]
fn an_account_the_service_has_does_not_wait_for_the_vault() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    let mail = FakeMail::with_accounts(vec![
        account("ada-at-example.com", "ada@example.com"),
        account("bob-at-example.com", "bob@example.com"),
    ]);
    mail.set_vault(Vault::Shut);

    let outcome = migrate(&config, &mail, STAMP);

    assert_eq!((outcome.already_there, outcome.waiting), (2, 0), "{outcome:?}");
    assert!(!read(&config).contains(ADA_PASSWORD));
}

// ── The person's config is never lost ────────────────────────────────────────────────────────

#[test]
fn the_original_is_archived_byte_for_byte_before_the_file_changes() {
    let dir = Scratch::new();
    let original = config_text();
    let config = write_config(&dir, &original);

    let outcome = migrate(&config, &FakeMail::empty(), STAMP);

    let archive = outcome.archive.clone().expect("an archive");
    assert_eq!(archive, dir.0.join("config.yaml.before-1022-20261030T101500"));
    assert_eq!(std::fs::read(&archive).unwrap(), original.as_bytes());
    assert!(config.exists());
    assert_eq!(dir.files(), vec!["config.yaml", "config.yaml.before-1022-20261030T101500"], "no temp file is left");
}

#[cfg(unix)]
#[test]
fn the_archive_is_private_and_belongs_to_the_files_owner_and_the_config_keeps_its_mode() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640)).unwrap();
    let owner = std::fs::metadata(&config).unwrap();

    let outcome = migrate(&config, &FakeMail::empty(), STAMP);

    let archive = std::fs::metadata(outcome.archive.unwrap()).unwrap();
    assert_eq!(archive.permissions().mode() & 0o777, 0o600, "the archive holds the old password");
    assert_eq!((archive.uid(), archive.gid()), (owner.uid(), owner.gid()));
    assert_eq!(std::fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o640, "the file's own mode");
}

#[test]
fn an_archive_that_cannot_be_made_leaves_the_file_alone() {
    let dir = Scratch::new();
    let original = config_text();
    let config = write_config(&dir, &original);

    // A stamp that makes the archive's directory not exist.
    let outcome = migrate(&config, &FakeMail::empty(), "no/such/dir");

    assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes());
    assert!(outcome.left_some() && outcome.moved == 0 && outcome.archive.is_none(), "{outcome:?}");
}

#[test]
fn an_archive_is_never_written_over() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    let taken = dir.0.join("config.yaml.before-1022-20261030T101500");
    std::fs::write(&taken, "someone's own file").unwrap();

    let outcome = migrate(&config, &FakeMail::empty(), STAMP);

    assert_eq!(read(&taken), "someone's own file");
    let archive = outcome.archive.unwrap();
    assert_ne!(archive, taken);
    assert!(read(&archive).contains(ADA_PASSWORD));
}

#[test]
fn a_second_pass_changes_nothing_and_asks_the_service_nothing() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    let mail = FakeMail::empty();
    migrate(&config, &mail, STAMP);
    let (text, calls) = (read(&config), mail.calls().len());

    let again = migrate(&config, &mail, "20261030T101600");

    assert_eq!(again, Outcome::default());
    assert_eq!(read(&config), text);
    assert_eq!(mail.calls().len(), calls);
    assert_eq!(dir.files().len(), 2, "no second archive");
}

#[test]
fn an_account_the_service_will_not_take_stays_in_the_file_and_is_named_without_its_password() {
    let dir = Scratch::new();
    let text = format!(
        "email:\n  accounts:\n    - name: Odd\n      email: odd@corp.example\n      provider: imap\n      password: {BOB_PASSWORD}\n    \
         - name: Ada\n      email: ada@example.com\n      provider: gmail\n      password: {ADA_PASSWORD}\n"
    );
    let config = write_config(&dir, &text);

    let outcome = migrate(&config, &FakeMail::empty(), STAMP);

    assert_eq!((outcome.moved, outcome.unmovable.clone()), (1, vec!["odd@corp.example".to_string()]));
    let after = read(&config);
    assert!(after.contains("odd@corp.example") && after.contains(BOB_PASSWORD), "the one not moved keeps its text");
    assert!(!after.contains(ADA_PASSWORD));
    let notice = outcome.notice().unwrap();
    assert!(notice.contains("odd@corp.example") && !notice.contains(BOB_PASSWORD), "{notice}");
}

#[test]
fn a_config_that_is_not_yaml_is_left_alone() {
    let dir = Scratch::new();
    let junk = "email: [unclosed\n  password: SENTINEL-junk\n";
    let config = write_config(&dir, junk);
    let outcome = migrate(&config, &FakeMail::empty(), STAMP);
    assert_eq!(outcome, Outcome::default());
    assert_eq!(read(&config), junk);
    assert_eq!(dir.files(), vec!["config.yaml"]);
}

// ── What the person is told, and what is never said ──────────────────────────────────────────

#[test]
fn the_notice_says_where_the_old_copy_is_and_that_it_holds_the_password() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    let outcome = migrate(&config, &FakeMail::empty(), STAMP);

    let notice = outcome.notice().expect("a notice");
    let archive = outcome.archive.as_ref().unwrap().display().to_string();
    assert!(
        notice.contains(&format!(
            "An older copy of your settings with your mail password in it is at {archive}; delete it when you're sure mail works."
        )),
        "{notice}"
    );
}

#[test]
fn a_pass_that_changed_nothing_has_no_notice() {
    assert_eq!(Outcome::default().notice(), None);
    let waiting = Outcome { waiting: 2, ..Outcome::default() };
    assert_eq!(waiting.notice(), None, "no archive, no mention of one");
}

#[test]
fn no_password_appears_in_an_outcome_a_notice_a_debug_or_the_calls_made() {
    let dir = Scratch::new();
    let config = write_config(&dir, &config_text());
    let mail = FakeMail::empty();
    let outcome = migrate(&config, &mail, STAMP);

    let said = format!("{outcome:?} {:?} {}", mail.calls(), outcome.notice().unwrap());
    for secret in [ADA_PASSWORD, BOB_PASSWORD] {
        assert!(!said.contains(secret), "{secret} leaked into: {said}");
    }
    let entry: serde_yaml::Value = serde_yaml::from_str(&format!("email: a@b.c\npassword: {ADA_PASSWORD}\n")).unwrap();
    let legacy = LegacyAccount::from_entry(&entry).unwrap();
    assert!(!format!("{legacy:?}").contains(ADA_PASSWORD));
    assert!(!format!("{:?}", legacy.to_import()).contains(ADA_PASSWORD));
}
