//! The accounts file and the credential vault, kept in step.
//!
//! [`AccountStore`] is the only thing in this service that writes `email.json` or touches an
//! account's secrets, so the rules below hold for every caller and are tested here without a
//! socket, a mailbox or a shell:
//!
//! - **A secret is in the vault before the account that needs it is in the file**, and it is kept
//!   only when the vault reads it back the same (`yantrik_foundation::vault::keep`). A shut vault
//!   or one that fails leaves the file as it was.
//! - **The file never gains a secret.** `accounts::save` writes only what it read.
//! - **A secret is in this process only while a call needs it.** [`AccountStore::resolve`] fills a
//!   copy of the account for the length of one call; nothing caches it and nothing saves it.
//! - **An older service's clear-text password is moved** ([`AccountStore::migrate`]): copied into
//!   the vault, read back, and only then removed from the file, in one atomic rewrite. Accounts
//!   whose copy was refused keep their text exactly as it was, and the next pass tries again.
//! - **Removing an account forgets its vault entry first**, so a secret is not left behind by an
//!   account that is gone from the list.
//!
//! No error here carries a secret, and none quotes the vault's own words.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use yantrik_foundation::vault::{keep, KeyVault, VaultError};
use yantrik_ipc_contracts::credentials::ERR_LOCKED;
use yantrik_ipc_contracts::email::{EmailAccountSummary, ServiceError, WAITING_FOR_VAULT};

use crate::accounts::{self, Account};
use crate::secrets::{vault_id, MailSecrets};

/// An id nothing is ever kept under, read to ask the vault whether it is open.
const PROBE: &str = "probe";

/// What the person was doing when the vault was in the way, for the sentence that says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doing {
    Reading,
    Saving,
    Removing,
}

/// Why an account's secrets could not be had or kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The vault has a passphrase and is shut.
    Waiting(Doing),
    /// The vault did not answer, or is not there.
    Unavailable(Doing),
    /// The vault is open and holds nothing for this account.
    Missing { email: String, google: bool },
    /// The accounts file could not be read or written.
    File(String),
}

impl StoreError {
    fn from_vault(e: VaultError, doing: Doing) -> Self {
        match e {
            VaultError::Locked => StoreError::Waiting(doing),
            VaultError::Failed => StoreError::Unavailable(doing),
        }
    }

    /// The error as the wire carries it. A shut vault has a code of its own, so a caller can wait.
    pub fn into_service_error(self) -> ServiceError {
        let (code, message) = match self {
            StoreError::Waiting(Doing::Reading) => (ERR_LOCKED, WAITING_FOR_VAULT.to_string()),
            StoreError::Waiting(Doing::Saving) => (
                ERR_LOCKED,
                "Mail is waiting for the vault, so the account was not saved. Unlock it and save again.".to_string(),
            ),
            StoreError::Waiting(Doing::Removing) => (
                ERR_LOCKED,
                "Mail is waiting for the vault, so the account was not removed. Unlock it and remove it again."
                    .to_string(),
            ),
            StoreError::Unavailable(doing) => (
                -32000,
                format!(
                    "The credential vault did not answer, so {}. Try again in a moment.",
                    match doing {
                        Doing::Reading => "mail was not checked",
                        Doing::Saving => "the account was not saved",
                        Doing::Removing => "the account was not removed",
                    }
                ),
            ),
            StoreError::Missing { email, google: true } => (
                -32000,
                format!("Google sign-in expired \u{2014} sign in again. {email}'s sign-in is no longer in the credential vault."),
            ),
            StoreError::Missing { email, google: false } => (
                -32000,
                format!("The password for {email} is no longer in the credential vault. Enter it again in the account settings."),
            ),
            StoreError::File(message) => (-32000, message),
        };
        ServiceError { code, message }
    }
}

/// What a migration pass did. Counts only.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Accounts whose secrets are now in the vault and gone from the file.
    pub moved: usize,
    /// Accounts left as they were because the vault is shut.
    pub waiting: usize,
    /// Accounts left as they were because the vault or the file refused.
    pub failed: usize,
}

impl Migration {
    /// Whether the pass left secrets in the file, to be tried again.
    pub fn left_some(&self) -> bool {
        self.waiting + self.failed > 0
    }
}

/// The accounts file and the vault behind it. Cheap to clone: the clones share the lock.
#[derive(Clone)]
pub struct AccountStore {
    path: PathBuf,
    vault: Arc<dyn KeyVault>,
    /// Held across every change to the file and to the vault entry that goes with it. Reads go to
    /// the file: it is a few hundred bytes, it can be edited by hand while this is running, and a
    /// cached copy is how a service comes to insist an account exists that somebody deleted.
    writing: Arc<Mutex<()>>,
}

impl AccountStore {
    pub fn new(path: PathBuf, vault: Arc<dyn KeyVault>) -> Self {
        Self { path, vault, writing: Arc::new(Mutex::new(())) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writing.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The accounts, for a caller that only reads them.
    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        accounts::load(&self.path)
    }

    /// Whether the vault can be written to now. A save checks this before it spends a mailbox
    /// sign-in on an account it would have to throw away.
    pub fn check_open(&self, doing: Doing) -> Result<(), StoreError> {
        // Reading an id nothing is kept under: the answer is `None` when open, and the vault's
        // own word when it is not.
        self.vault.value(&vault_id(PROBE)).map(|_| ()).map_err(|e| StoreError::from_vault(e, doing))
    }

    /// Whether mail is waiting for the vault: there are accounts, and the vault is shut.
    pub fn waiting_for_vault(&self, accounts: &[Account]) -> bool {
        accounts.iter().any(|a| !a.from_env) && self.check_open(Doing::Reading) == Err(StoreError::Waiting(Doing::Reading))
    }

    /// `account` with its secrets in it, for the length of one call to a mail server.
    ///
    /// An account an older service wrote with its password in the file is moved first, and used
    /// from the vault like any other: while the vault is shut it is waiting, not working from the
    /// clear-text copy, so the answer is the same on the first day as on the hundredth.
    pub fn resolve(&self, account: Account) -> Result<Account, StoreError> {
        if account.from_env {
            return Ok(account);
        }
        let mut account = account;
        if account.held_in_file() {
            let pass = self.migrate();
            let fresh = accounts::load_file(&self.path).map_err(StoreError::File)?;
            match fresh.into_iter().find(|a| a.id == account.id) {
                Some(moved) if !moved.held_in_file() => account = moved,
                _ if pass.waiting > 0 => return Err(StoreError::Waiting(Doing::Reading)),
                _ => return Err(StoreError::Unavailable(Doing::Reading)),
            }
        }
        let google = account.use_oauth;
        let text = self
            .vault
            .value(&vault_id(&account.id))
            .map_err(|e| StoreError::from_vault(e, Doing::Reading))?;
        let secrets = text
            .as_deref()
            .and_then(MailSecrets::from_json)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| StoreError::Missing { email: account.email.clone(), google })?;
        secrets.fill(&mut account);
        Ok(account)
    }

    /// Keep the secrets typed into the form, then the account that goes with them.
    pub fn save_settings(
        &self,
        settings: &yantrik_ipc_contracts::email::AccountSettings,
    ) -> Result<EmailAccountSummary, StoreError> {
        let secrets = MailSecrets::from_settings(settings);
        self.save_with(&secrets, |all| accounts::upsert(all, settings))
    }

    /// Keep the tokens a Google sign-in issued, then the account that goes with them. A password
    /// the address had before is dropped from the vault entry: it is no longer used for anything.
    pub fn save_google(
        &self,
        email: &str,
        access_token: &str,
        refresh_token: &str,
        expires_at: i64,
    ) -> Result<EmailAccountSummary, StoreError> {
        let secrets = MailSecrets::google(access_token, refresh_token);
        self.save_with(&secrets, |all| accounts::upsert_google(all, email, access_token, refresh_token, expires_at))
    }

    fn save_with(
        &self,
        secrets: &MailSecrets,
        upsert: impl FnOnce(&mut Vec<Account>) -> String,
    ) -> Result<EmailAccountSummary, StoreError> {
        let _writing = self.lock();
        let mut all = accounts::load_file(&self.path).map_err(StoreError::File)?;
        let before: Vec<String> = all.iter().map(|a| a.id.clone()).collect();
        let id = upsert(&mut all);
        keep(&*self.vault, &vault_id(&id), &secrets.to_json())
            .map_err(|e| StoreError::from_vault(e, Doing::Saving))?;
        let written = accounts::save(&self.path, &all).and_then(|()| accounts::load_file(&self.path));
        match written {
            Ok(stored) => stored
                .iter()
                .find(|a| a.id == id)
                .map(Account::summary)
                .ok_or_else(|| StoreError::File(format!("the account was written to {} and is not in it", self.path.display()))),
            Err(message) => {
                // An account that was not there before has no business holding a secret.
                if !before.contains(&id) {
                    let _ = self.vault.remove(&vault_id(&id));
                }
                Err(StoreError::File(message))
            }
        }
    }

    /// A refreshed access token: kept in the vault with the account's other secrets, and the time
    /// it stops working in the file. `Ok(false)` when the account was removed meanwhile.
    pub fn store_refreshed(
        &self,
        account: &Account,
        access_token: &str,
        refresh_token: &str,
        expires_at: i64,
    ) -> Result<bool, StoreError> {
        let _writing = self.lock();
        let mut all = accounts::load_file(&self.path).map_err(StoreError::File)?;
        if !all.iter().any(|a| a.id == account.id) {
            return Ok(false);
        }
        let secrets = MailSecrets::google(access_token, refresh_token);
        keep(&*self.vault, &vault_id(&account.id), &secrets.to_json())
            .map_err(|e| StoreError::from_vault(e, Doing::Reading))?;
        accounts::store_expiry(&mut all, &account.id, expires_at);
        accounts::save(&self.path, &all).map_err(StoreError::File)?;
        Ok(true)
    }

    /// Forget the account called `wanted` and its vault entry. The vault goes first: if it is
    /// shut the account stays, because an account gone from the list leaves no way to find the
    /// entry again. `Ok(false)` when there is no such account.
    pub fn remove(&self, wanted: &str) -> Result<bool, StoreError> {
        let _writing = self.lock();
        let mut all = accounts::load_file(&self.path).map_err(StoreError::File)?;
        let Some(gone) = accounts::take(&mut all, wanted) else { return Ok(false) };
        self.vault.remove(&vault_id(&gone.id)).map_err(|e| StoreError::from_vault(e, Doing::Removing))?;
        accounts::save(&self.path, &all).map_err(StoreError::File)?;
        Ok(true)
    }

    /// Move the secrets an older service left in the file into the vault.
    ///
    /// - Each account's secrets are stored and read back; only an account whose copy reads back
    ///   equal is cleared in the file.
    /// - The file is rewritten once, atomically, and only if the vault took at least one account.
    ///   A vault that is shut, refuses or garbles leaves the file exactly as it was; so does a
    ///   rewrite that fails.
    /// - Idempotent: with nothing left in the file it does nothing, and asks the vault nothing.
    pub fn migrate(&self) -> Migration {
        let _writing = self.lock();
        let mut report = Migration::default();
        let Ok(mut all) = accounts::load_file(&self.path) else { return report };
        let mut verified: Vec<String> = Vec::new();
        for account in all.iter().filter(|a| a.held_in_file()) {
            let kept = keep(&*self.vault, &vault_id(&account.id), &MailSecrets::from_account(account).to_json());
            match kept {
                Ok(()) => verified.push(account.id.clone()),
                Err(VaultError::Locked) => report.waiting += 1,
                Err(VaultError::Failed) => report.failed += 1,
            }
        }
        if verified.is_empty() {
            return report;
        }
        for account in all.iter_mut().filter(|a| verified.contains(&a.id)) {
            account.clear_secrets();
        }
        match accounts::save(&self.path, &all) {
            Ok(()) => report.moved = verified.len(),
            Err(_) => report.failed += verified.len(),
        }
        report
    }
}
