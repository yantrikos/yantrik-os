//! The companion's one way to mail: `email-service`, over its own IPC.
//!
//! The companion used to run IMAP and SMTP clients of its own, signed in with passwords kept in
//! clear text in `config.yaml` (#1022). There is one mail stack now. `email-service` owns the
//! accounts and signs in with secrets it reads from the credential vault for the length of one
//! call; this module holds none of that. Every method here is a method of the service's contract
//! (`yantrik_ipc_contracts::email::method`), the ones the Email app calls.
//!
//! The trait exists so the tools can be tested without a socket: the fake in the tests answers the
//! same questions the service does, including the answer a shut vault gives.

use serde_json::json;
use yantrik_ipc_contracts::credentials::ERR_LOCKED;
use yantrik_ipc_contracts::email::{
    method, AccountsResult, ComposeRequest, EmailAccountSummary, EmailDetail, EmailFolder, EmailSummary,
    ImportAccount, ImportResult, ServiceError,
};

/// What the mail service has to be able to answer for the mind's tools.
///
/// Errors are the service's own, with its code: a tool reports what it observed, and the
/// migration needs [`is_locked`] to tell "the vault is shut, ask again when it opens" from "no".
/// No method here ever returns, logs or formats a secret: none passes through it except
/// [`MailPort::import_account`], which sends one and answers without it.
pub trait MailPort: Send + Sync {
    fn accounts(&self) -> Result<Vec<EmailAccountSummary>, ServiceError>;
    fn list_folders(&self, account_id: &str) -> Result<Vec<EmailFolder>, ServiceError>;
    fn list_messages(&self, account_id: &str, folder: &str, page: u32, per_page: u32) -> Result<Vec<EmailSummary>, ServiceError>;
    fn search(&self, account_id: &str, folder: &str, query: &str) -> Result<Vec<EmailSummary>, ServiceError>;
    fn get_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<EmailDetail, ServiceError>;
    fn send(&self, account_id: &str, compose: &ComposeRequest) -> Result<(), ServiceError>;
    /// Set or clear the read flag on every message in `message_ids`, in one mailbox session.
    fn mark_read(&self, account_id: &str, folder: &str, message_ids: &[String], read: bool) -> Result<(), ServiceError>;
    fn mark_starred(&self, account_id: &str, folder: &str, message_id: &str, starred: bool) -> Result<(), ServiceError>;
    fn move_message(&self, account_id: &str, folder: &str, message_id: &str, target_folder: &str) -> Result<(), ServiceError>;
    fn delete_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<(), ServiceError>;
    /// Bring in an account that was kept in `config.yaml`; see [`method::IMPORT_ACCOUNT`].
    fn import_account(&self, account: &ImportAccount) -> Result<ImportResult, ServiceError>;
}

/// Whether the service said the credential vault is shut.
pub fn is_locked(error: &ServiceError) -> bool {
    error.code == ERR_LOCKED
}

/// The real one: `email-service` on its socket, started on demand.
pub struct ServiceMail;

const SERVICE: &str = "email";

impl ServiceMail {
    fn call<R: serde::de::DeserializeOwned>(method_name: &str, params: serde_json::Value) -> Result<R, ServiceError> {
        let client = yantrik_ipc_transport::service::client(SERVICE)
            .map_err(|message| ServiceError { code: -32000, message })?;
        client
            .call_typed(method_name, &params)
            .map_err(|e| ServiceError { code: e.code, message: e.message })
    }

    fn call_unit(method_name: &str, params: serde_json::Value) -> Result<(), ServiceError> {
        Self::call::<serde_json::Value>(method_name, params).map(|_| ())
    }
}

impl MailPort for ServiceMail {
    fn accounts(&self) -> Result<Vec<EmailAccountSummary>, ServiceError> {
        Self::call::<AccountsResult>(method::ACCOUNTS, json!({})).map(|r| r.accounts)
    }

    fn list_folders(&self, account_id: &str) -> Result<Vec<EmailFolder>, ServiceError> {
        Self::call(method::LIST_FOLDERS, json!({ "account_id": account_id }))
    }

    fn list_messages(&self, account_id: &str, folder: &str, page: u32, per_page: u32) -> Result<Vec<EmailSummary>, ServiceError> {
        Self::call(
            method::LIST_MESSAGES,
            json!({ "account_id": account_id, "folder": folder, "page": page, "per_page": per_page }),
        )
    }

    fn search(&self, account_id: &str, folder: &str, query: &str) -> Result<Vec<EmailSummary>, ServiceError> {
        Self::call(method::SEARCH, json!({ "account_id": account_id, "folder": folder, "query": query }))
    }

    fn get_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<EmailDetail, ServiceError> {
        Self::call(
            method::GET_MESSAGE,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id }),
        )
    }

    fn send(&self, account_id: &str, compose: &ComposeRequest) -> Result<(), ServiceError> {
        let mut params = serde_json::to_value(compose).unwrap_or_default();
        params["account_id"] = json!(account_id);
        Self::call_unit(method::SEND_MESSAGE, params)
    }

    fn mark_read(&self, account_id: &str, folder: &str, message_ids: &[String], read: bool) -> Result<(), ServiceError> {
        Self::call_unit(
            method::MARK_READ,
            json!({ "account_id": account_id, "folder": folder, "message_ids": message_ids, "read": read }),
        )
    }

    fn mark_starred(&self, account_id: &str, folder: &str, message_id: &str, starred: bool) -> Result<(), ServiceError> {
        Self::call_unit(
            method::MARK_STARRED,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id, "starred": starred }),
        )
    }

    fn move_message(&self, account_id: &str, folder: &str, message_id: &str, target_folder: &str) -> Result<(), ServiceError> {
        Self::call_unit(
            method::MOVE_MESSAGE,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id, "target_folder": target_folder }),
        )
    }

    fn delete_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<(), ServiceError> {
        Self::call_unit(
            method::DELETE_MESSAGE,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id }),
        )
    }

    fn import_account(&self, account: &ImportAccount) -> Result<ImportResult, ServiceError> {
        Self::call(method::IMPORT_ACCOUNT, serde_json::to_value(account).unwrap_or_default())
    }
}
