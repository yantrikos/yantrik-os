//! The mail service's methods, typed.
//!
//! Every one of these returned `Option` and every call site wrote `let _ =` or `if ….is_some()`,
//! which is how a mail server's refusal became silence on a screen. They return `Result`, and the
//! reason is the thing the caller is obliged to do something with.

use yantrik_ipc_contracts::email::{
    method, AccountSettings, AccountsResult, EmailAccountSummary, EmailDetail, EmailFolder,
    EmailSummary, OAuthBeginResult, OAuthStatus, TestAccountResult,
};

use super::MailService;

fn call(api: &dyn MailService, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    api.call(method, params)
}

fn call_typed<T: serde::de::DeserializeOwned>(
    api: &dyn MailService,
    method: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    let value = call(api, method, params)?;
    serde_json::from_value(value)
        .map_err(|e| format!("the mail service answered with something this app cannot read: {e}"))
}

/// What accounts are configured. Answers without touching a mail server, which is the whole
/// reason it exists: it is the one question whose failure means the service is not there.
pub fn accounts(api: &dyn MailService) -> Result<AccountsResult, String> {
    call_typed(api, method::ACCOUNTS, serde_json::json!({}))
}

pub fn list_folders(api: &dyn MailService, account: &str) -> Result<Vec<EmailFolder>, String> {
    call_typed(api, method::LIST_FOLDERS, serde_json::json!({ "account_id": account }))
}

pub fn list_messages(
    api: &dyn MailService,
    account: &str,
    folder: &str,
    page: u32,
) -> Result<Vec<EmailSummary>, String> {
    call_typed(
        api,
        method::LIST_MESSAGES,
        serde_json::json!({ "account_id": account, "folder": folder, "page": page.max(1) }),
    )
}

/// Read one message out of `folder` — the folder its row was listed from. UIDs are per-mailbox,
/// so the folder is part of naming the message, not context the service can assume (#275).
pub fn get_message(
    api: &dyn MailService,
    account: &str,
    folder: &str,
    message_id: &str,
) -> Result<EmailDetail, String> {
    call_typed(
        api,
        method::GET_MESSAGE,
        serde_json::json!({ "account_id": account, "folder": folder, "message_id": message_id }),
    )
}

/// The five fields of the composer, as `send_message` takes them.
pub struct Outgoing<'a> {
    pub to: &'a str,
    pub cc: &'a str,
    pub bcc: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
}

pub fn send_message(api: &dyn MailService, account: &str, mail: &Outgoing<'_>) -> Result<(), String> {
    let addresses = |line: &str| -> Vec<String> {
        line.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    call(
        api,
        method::SEND_MESSAGE,
        serde_json::json!({
            "account_id": account,
            "to": addresses(mail.to),
            "cc": addresses(mail.cc),
            "bcc": addresses(mail.bcc),
            "subject": mail.subject,
            "body": mail.body,
        }),
    )
    .map(|_| ())
}

/// Search `folder` — the one on screen. Search ran in INBOX whichever folder was open, so a
/// search from Spam answered with mail nobody was looking at (#288).
pub fn search(
    api: &dyn MailService,
    account: &str,
    folder: &str,
    query: &str,
) -> Result<Vec<EmailSummary>, String> {
    call_typed(
        api,
        method::SEARCH,
        serde_json::json!({ "account_id": account, "folder": folder, "query": query }),
    )
}

/// Flag one message in `folder`, for [`get_message`]'s reason: the app marks a message read right
/// after opening it, and flagging INBOX's UID instead would touch a stranger's mail.
pub fn mark_read(
    api: &dyn MailService,
    account: &str,
    folder: &str,
    message_id: &str,
    read: bool,
) -> Result<(), String> {
    call(
        api,
        method::MARK_READ,
        serde_json::json!({
            "account_id": account,
            "folder": folder,
            "message_id": message_id,
            "read": read,
        }),
    )
    .map(|_| ())
}

/// Star one message in `folder`, for [`mark_read`]'s reason: starring ran in INBOX whatever
/// folder the row was listed from, and moved the star on a stranger's message (#288).
pub fn mark_starred(
    api: &dyn MailService,
    account: &str,
    folder: &str,
    message_id: &str,
    starred: bool,
) -> Result<(), String> {
    call(
        api,
        method::MARK_STARRED,
        serde_json::json!({
            "account_id": account,
            "folder": folder,
            "message_id": message_id,
            "starred": starred,
        }),
    )
    .map(|_| ())
}

/// Delete one message from `folder` — always the row's own, never omitted. This cannot be
/// undone, and a UID with no folder beside it can destroy a different message in INBOX (#288).
pub fn delete_message(
    api: &dyn MailService,
    account: &str,
    folder: &str,
    message_id: &str,
) -> Result<(), String> {
    call(
        api,
        method::DELETE_MESSAGE,
        serde_json::json!({ "account_id": account, "folder": folder, "message_id": message_id }),
    )
    .map(|_| ())
}

/// Move one message out of `folder` into `target_folder`. The source folder is never omitted
/// either, for [`delete_message`]'s reason (#288).
pub fn move_message(
    api: &dyn MailService,
    account: &str,
    folder: &str,
    message_id: &str,
    target_folder: &str,
) -> Result<(), String> {
    call(
        api,
        method::MOVE_MESSAGE,
        serde_json::json!({
            "account_id": account,
            "folder": folder,
            "message_id": message_id,
            "target_folder": target_folder,
        }),
    )
    .map(|_| ())
}

pub fn test_account(api: &dyn MailService, settings: &AccountSettings) -> Result<TestAccountResult, String> {
    let params = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    call_typed(api, method::TEST_ACCOUNT, params)
}

pub fn save_account(api: &dyn MailService, settings: &AccountSettings) -> Result<EmailAccountSummary, String> {
    let params = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    call_typed(api, method::SAVE_ACCOUNT, params)
}

// ── The Google sign-in ───────────────────────────────────────────────
//
// Three calls, because there is a person in the middle of it. `begin` answers in microseconds
// with somewhere to send them; the mail service sits on a loopback socket meanwhile; `status` is
// asked from a job once a second until it stops saying "waiting". A single blocking call would
// hold this app's ten-second budget for the minutes a person takes to choose a Google account.
//
// No token reaches this process. What comes back from a finished sign-in is the same account
// summary `save_account` answers with — an address and the servers behind it.

pub fn oauth_begin(api: &dyn MailService) -> Result<OAuthBeginResult, String> {
    call_typed(api, method::OAUTH_BEGIN, serde_json::json!({ "provider": "google" }))
}

pub fn oauth_status(api: &dyn MailService, flow_id: &str) -> Result<OAuthStatus, String> {
    call_typed(api, method::OAUTH_STATUS, serde_json::json!({ "flow_id": flow_id }))
}

pub fn oauth_cancel(api: &dyn MailService, flow_id: &str) -> Result<(), String> {
    call(api, method::OAUTH_CANCEL, serde_json::json!({ "flow_id": flow_id })).map(|_| ())
}
