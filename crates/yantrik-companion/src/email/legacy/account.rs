//! One mail account as an older `config.yaml` kept it, secrets and all, and how it becomes an
//! import for `email-service`.
//!
//! This is the only place in the companion where a mail password is held, and only for the length
//! of one migration pass. It prints `<redacted>` for every secret, and a parse failure is reported
//! as a fixed sentence: a deserialiser's error can quote the text it could not read.

use serde::Deserialize;
use yantrik_ipc_contracts::email::ImportAccount;

#[derive(Clone, Deserialize)]
pub struct LegacyAccount {
    #[serde(default)]
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    imap_server: Option<String>,
    #[serde(default = "default_imap_port")]
    imap_port: u16,
    #[serde(default)]
    smtp_server: Option<String>,
    #[serde(default = "default_smtp_port")]
    smtp_port: u16,
    #[serde(default)]
    password: String,
    #[serde(default)]
    auth_method: Option<String>,
    #[serde(default)]
    oauth_access_token: Option<String>,
    #[serde(default)]
    oauth_refresh_token: Option<String>,
    #[serde(default)]
    oauth_token_expiry: Option<f64>,
}

fn default_imap_port() -> u16 {
    993
}

fn default_smtp_port() -> u16 {
    587
}

impl std::fmt::Debug for LegacyAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyAccount")
            .field("name", &self.name)
            .field("email", &self.email)
            .field("provider", &self.provider)
            .field("imap_server", &self.imap_server)
            .field("imap_port", &self.imap_port)
            .field("smtp_server", &self.smtp_server)
            .field("smtp_port", &self.smtp_port)
            .field("password", &"<redacted>")
            .field("auth_method", &self.auth_method)
            .field("oauth_access_token", &self.oauth_access_token.as_ref().map(|_| "<redacted>"))
            .field("oauth_refresh_token", &self.oauth_refresh_token.as_ref().map(|_| "<redacted>"))
            .field("oauth_token_expiry", &self.oauth_token_expiry)
            .finish()
    }
}

impl LegacyAccount {
    /// The account in one entry of the `accounts` list, or `None` when it is not one. The entry is
    /// left in the file either way.
    pub fn from_entry(entry: &serde_yaml::Value) -> Option<Self> {
        serde_yaml::from_value(entry.clone()).ok()
    }

    /// Every secret this entry holds, for checking that none is left in a file after a rewrite.
    pub fn secrets(&self) -> Vec<&str> {
        [Some(self.password.as_str()), self.oauth_access_token.as_deref(), self.oauth_refresh_token.as_deref()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Whether there is anything secret to move. An entry without one holds nothing worth hiding.
    pub fn has_secret(&self) -> bool {
        !self.secrets().is_empty()
    }

    /// The account as `email.import_account` takes it. Servers the entry left to its provider are
    /// filled in, as the companion's own client used to.
    pub fn to_import(&self) -> ImportAccount {
        let oauth = self.auth_method.as_deref() == Some("oauth2");
        let (imap_default, smtp_default) = provider_servers(&self.provider);
        ImportAccount {
            email: self.email.trim().to_string(),
            display_name: self.name.clone(),
            provider: self.provider.clone(),
            imap_server: self.imap_server.clone().filter(|s| !s.is_empty()).unwrap_or(imap_default.to_string()),
            imap_port: self.imap_port,
            smtp_server: self.smtp_server.clone().filter(|s| !s.is_empty()).unwrap_or(smtp_default.to_string()),
            smtp_port: self.smtp_port,
            password: if oauth { String::new() } else { self.password.clone() },
            oauth_access_token: self.oauth_access_token.clone().filter(|_| oauth),
            oauth_refresh_token: self.oauth_refresh_token.clone().filter(|_| oauth),
            oauth_expires_at: self.oauth_token_expiry.map(|t| t as i64).unwrap_or(0),
        }
    }
}

/// The IMAP and SMTP servers a provider name stands for. Empty for one that has none, which the
/// service then refuses as an account with no server.
fn provider_servers(provider: &str) -> (&'static str, &'static str) {
    match provider.to_lowercase().as_str() {
        "gmail" => ("imap.gmail.com", "smtp.gmail.com"),
        "outlook" => ("outlook.office365.com", "smtp.office365.com"),
        "yahoo" => ("imap.mail.yahoo.com", "smtp.mail.yahoo.com"),
        "icloud" => ("imap.mail.me.com", "smtp.mail.me.com"),
        _ => ("", ""),
    }
}
