//! What the credential vault holds for one mail account.
//!
//! One vault entry per account, `mail:<account id>`, holding a small JSON object: the password,
//! and for an account that signs in with Google its access token and refresh token. One entry and
//! not three, so that "the account's secrets" is kept and read back in one step and an account is
//! never half in the vault.
//!
//! Nothing here prints a secret. [`MailSecrets`] has a hand-written [`Debug`]; a vault entry that
//! cannot be read is reported as that, never as the text that failed to parse.

use crate::accounts::Account;
use yantrik_ipc_contracts::credentials::mail_id;
use yantrik_ipc_contracts::email::AccountSettings;

/// The vault id of an account's secrets.
pub fn vault_id(account_id: &str) -> String {
    mail_id(account_id)
}

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MailSecrets {
    #[serde(default)]
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_refresh_token: Option<String>,
}

// By hand, so a secret never reaches a log through `{:?}`.
impl std::fmt::Debug for MailSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailSecrets")
            .field("password", &"<redacted>")
            .field("oauth_token", &self.oauth_token.as_ref().map(|_| "<redacted>"))
            .field("oauth_refresh_token", &self.oauth_refresh_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl MailSecrets {
    /// What a typed account form carries.
    pub fn from_settings(settings: &AccountSettings) -> Self {
        Self { password: settings.password.clone(), oauth_token: None, oauth_refresh_token: None }
    }

    /// What an account holds in memory: a password it was read with, or tokens it signed in with.
    pub fn from_account(account: &Account) -> Self {
        Self {
            password: account.password.clone(),
            oauth_token: account.oauth_token.clone(),
            oauth_refresh_token: account.oauth_refresh_token.clone(),
        }
    }

    /// Google's tokens and no password.
    pub fn google(access_token: &str, refresh_token: &str) -> Self {
        Self {
            password: String::new(),
            oauth_token: Some(access_token.to_string()),
            oauth_refresh_token: Some(refresh_token.to_string()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.password.is_empty()
            && self.oauth_token.as_deref().is_none_or(str::is_empty)
            && self.oauth_refresh_token.as_deref().is_none_or(str::is_empty)
    }

    /// The text the vault keeps.
    pub fn to_json(&self) -> String {
        // Three strings cannot fail to serialise; an empty object would be read as "no secrets".
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The secrets in the vault's text, or `None` when it is not what this service wrote. The
    /// parse error is dropped: it can quote the text it could not read.
    pub fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    /// Put these on an account, for the length of a call to a mail server.
    pub fn fill(&self, account: &mut Account) {
        account.password = self.password.clone();
        account.oauth_token = self.oauth_token.clone();
        account.oauth_refresh_token = self.oauth_refresh_token.clone();
        // From the vault, not from the file: never to be written back.
        account.in_file = false;
    }
}
