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

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use yantrik_ipc_transport::SyncRpcClient;
use super::flight::Flight;
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

/// How long a call that reaches a mail server may take, and how long one that does not.
///
/// The transport's default is two seconds, which is right for a local service answering out of
/// memory and wrong here: a list, a read, a search or a send leaves the machine, and on a real
/// mailbox takes 0.5 to 5 seconds and sometimes much more. Under the default every one of them
/// failed with a bare "Resource temporarily unavailable" (the read timing out). The Email app gives
/// the same calls ten seconds from a job; the companion runs on a worker and waits longer. One
/// place, so no call can be left on the default.
pub const NETWORK_BUDGET: Duration = Duration::from_secs(60);
/// Calls that stay on this machine: the account list, and the import, which only reaches the vault.
pub const LOCAL_BUDGET: Duration = Duration::from_secs(15);

/// The code of "the service did not answer in time". Its own, so a caller can tell it from a
/// refusal.
pub const TIMED_OUT: i32 = -32013;

const SERVICE: &str = "email";

/// The code of "too many mail calls are already waiting". Nothing was sent.
pub const BUSY: i32 = -32014;

/// What the Mind is told when the service did not answer in time. A change may have gone through
/// anyway, and the sentence says so rather than inviting a blind repeat.
fn slow_sentence(changes_something: bool) -> String {
    if changes_something {
        "Mail is taking longer than usual to answer, so I can't say whether that went through. Check before doing it again."
            .to_string()
    } else {
        "Mail is taking longer than usual to answer. Nothing was changed; try again in a moment.".to_string()
    }
}

/// What the Mind is told when the transport refused to dial an address that failed a moment ago.
/// Nothing was sent, so this is true of a change as well as a read.
const NOT_SENT: &str = "Mail did not answer a moment ago, so this was not tried. Nothing was changed; try again in a moment.";

const TOO_BUSY: &str = "Several mail requests are already waiting. Nothing was changed; try again in a moment.";

/// The real one: `email-service` on its socket, started on demand.
#[derive(Default, Clone)]
pub struct ServiceMail {
    /// A socket to use instead of the service's own, for tests.
    address: Option<String>,
    /// Budgets other than [`NETWORK_BUDGET`] and [`LOCAL_BUDGET`], for tests.
    budgets: Option<(Duration, Duration)>,
    /// The ceiling on calls waiting, and the time of this port's last timeout.
    flight: Arc<Flight>,
}

impl ServiceMail {
    /// The live service.
    pub fn live() -> Self {
        Self::default()
    }

    /// A service listening at `address`, with these budgets (network, local). For tests.
    pub fn at(address: &str, network: Duration, local: Duration) -> Self {
        Self { address: Some(address.to_string()), budgets: Some((network, local)), flight: Arc::default() }
    }

    fn budget(&self, network: bool) -> Duration {
        let (net, local) = self.budgets.unwrap_or((NETWORK_BUDGET, LOCAL_BUDGET));
        if network { net } else { local }
    }

    fn client(&self, budget: Duration) -> Result<SyncRpcClient, ServiceError> {
        let client = match &self.address {
            Some(address) => SyncRpcClient::new(address),
            None => yantrik_ipc_transport::service::client(SERVICE)
                .map_err(|message| ServiceError { code: -32000, message })?,
        };
        Ok(client.with_timeout(budget))
    }

    /// One call. `network` picks the budget; `changes` words a timeout honestly.
    ///
    /// Retried once, and only when the transport refused to dial (its breaker, tripped by some
    /// other call): that refusal is made before anything is built or written, so nothing can be
    /// done twice. A call that was sent and then timed out is never asked again, and a breaker
    /// that this port's own timeout just tripped is believed (see [`Flight`]).
    fn call<R: serde::de::DeserializeOwned>(
        &self,
        method_name: &str,
        params: serde_json::Value,
        network: bool,
        changes: bool,
    ) -> Result<R, ServiceError> {
        let Some(_place) = self.flight.enter() else {
            return Err(ServiceError { code: BUSY, message: TOO_BUSY.to_string() });
        };
        let client = self.client(self.budget(network))?;
        let mut result = client.call_typed(method_name, &params);
        if matches!(&result, Err(e) if SyncRpcClient::is_breaker_refusal(e)) && !self.flight.timed_out_recently() {
            // The client clears the key it dials, which in a mind's process is the door's path.
            client.forget_breaker();
            result = client.call_typed(method_name, &params);
        }
        result.map_err(|e| {
            if SyncRpcClient::is_io_timeout(&e) {
                self.flight.note_timeout();
                ServiceError { code: TIMED_OUT, message: slow_sentence(changes) }
            } else if SyncRpcClient::is_breaker_refusal(&e) {
                ServiceError { code: TIMED_OUT, message: NOT_SENT.to_string() }
            } else {
                ServiceError { code: e.code, message: e.message }
            }
        })
    }

    fn read<R: serde::de::DeserializeOwned>(&self, method_name: &str, params: serde_json::Value) -> Result<R, ServiceError> {
        self.call(method_name, params, true, false)
    }

    fn change(&self, method_name: &str, params: serde_json::Value) -> Result<(), ServiceError> {
        self.call::<serde_json::Value>(method_name, params, true, true).map(|_| ())
    }
}

impl MailPort for ServiceMail {
    fn accounts(&self) -> Result<Vec<EmailAccountSummary>, ServiceError> {
        self.call::<AccountsResult>(method::ACCOUNTS, json!({}), false, false).map(|r| r.accounts)
    }

    fn list_folders(&self, account_id: &str) -> Result<Vec<EmailFolder>, ServiceError> {
        self.read(method::LIST_FOLDERS, json!({ "account_id": account_id }))
    }

    fn list_messages(&self, account_id: &str, folder: &str, page: u32, per_page: u32) -> Result<Vec<EmailSummary>, ServiceError> {
        self.read(
            method::LIST_MESSAGES,
            json!({ "account_id": account_id, "folder": folder, "page": page, "per_page": per_page }),
        )
    }

    fn search(&self, account_id: &str, folder: &str, query: &str) -> Result<Vec<EmailSummary>, ServiceError> {
        self.read(method::SEARCH, json!({ "account_id": account_id, "folder": folder, "query": query }))
    }

    fn get_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<EmailDetail, ServiceError> {
        self.read(
            method::GET_MESSAGE,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id }),
        )
    }

    fn send(&self, account_id: &str, compose: &ComposeRequest) -> Result<(), ServiceError> {
        let mut params = serde_json::to_value(compose).unwrap_or_default();
        params["account_id"] = json!(account_id);
        self.change(method::SEND_MESSAGE, params)
    }

    fn mark_read(&self, account_id: &str, folder: &str, message_ids: &[String], read: bool) -> Result<(), ServiceError> {
        self.change(
            method::MARK_READ,
            json!({ "account_id": account_id, "folder": folder, "message_ids": message_ids, "read": read }),
        )
    }

    fn mark_starred(&self, account_id: &str, folder: &str, message_id: &str, starred: bool) -> Result<(), ServiceError> {
        self.change(
            method::MARK_STARRED,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id, "starred": starred }),
        )
    }

    fn move_message(&self, account_id: &str, folder: &str, message_id: &str, target_folder: &str) -> Result<(), ServiceError> {
        self.change(
            method::MOVE_MESSAGE,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id, "target_folder": target_folder }),
        )
    }

    fn delete_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<(), ServiceError> {
        self.change(
            method::DELETE_MESSAGE,
            json!({ "account_id": account_id, "folder": folder, "message_id": message_id }),
        )
    }

    fn import_account(&self, account: &ImportAccount) -> Result<ImportResult, ServiceError> {
        self.call(method::IMPORT_ACCOUNT, serde_json::to_value(account).unwrap_or_default(), false, true)
    }
}
