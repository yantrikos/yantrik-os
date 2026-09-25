//! Email service contract — IMAP/SMTP operations.

use serde::{Deserialize, Serialize};

/// The names of the email service's JSON-RPC methods.
///
/// Here rather than spelled out at each call site, for the reason the calendar's are: the two
/// ends of a wire that each spell their own strings drift apart while both files look correct on
/// their own page.
pub mod method {
    pub const LIST_FOLDERS: &str = "email.list_folders";
    pub const LIST_MESSAGES: &str = "email.list_messages";
    pub const GET_MESSAGE: &str = "email.get_message";
    pub const SEND_MESSAGE: &str = "email.send_message";
    pub const MARK_READ: &str = "email.mark_read";
    pub const MARK_STARRED: &str = "email.mark_starred";
    pub const MOVE_MESSAGE: &str = "email.move_message";
    pub const DELETE_MESSAGE: &str = "email.delete_message";
    pub const SEARCH: &str = "email.search";
    /// What accounts are configured, and where they are kept. Answers without touching a mail
    /// server, so a caller can tell "no account" from "the mail server is unreachable".
    pub const ACCOUNTS: &str = "email.accounts";
    /// Try the supplied settings against IMAP and SMTP, store nothing.
    pub const TEST_ACCOUNT: &str = "email.test_account";
    /// Store an account the service will read from then on.
    pub const SAVE_ACCOUNT: &str = "email.save_account";
    /// Start a Google sign-in: answers with the consent page to open, and a flow id.
    ///
    /// Three methods rather than one, because this flow cannot be a request and a reply. The
    /// person leaves for a browser in the middle of it, and the service has to sit on a loopback
    /// socket meanwhile — so `oauth_begin` returns immediately with somewhere to send them,
    /// [`OAUTH_STATUS`] is asked afterwards, and [`OAUTH_CANCEL`] is what a Cancel button calls.
    /// A single blocking method would hold the app's ten-second budget for the minutes a person
    /// takes to choose an account, and the window would be frozen for all of them.
    pub const OAUTH_BEGIN: &str = "email.oauth_begin";
    /// Where a started sign-in has got to: waiting, done, or failed with a reason.
    pub const OAUTH_STATUS: &str = "email.oauth_status";
    /// Give up on a started sign-in and stop the socket that is waiting for it.
    pub const OAUTH_CANCEL: &str = "email.oauth_cancel";
}

/// What the service will say about a configured account. There is no password on it, and there
/// must never be: this travels to a window, into `app.describe`, and from there to whatever is
/// reading the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub provider: String,
    pub imap_server: String,
    pub imap_port: u16,
    pub smtp_server: String,
    pub smtp_port: u16,
    /// True when the account signs in with an OAuth2 token rather than a password.
    #[serde(default)]
    pub uses_oauth: bool,
}

/// The answer to [`method::ACCOUNTS`].
///
/// `config_path` and `secrets_are_plaintext` are here because the app has to be able to say, in
/// words, where an account lives and what is done with its password. An app that asks a person
/// for a credential and will not say where it puts it is asking them to trust a black box.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountsResult {
    pub accounts: Vec<EmailAccountSummary>,
    /// The file the service reads accounts from, absolute.
    pub config_path: String,
    /// True while passwords are kept in that file rather than in a secret store.
    pub secrets_are_plaintext: bool,
    /// Whether this build can start a Google sign-in at all, and what to say when it cannot.
    ///
    /// The screen has to know before it draws the button, because a "Sign in with Google" that
    /// cannot work is the dead control this whole flow replaces. `#[serde(default)]` so that an
    /// older service answering without it parses as "not available", which is the truth about an
    /// older service.
    #[serde(default)]
    pub google_sign_in: GoogleSignIn,
}

/// Whether a Google sign-in can be started on this machine, and why not when it cannot.
///
/// `note` is written for a person to read on the setup screen, not for a log. When Google
/// sign-in is unavailable it is the whole of the explanation — including what to do instead —
/// because an unexplained missing button is indistinguishable from a broken one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleSignIn {
    pub available: bool,
    pub note: String,
}

/// Everything needed to sign in to one account, including the password.
///
/// This is the one type in this contract that carries a secret, and it travels in one direction
/// only: from the window a person typed into, to the service. Nothing answers with it.
///
/// [`Debug`] is written by hand below and redacts the password, because the derived one would
/// print it into any `{:?}` — a tracing line, a panic message, an error built with `format!` —
/// and every one of those ends up somewhere a person or a mind can read.
#[derive(Clone, Serialize, Deserialize)]
pub struct AccountSettings {
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    /// `gmail`, `outlook`, `yahoo`, `icloud`, or `advanced` for a hand-entered server.
    #[serde(default)]
    pub provider: String,
    pub imap_server: String,
    pub imap_port: u16,
    pub smtp_server: String,
    pub smtp_port: u16,
    pub password: String,
}

impl std::fmt::Debug for AccountSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountSettings")
            .field("email", &self.email)
            .field("display_name", &self.display_name)
            .field("provider", &self.provider)
            .field("imap_server", &self.imap_server)
            .field("imap_port", &self.imap_port)
            .field("smtp_server", &self.smtp_server)
            .field("smtp_port", &self.smtp_port)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// The answer to [`method::TEST_ACCOUNT`]: what each half of the connection did.
///
/// Both halves are reported rather than the first failure, because an account whose IMAP works
/// and whose SMTP does not can read mail and cannot send it, and a person given one word for
/// both learns neither.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAccountResult {
    pub imap_ok: bool,
    /// What happened, named: the server's own answer, or the failure this was classified as.
    pub imap: String,
    pub smtp_ok: bool,
    pub smtp: String,
}

/// The answer to [`method::OAUTH_BEGIN`].
///
/// There is no secret on it. `auth_url` carries the client id — which is public by construction
/// in a desktop OAuth client, since it ships inside the binary — and the PKCE *challenge*, which
/// is a hash and is meant to be seen. The verifier behind it never leaves the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthBeginResult {
    /// What [`method::OAUTH_STATUS`] and [`method::OAUTH_CANCEL`] are asked about.
    pub flow_id: String,
    /// The Google consent page to open in a browser.
    pub auth_url: String,
    /// How long the service will keep the loopback socket open waiting for the browser to come
    /// back. Said out loud so the screen can promise the person the same number.
    pub expires_in_secs: u64,
}

/// The answer to [`method::OAUTH_STATUS`]: where a started sign-in has got to.
///
/// `Failed` carries the reason in words, for the same rule the rest of this contract follows —
/// a sign-in that did not happen and a sign-in that was declined are different things and a
/// person can act on the difference. No token is ever on this type: what a completed flow
/// answers with is the same [`EmailAccountSummary`] that `save_account` answers with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OAuthStatus {
    /// The browser has not come back yet.
    Waiting,
    /// Signed in, verified against IMAP, and written to the accounts file.
    Done { account: EmailAccountSummary },
    Failed { reason: String },
}

/// Take `secret` out of text that is about to be shown to someone.
///
/// The failures this service reports are the mail server's own words, and a server is free to
/// quote back what it was sent. One `LOGIN` line echoed into an error message would put a
/// password in a notice, in `app.describe`, and in the transcript a mind is reading. So every
/// sentence built from a server's reply goes through here first, and the tests construct one
/// with a sentinel password and assert it never comes out the other side.
///
/// An empty secret matches nothing: `replace("", _)` would otherwise splice the marker between
/// every character of the message.
pub fn without_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "<redacted>")
}

/// The same, for an account that has more than one secret to lose.
///
/// A password was the only one until Google sign-in: an OAuth account carries an access token, a
/// refresh token and, for the length of one exchange, an authorization code — and every one of
/// them is a credential. A mail server or Google's own token endpoint can quote any of them back
/// in a refusal, so the sentence built from that refusal has to be cleared of all of them rather
/// than of whichever one the call site happened to remember.
pub fn without_secrets(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        out = without_secret(&out, secret);
    }
    out
}

/// The folder a per-message method acts on: the optional `folder` parameter, defaulting to
/// `INBOX`.
///
/// IMAP UIDs are per-mailbox, so "message 412" only means anything together with the folder it
/// was listed from. Callers that show a folder's own listing pass that folder; the default is
/// INBOX because that is all earlier callers ever asked about, and their calls must keep
/// working unchanged (#275).
///
/// Every per-message method takes the parameter, not only the two #275 added: star, move,
/// delete and search all acted on INBOX whatever folder was on screen (#288). The default is
/// kept for old callers of the two that cannot be undone — `delete_message` and `move_message`
/// — and the service logs it whenever one leans on it, so a wrong-mailbox delete leaves a
/// trace.
pub fn folder_or_inbox(params: &serde_json::Value) -> &str {
    params["folder"]
        .as_str()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .unwrap_or("INBOX")
}

/// An email message summary (for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailSummary {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub snippet: String,
    pub date: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub folder: String,
    pub thread_id: Option<String>,
}

/// Full email detail (for reading view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailDetail {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
    pub date: String,
    pub attachments: Vec<EmailAttachment>,
    pub thread_messages: Vec<EmailThreadEntry>,
    /// The IMAP flags, which the service already fetched and threw away.
    ///
    /// Without them the app had nothing to read a message's state from, so the reading pane
    /// hardcoded `is_read: true` and `is_flagged: false` — a star that was always hollow, and an
    /// unread message that looked read the moment it was opened, whether or not the mark
    /// actually happened. Both default, so an older service answering without them parses.
    #[serde(default)]
    pub is_read: bool,
    #[serde(default)]
    pub is_starred: bool,
}

/// An email attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

/// A message within a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailThreadEntry {
    pub id: String,
    pub from: String,
    pub date: String,
    pub snippet: String,
}

/// A folder's size and how much of it is unread, as the mail server counts them.
///
/// A zero here is a zero the server said. A folder whose counts the server would not give this
/// time has no `FolderCounts` at all — see [`EmailFolder::counts`] — because "0 unread of 0" is
/// what an empty folder reads as, and a refused STATUS or one that did not answer in time is
/// not that (#131).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FolderCounts {
    pub unread: i32,
    pub total: i32,
}

/// An email folder (IMAP mailbox).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailFolder {
    pub name: String,
    /// What the server answered when asked, or `None` when it did not answer — a `\Noselect`
    /// container that cannot hold mail, a STATUS the server refused, a reply that did not come in
    /// time. `None` is "not read", never "empty"; [`Self::reason`] says which.
    pub counts: Option<FolderCounts>,
    /// Why `counts` is `None`, in a sentence a reader can act on: the server's own refusal, or
    /// what the service tried and could not do. Absent when the counts are there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl EmailFolder {
    /// A folder the server counted.
    pub fn counted(name: impl Into<String>, unread: i32, total: i32) -> Self {
        EmailFolder { name: name.into(), counts: Some(FolderCounts { unread, total }), reason: None }
    }

    /// A folder the server did not count this time, and why.
    pub fn uncounted(name: impl Into<String>, reason: impl Into<String>) -> Self {
        EmailFolder { name: name.into(), counts: None, reason: Some(reason.into()) }
    }
}

/// Compose/send request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeRequest {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub reply_to_id: Option<String>,
    pub signature: Option<String>,
}

/// Email service operations.
pub trait EmailService: Send + Sync {
    fn list_folders(&self, account_id: &str) -> Result<Vec<EmailFolder>, ServiceError>;
    fn list_messages(&self, account_id: &str, folder: &str, page: u32, per_page: u32) -> Result<Vec<EmailSummary>, ServiceError>;
    /// Read one message from `folder` — the folder its summary was listed from; UIDs are
    /// per-mailbox, so the same id in another folder is another message (#275).
    fn get_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<EmailDetail, ServiceError>;
    fn send_message(&self, account_id: &str, compose: ComposeRequest) -> Result<(), ServiceError>;
    /// Flag one message in `folder`, for the same reason [`EmailService::get_message`] takes it.
    fn mark_read(&self, account_id: &str, folder: &str, message_id: &str, read: bool) -> Result<(), ServiceError>;
    /// Star one message in `folder`, for the same reason [`EmailService::get_message`] takes
    /// it: starring ran in INBOX whatever folder was on screen, so it moved the star on a
    /// different message than the one that was clicked (#288).
    fn mark_starred(&self, account_id: &str, folder: &str, message_id: &str, starred: bool) -> Result<(), ServiceError>;
    /// Move one message out of `folder`, for the same reason — and destructively: a move of a
    /// row listed from Spam moved whichever message wore this UID in INBOX (#288).
    fn move_message(&self, account_id: &str, folder: &str, message_id: &str, target_folder: &str) -> Result<(), ServiceError>;
    /// Delete one message from `folder`, for [`EmailService::move_message`]'s reason (#288).
    fn delete_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<(), ServiceError>;
    /// Search `folder` — the one on the caller's screen. Searching ran in INBOX whichever
    /// folder was open (#288).
    fn search(&self, account_id: &str, folder: &str, query: &str) -> Result<Vec<EmailSummary>, ServiceError>;
}

/// Shared error type for all services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

#[cfg(test)]
mod tests {
    use super::folder_or_inbox;

    #[test]
    fn folder_param_defaults_to_inbox_so_old_callers_keep_working() {
        assert_eq!(folder_or_inbox(&serde_json::json!({})), "INBOX");
        assert_eq!(folder_or_inbox(&serde_json::json!({"folder": ""})), "INBOX");
        assert_eq!(folder_or_inbox(&serde_json::json!({"folder": "  "})), "INBOX");
        assert_eq!(folder_or_inbox(&serde_json::json!({"folder": null})), "INBOX");
        assert_eq!(folder_or_inbox(&serde_json::json!({"folder": 7})), "INBOX");
    }

    #[test]
    fn folder_param_names_the_folder_the_message_was_listed_from() {
        assert_eq!(folder_or_inbox(&serde_json::json!({"folder": "Spam"})), "Spam");
        assert_eq!(
            folder_or_inbox(&serde_json::json!({"folder": "[Gmail]/Sent Mail"})),
            "[Gmail]/Sent Mail"
        );
    }
}
