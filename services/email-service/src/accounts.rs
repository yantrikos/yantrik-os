//! Where mail accounts are kept, and what is kept in them.
//!
//! This module is the whole of the service's account model and it touches no network, so the
//! rules below can be tested on a machine with no mailbox — which is every machine this is
//! developed on. `tests/email-core` includes this file directly.
//!
//! ## The file holds servers, not secrets
//!
//! `email.json` is the account list: addresses, servers, ports, and when an OAuth access token
//! stops working. An account's password and its OAuth tokens are in the credential vault, under
//! `mail:<account id>` (`secrets.rs`, `store.rs`), and reach this process only for as long as a
//! call to a mail server takes. [`save`] writes no secret that was not already in the file when it
//! was read: an [`Account`] built in memory (a form, a Google sign-in, a resolved account) has
//! [`Account::in_file`] false, and [`Account::stored`] leaves its secrets out.
//!
//! The one way a secret is in the file is an account an older service wrote with its password in
//! clear text. [`load`] marks it, [`save`] leaves it exactly as it was until `store::migrate` has
//! copied it into the vault, read it back and cleared it, and `email.accounts` says
//! `secrets_are_plaintext` for as long as that is so.

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::email::{AccountSettings, EmailAccountSummary};

/// One configured account, as the service holds it.
///
/// [`Debug`] is written out rather than derived, for the reason [`AccountSettings`] does the
/// same: a derived one prints the password into every `{:?}`, and a tracing line is a file on
/// disk that outlives the session.
#[derive(Clone, Default, serde::Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub password: String,
    pub imap_server: String,
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    pub smtp_server: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub use_oauth: bool,
    /// The OAuth2 access token, which is what XOAUTH2 actually signs in with. Short-lived:
    /// Google's last an hour.
    #[serde(default)]
    pub oauth_token: Option<String>,
    /// What a new access token is obtained with when the one above has expired.
    ///
    /// This is the credential that matters on an OAuth account — an access token is an hour of
    /// access and this is all of it, until it is revoked. It is in the same clear-text file as
    /// the passwords, for the same reason and with the same 0600, and the decision about a
    /// secret store in `design/email-2026-09-20.md` covers it.
    #[serde(default)]
    pub oauth_refresh_token: Option<String>,
    /// Unix seconds at which [`Account::oauth_token`] stops working.
    ///
    /// Absolute rather than a lifetime, because it is written to a file and read back by another
    /// process minutes or days later. `None` on an account written before this service stored
    /// one, and `None` is treated as expired: refreshing a token that was still good costs one
    /// HTTPS round trip, and not refreshing one that was not costs a sign-in failure that reads
    /// as a broken account.
    #[serde(default)]
    pub oauth_expires_at: Option<i64>,
    /// This account came from the environment (`YANTRIK_EMAIL_*`), not from the file. Its secret
    /// is in this process already, it is never saved, and there is nothing to move.
    #[serde(skip)]
    pub from_env: bool,
    /// Whatever secret this account holds was read from the accounts file and is still there: an
    /// older service's clear-text password. False for every account built in memory, so [`save`]
    /// can never write one of those secrets down.
    #[serde(skip)]
    pub in_file: bool,
}

/// What [`save`] writes for one account: [`Account`] without a secret, unless the secret is one
/// that was read out of this file and has not been moved yet.
#[derive(serde::Serialize)]
struct Stored<'a> {
    id: &'a str,
    email: &'a str,
    display_name: &'a str,
    provider: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    password: &'a str,
    imap_server: &'a str,
    imap_port: u16,
    smtp_server: &'a str,
    smtp_port: u16,
    use_oauth: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth_token: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth_refresh_token: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth_expires_at: Option<i64>,
}

fn default_imap_port() -> u16 {
    993
}

fn default_smtp_port() -> u16 {
    587
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("provider", &self.provider)
            .field("imap_server", &self.imap_server)
            .field("imap_port", &self.imap_port)
            .field("smtp_server", &self.smtp_server)
            .field("smtp_port", &self.smtp_port)
            .field("use_oauth", &self.use_oauth)
            .field("password", &"<redacted>")
            .field("oauth_token", &self.oauth_token.as_ref().map(|_| "<redacted>"))
            .field(
                "oauth_refresh_token",
                &self.oauth_refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("oauth_expires_at", &self.oauth_expires_at)
            .field("in_file", &self.in_file)
            .finish()
    }
}

/// A token as the file may hold it: only one that was read from the file.
fn only_if_legacy(legacy: bool, secret: &Option<String>) -> Option<&str> {
    secret.as_deref().filter(|s| legacy && !s.is_empty())
}

impl Account {
    /// The account as the file holds it. A secret is written only when it was read from the file:
    /// anything this process learned since (a typed password, a token Google issued, a refreshed
    /// one) belongs to the vault.
    fn stored(&self) -> Stored<'_> {
        let legacy = self.in_file;
        Stored {
            id: &self.id,
            email: &self.email,
            display_name: &self.display_name,
            provider: &self.provider,
            password: if legacy { &self.password } else { "" },
            imap_server: &self.imap_server,
            imap_port: self.imap_port,
            smtp_server: &self.smtp_server,
            smtp_port: self.smtp_port,
            use_oauth: self.use_oauth,
            oauth_token: only_if_legacy(legacy, &self.oauth_token),
            oauth_refresh_token: only_if_legacy(legacy, &self.oauth_refresh_token),
            oauth_expires_at: self.oauth_expires_at,
        }
    }

    /// Whether a secret of this account is in the accounts file now, in clear text.
    pub fn held_in_file(&self) -> bool {
        self.in_file && !self.from_env && self.secrets().iter().any(|s| !s.is_empty())
    }

    /// Forget the secrets this account holds in memory, once the vault has them.
    pub fn clear_secrets(&mut self) {
        self.password.clear();
        self.oauth_token = None;
        self.oauth_refresh_token = None;
    }

    /// Every string on this account that must never reach a screen, a log or `describe`.
    ///
    /// A password was the only one until Google sign-in. An OAuth account has two more, and a
    /// mail server is free to quote back the AUTHENTICATE line it was sent — so the sentence
    /// built from a refusal is cleared of all of them rather than of whichever one the call site
    /// remembered. Empty strings are harmless: `without_secret` ignores them.
    pub fn secrets(&self) -> Vec<&str> {
        vec![
            self.password.as_str(),
            self.oauth_token.as_deref().unwrap_or(""),
            self.oauth_refresh_token.as_deref().unwrap_or(""),
        ]
    }

    /// What may leave this process about this account. No password, no token.
    pub fn summary(&self) -> EmailAccountSummary {
        EmailAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            display_name: self.display_name.clone(),
            provider: self.provider.clone(),
            imap_server: self.imap_server.clone(),
            imap_port: self.imap_port,
            smtp_server: self.smtp_server.clone(),
            smtp_port: self.smtp_port,
            uses_oauth: self.use_oauth,
        }
    }
}

/// Every account's summary, in the order they are stored.
pub fn summaries(accounts: &[Account]) -> Vec<EmailAccountSummary> {
    accounts.iter().map(Account::summary).collect()
}

/// The file the service reads accounts from.
pub fn config_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("YANTRIK_EMAIL_CONFIG") {
        return PathBuf::from(explicit);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config/yantrik/email.json")
}

/// The account a request is about.
///
/// `requested` is whatever the caller put in `account_id`, which for most of this service's
/// history was nothing at all — so the handler substituted the literal string `default` and
/// looked for an account with that id. An account saved under any other id was then unreachable,
/// and the service answered `Unknown account: default`, which the app rendered as having no
/// account configured. So: an id that names an account wins; anything else, including the old
/// `default` and an absent argument, means the first account there is.
pub fn pick<'a>(accounts: &'a [Account], requested: Option<&str>) -> Option<&'a Account> {
    let want = requested.unwrap_or("").trim();
    if !want.is_empty() && want != "default" {
        if let Some(found) = accounts
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(want) || a.email.eq_ignore_ascii_case(want))
        {
            return Some(found);
        }
    }
    accounts.first()
}

/// A stable id for an account, derived from its address.
///
/// Derived rather than counted, so re-saving the same address edits that account instead of
/// piling up `account-1`, `account-2` beside it.
pub fn id_for(email: &str) -> String {
    let slug: String = email
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "account".to_string()
    } else {
        trimmed
    }
}

/// Read the accounts file. A file that is not there is no accounts, not an error — that is the
/// ordinary state of a machine nobody has configured yet, and it is the state the app has to be
/// able to tell apart from a service it could not reach.
///
/// A file that is there and unreadable *is* an error: silently reporting "no account" for a
/// mailbox whose config has a typo in it is how this app came to show an onboarding screen to
/// someone who already had an account.
pub fn load(path: &Path) -> Result<Vec<Account>, String> {
    let from_file = load_file(path)?;
    if from_file.is_empty() {
        return Ok(from_env());
    }
    Ok(from_file)
}

/// The accounts the file holds, and only those: none when it is missing or empty, and never the
/// environment's. What a change to the list starts from, so that an account that exists only in
/// the environment is not written into the file by a change to another.
pub fn load_file(path: &Path) -> Result<Vec<Account>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut accounts: Vec<Account> = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a list of accounts: {e}", path.display()))?;
    for account in &mut accounts {
        if account.id.trim().is_empty() {
            account.id = id_for(&account.email);
        }
        // Whatever secret it holds, it read from this file.
        account.in_file = true;
    }
    Ok(accounts)
}

/// The single-account escape hatch that predates the config file, kept because deployments use
/// it. Returns nothing unless all three variables are set, so a half-set environment is not read
/// as an account whose server is the empty string.
pub fn from_env() -> Vec<Account> {
    let (Ok(email), Ok(password), Ok(imap)) = (
        std::env::var("YANTRIK_EMAIL"),
        std::env::var("YANTRIK_EMAIL_PASSWORD"),
        std::env::var("YANTRIK_EMAIL_IMAP"),
    ) else {
        return Vec::new();
    };
    let smtp =
        std::env::var("YANTRIK_EMAIL_SMTP").unwrap_or_else(|_| imap.replace("imap.", "smtp."));
    vec![Account {
        id: id_for(&email),
        email,
        display_name: String::new(),
        provider: "advanced".to_string(),
        password,
        imap_server: imap,
        imap_port: 993,
        smtp_server: smtp,
        smtp_port: 587,
        use_oauth: false,
        oauth_token: None,
        oauth_refresh_token: None,
        oauth_expires_at: None,
        from_env: true,
        in_file: false,
    }]
}

/// Put `settings` in the list, replacing an account with the same id.
///
/// Returns the id it was stored under, so the caller answers with what it did rather than with
/// what it was asked to do.
pub fn upsert(accounts: &mut Vec<Account>, settings: &AccountSettings) -> String {
    let id = id_for(&settings.email);
    let account = Account {
        id: id.clone(),
        email: settings.email.trim().to_string(),
        display_name: settings.display_name.trim().to_string(),
        provider: settings.provider.trim().to_string(),
        password: settings.password.clone(),
        imap_server: settings.imap_server.trim().to_string(),
        imap_port: settings.imap_port,
        smtp_server: settings.smtp_server.trim().to_string(),
        smtp_port: settings.smtp_port,
        use_oauth: false,
        oauth_token: None,
        oauth_refresh_token: None,
        oauth_expires_at: None,
        from_env: false,
        in_file: false,
    };
    match accounts.iter().position(|a| a.id == id) {
        Some(at) => accounts[at] = account,
        None => accounts.push(account),
    }
    id
}

/// Gmail's servers, which are the only ones a Google sign-in can be for.
///
/// Spelled here rather than taken from the app, because nothing was typed into a form for this
/// flow: the address came from Google and the servers follow from that. Port 587 is submission
/// with STARTTLS, which is what `smtp_transport` reads the port to mean.
pub const GOOGLE_SERVERS: (&str, u16, &str, u16) = ("imap.gmail.com", 993, "smtp.gmail.com", 587);

/// Put a Google-signed-in account in the list, replacing one with the same address.
///
/// The display name is left as whatever was already stored for this address, if anything: a
/// sign-in does not know what the person calls this account, and overwriting a name they typed
/// with an empty string is a change nobody asked for.
///
/// Returns the id, like [`upsert`], so the caller answers with what it did.
pub fn upsert_google(
    accounts: &mut Vec<Account>,
    email: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
) -> String {
    let id = id_for(email);
    let (imap_server, imap_port, smtp_server, smtp_port) = GOOGLE_SERVERS;
    let display_name = accounts
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.display_name.clone())
        .unwrap_or_default();
    let account = Account {
        id: id.clone(),
        email: email.trim().to_string(),
        display_name,
        provider: "gmail".to_string(),
        // Deliberately cleared. An address that had an App Password and now signs in with Google
        // should not keep the password lying in the file: it is no longer used for anything, and
        // a credential kept after it stops being needed is a credential kept for no reason.
        password: String::new(),
        imap_server: imap_server.to_string(),
        imap_port,
        smtp_server: smtp_server.to_string(),
        smtp_port,
        use_oauth: true,
        oauth_token: Some(access_token.to_string()),
        oauth_refresh_token: Some(refresh_token.to_string()),
        oauth_expires_at: Some(expires_at),
        from_env: false,
        in_file: false,
    };
    match accounts.iter().position(|a| a.id == id) {
        Some(at) => accounts[at] = account,
        None => accounts.push(account),
    }
    id
}

/// Write when a refreshed access token stops working onto the stored account.
///
/// The token itself is not written: it goes to the credential vault with the rest of the
/// account's secrets (`store::AccountStore::store_refreshed`), and the file keeps only the time.
/// Separate from [`upsert_google`] because a refresh must not touch anything else: it happens on
/// the way to opening a mailbox, with no person watching, and a function that rebuilt the whole
/// record would be one edit away from resetting a display name or a server on every poll.
///
/// Returns false when the address is not in the list any more — somebody removed the account
/// while a mail call was in flight — so the caller can say that rather than report a write that
/// did not happen.
pub fn store_expiry(accounts: &mut [Account], id: &str, expires_at: i64) -> bool {
    match accounts.iter_mut().find(|a| a.id == id) {
        Some(account) => {
            account.oauth_expires_at = Some(expires_at);
            true
        }
        None => false,
    }
}

/// Take the account called `wanted` (its id or its address, exactly, with no fallback to "the
/// first one": this is for a removal) out of the list, and hand it back.
pub fn take(accounts: &mut Vec<Account>, wanted: &str) -> Option<Account> {
    let wanted = wanted.trim();
    let at = accounts
        .iter()
        .position(|a| a.id.eq_ignore_ascii_case(wanted) || a.email.eq_ignore_ascii_case(wanted))?;
    Some(accounts.remove(at))
}

/// Write the accounts file, and the directory above it, so that nobody but this user can read
/// either.
///
/// Whole or not at all: the new text goes to a file of its own, mode 0600 from the moment it is
/// made, is synced, and replaces the old file in one rename (`yantrik_foundation::json_store`). A
/// crash or a full disk leaves the file as it was, which is what a migration that fails depends
/// on. What is written is [`Account::stored`]: no secret that is not already in this file.
pub fn save(path: &Path, accounts: &[Account]) -> Result<(), String> {
    let stored: Vec<Stored<'_>> = accounts.iter().map(Account::stored).collect();
    let text = serde_json::to_string_pretty(&stored)
        .map_err(|e| format!("could not serialise the accounts: {e}"))?;

    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not make {}: {e}", dir.display()))?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    yantrik_foundation::json_store::write_atomically(path, text.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// What is wrong with these settings, before anything is tried against a server.
///
/// The service checks as well as the window, because the window is not the only thing that can
/// call this and a saved account with an empty server is an account that fails on every later
/// call with a message about the server rather than about the setup.
pub fn refuse_bad_settings(settings: &AccountSettings) -> Result<(), String> {
    if !settings.email.contains('@') {
        return Err(format!("`{}` is not an email address", settings.email.trim()));
    }
    if settings.password.is_empty() {
        return Err("no password was given".to_string());
    }
    if settings.imap_server.trim().is_empty() {
        return Err("no IMAP server was given".to_string());
    }
    if settings.smtp_server.trim().is_empty() {
        return Err("no SMTP server was given".to_string());
    }
    if settings.imap_port == 0 || settings.smtp_port == 0 {
        return Err("a port of 0 is not a port".to_string());
    }
    Ok(())
}
