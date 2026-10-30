//! Moving the mail accounts an older companion kept in `config.yaml` (#1022).
//!
//! Older versions kept `email.accounts` in the config, each with its password in clear text, and
//! ran their own IMAP and SMTP clients with them. There is one mail stack now: `email-service`
//! owns the accounts and the credential vault holds their secrets. A pass of [`migrate`], per
//! account in the file:
//!
//! - **The address is already an account in the service:** nothing is added or changed there, and
//!   the entry, password and all, leaves the file.
//! - **It is not:** the service creates it and keeps its secret in the vault as `mail:<account id>`,
//!   reading it back before it answers (the same path the Email app's own save takes). Only then
//!   does the entry leave the file.
//! - **The vault is shut or fails:** the file stays exactly as it was and the caller asks again
//!   when the vault opens, as the saved-provider and mail-service migrations do.
//! - **An entry with no secret** holds nothing worth hiding and is left where it is.
//!
//! The person's config is never lost. Before the file is rewritten the original is archived as
//! `config.yaml.before-1022-<stamp>` (mode 0600, the file's owner) and is never deleted here: it
//! holds the old password, and [`Outcome::notice`] tells the person where it is so they can delete
//! it when they are sure mail works. The rewrite is atomic where the config's directory takes a new
//! file; on an installed machine, where `/opt/yantrik` is root's and only `config.yaml` is the
//! person's, the archive goes to their state directory and the file is written in place. A
//! `config.yaml` that is a link, or has a second name, is left alone and the person is told: the
//! password would stay in the file behind the link. No password is echoed to a log, a
//! notice or an error; the pass reports counts and addresses only.
//!
//! Rewriting goes through a YAML tree, so comments in `config.yaml` do not survive the rewrite.
//! They are in the archive.

mod account;
mod file;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde_yaml::Value;

pub use account::LegacyAccount;
pub use file::Refusal;
use super::port::{is_locked, MailPort};

/// What one pass did. Counts and addresses, never a secret.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Entries now in the service (new) and gone from the file.
    pub moved: usize,
    /// Of those, ones whose address the service already had.
    pub already_there: usize,
    /// Entries left in the file because the vault is shut.
    pub waiting: usize,
    /// Entries left in the file because something refused them or failed. Asked again later.
    pub failed: usize,
    /// Addresses of entries the service will not take as they are (no server, not an address).
    pub unmovable: Vec<String>,
    /// Where the original `config.yaml` was archived, when this pass rewrote the file.
    pub archive: Option<PathBuf>,
    /// Accounts the service now has whose entry could not be taken out of the file: the password
    /// is still there, and the person is told so.
    pub uncleaned: usize,
    /// Why the file was not touched at all: it is a link, or has a second name.
    pub blocked: Option<Refusal>,
}

impl Outcome {
    /// Whether secrets are left in the file to be asked about again.
    pub fn left_some(&self) -> bool {
        self.waiting + self.failed > 0
    }

    /// What to tell the person, when there is something to tell. Names the archive, which holds
    /// the old password, and never quotes it.
    pub fn notice(&self) -> Option<String> {
        if let Some(why) = self.blocked {
            return Some(format!(
                "Your mail password is still in config.yaml: it was left alone because {}, and a rewrite would not reach the file behind it. Move it into the Email app's account settings, then remove the mail section from config.yaml.",
                why.says()
            ));
        }
        if self.uncleaned > 0 {
            let copy = self
                .archive
                .as_ref()
                .map(|a| format!(" An older copy of your settings with your mail password in it is at {}; delete it when you're sure mail works.", a.display()))
                .unwrap_or_default();
            return Some(format!(
                "Your mail accounts are in the mail service now, but config.yaml could not be cleaned, so your mail password is still in it. Yantrik will keep trying.{copy}"
            ));
        }
        let archive = self.archive.as_ref()?;
        let mut text = format!(
            "Your mail accounts moved out of config.yaml into the mail service, and their passwords into the credential vault. An older copy of your settings with your mail password in it is at {}; delete it when you're sure mail works.",
            archive.display()
        );
        if !self.unmovable.is_empty() {
            text.push_str(&format!(
                " These could not be moved and are still in config.yaml: {}.",
                self.unmovable.join(", ")
            ));
        }
        Some(text)
    }
}

/// `email.accounts` of a parsed config, if it has any.
fn accounts_of(root: &Value) -> Option<&Vec<Value>> {
    root.get("email")?.get("accounts")?.as_sequence().filter(|s| !s.is_empty())
}

/// The config without the entries at `gone`; the `accounts` key goes with its last entry.
fn without(mut root: Value, gone: &[usize]) -> Value {
    if let Some(email) = root.get_mut("email").and_then(Value::as_mapping_mut) {
        let key = Value::String("accounts".to_string());
        let kept: Vec<Value> = email
            .get(&key)
            .and_then(Value::as_sequence)
            .map(|all| all.iter().enumerate().filter(|(i, _)| !gone.contains(i)).map(|(_, v)| v.clone()).collect())
            .unwrap_or_default();
        if kept.is_empty() {
            email.remove(&key);
        } else {
            email.insert(key, Value::Sequence(kept));
        }
    }
    root
}

/// One pass over `config`. `stamp` names the archive (the caller supplies the clock).
///
/// Safe to call again and again: with nothing left in the file it reads the file and does nothing
/// else, and asks the service nothing.
pub fn migrate(config: &Path, mail: &dyn MailPort, stamp: &str) -> Outcome {
    migrate_in(config, mail, stamp, &yantrik_foundation::xdg::state_home().join("yantrik"))
}

/// [`migrate`], with the directory an archive goes to when the config's own takes no new file.
pub fn migrate_in(config: &Path, mail: &dyn MailPort, stamp: &str, fallback: &Path) -> Outcome {
    let mut outcome = Outcome::default();
    let Ok(original) = std::fs::read(config) else { return outcome };
    // An unreadable file is left alone. The parse error is not logged: it can quote the text.
    let Ok(root) = serde_yaml::from_slice::<Value>(&original) else {
        tracing::warn!("config.yaml could not be read as YAML, so its mail accounts were not moved");
        return outcome;
    };
    let Some(entries) = accounts_of(&root) else { return outcome };
    // Before anything is copied anywhere: a file that cannot be cleaned is not imported from.
    let holds_a_secret = entries.iter().filter_map(LegacyAccount::from_entry).any(|a| a.has_secret());
    if let (true, Err(why)) = (holds_a_secret, file::check_target(config)) {
        outcome.blocked = Some(why);
        return outcome;
    }

    let mut gone: Vec<usize> = Vec::new();
    let mut secrets: Vec<String> = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        let Some(account) = LegacyAccount::from_entry(entry) else {
            outcome.failed += 1;
            continue;
        };
        if !account.has_secret() {
            continue;
        }
        match mail.import_account(&account.to_import()) {
            Ok(result) => {
                gone.push(at);
                secrets.extend(account.secrets().into_iter().map(str::to_string));
                if result.created {
                    outcome.moved += 1;
                } else {
                    outcome.already_there += 1;
                }
            }
            Err(e) if is_locked(&e) => {
                // Every other account would meet the same shut vault.
                outcome.waiting += entries.len() - at;
                break;
            }
            Err(e) if e.code == -32602 => outcome.unmovable.push(account.email.trim().to_string()),
            Err(_) => outcome.failed += 1,
        }
    }
    if gone.is_empty() {
        return outcome;
    }

    // The original is safe before the file changes. If it cannot be kept, nothing changes.
    let archive = match file::archive(config, &original, stamp, fallback) {
        Ok(path) => path,
        Err(_) => {
            tracing::error!("config.yaml could not be archived, so its mail accounts were left in it");
            outcome.failed += gone.len();
            outcome.uncleaned = gone.len();
            outcome.moved = 0;
            outcome.already_there = 0;
            return outcome;
        }
    };
    let rewritten = serde_yaml::to_string(&without(root, &gone))
        .map_err(|_| ())
        .and_then(|text| file::rewrite(config, &text).map_err(|_| ()));
    if rewritten.is_err() {
        // The service has the accounts, so the next pass drops them from the file.
        tracing::error!("config.yaml could not be rewritten, so its mail accounts are still in it");
        outcome.failed += gone.len();
        outcome.uncleaned = gone.len();
        outcome.moved = 0;
        outcome.already_there = 0;
        outcome.archive = Some(archive);
        return outcome;
    }
    let left = std::fs::read_to_string(config).unwrap_or_default();
    if secrets.iter().any(|s| left.contains(s.as_str())) {
        tracing::error!("config.yaml was not cleaned by the rewrite");
    }
    outcome.archive = Some(archive);
    outcome
}

/// A stamp for the archive's name: `20261030T141500`, local time.
pub fn stamp_now() -> String {
    chrono::Local::now().format("%Y%m%dT%H%M%S").to_string()
}

/// One pass as the shell runs it, logged by counts. `true` while secrets are left in the file.
/// Returns the notice for the person when the pass changed the file.
pub fn pass(config: &Path, mail: &dyn MailPort, when: &str) -> (bool, Option<String>) {
    let outcome = migrate(config, mail, &stamp_now());
    if outcome != Outcome::default() {
        tracing::info!(
            moved = outcome.moved,
            already_in_the_service = outcome.already_there,
            waiting_for_the_vault = outcome.waiting,
            failed = outcome.failed,
            unmovable = outcome.unmovable.len(),
            uncleaned = outcome.uncleaned,
            blocked = outcome.blocked.is_some(),
            archived = outcome.archive.is_some(),
            when,
            "Mail accounts moved out of config.yaml"
        );
    }
    (outcome.left_some(), outcome.notice())
}
