//! A mail service that lives in memory, for the tests of the tools and of the migration.
//!
//! It answers the questions `email-service` answers, in its shapes, and keeps the same promises the
//! real one is held to: an import whose address is already an account changes nothing, a new one
//! keeps its secret in the "vault" before it is listed, and a shut vault refuses with the code the
//! door uses so callers can tell "ask again when it opens" from "no".
//!
//! Every fake password in the tests is a made-up sentinel; nothing here is a real credential.

use std::collections::HashMap;
use std::sync::Mutex;

use yantrik_ipc_contracts::credentials::ERR_LOCKED;
use yantrik_ipc_contracts::email::{
    ComposeRequest, EmailAccountSummary, EmailDetail, EmailFolder, EmailSummary, ImportAccount, ImportResult,
    ServiceError, WAITING_FOR_VAULT,
};

use super::port::MailPort;

/// What the credential vault is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vault {
    Open,
    Shut,
    Failing,
}

pub struct State {
    pub vault: Vault,
    pub accounts: Vec<EmailAccountSummary>,
    /// What the "vault" holds, by account id.
    pub kept: HashMap<String, String>,
    pub listed: Vec<EmailSummary>,
    pub found: Vec<EmailSummary>,
    pub detail: Option<EmailDetail>,
    pub folders: Vec<EmailFolder>,
    /// Every call, as `method account folder ids`, in order. Never holds a body or a secret.
    pub calls: Vec<String>,
    pub sent: Vec<(String, ComposeRequest)>,
    /// A refusal to give to every call that touches a mailbox.
    pub refuse: Option<ServiceError>,
}

pub struct FakeMail {
    pub state: Mutex<State>,
}

impl FakeMail {
    pub fn with_accounts(accounts: Vec<EmailAccountSummary>) -> Self {
        Self {
            state: Mutex::new(State {
                vault: Vault::Open,
                accounts,
                kept: HashMap::new(),
                listed: Vec::new(),
                found: Vec::new(),
                detail: None,
                folders: Vec::new(),
                calls: Vec::new(),
                sent: Vec::new(),
                refuse: None,
            }),
        }
    }

    pub fn empty() -> Self {
        Self::with_accounts(Vec::new())
    }

    pub fn set_vault(&self, vault: Vault) {
        self.state.lock().unwrap().vault = vault;
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn sent(&self) -> Vec<(String, ComposeRequest)> {
        self.state.lock().unwrap().sent.clone()
    }

    pub fn count(&self, method: &str) -> usize {
        self.calls().iter().filter(|c| c.starts_with(method)).count()
    }

    fn touch(&self, what: String) -> Result<(), ServiceError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(what);
        match (&state.refuse, state.vault) {
            (Some(e), _) => Err(e.clone()),
            (None, Vault::Shut) => Err(ServiceError { code: ERR_LOCKED, message: WAITING_FOR_VAULT.to_string() }),
            (None, Vault::Failing) => Err(ServiceError {
                code: -32000,
                message: "The credential vault did not answer, so mail was not checked. Try again in a moment.".to_string(),
            }),
            (None, Vault::Open) => Ok(()),
        }
    }
}

/// An account as the service lists one.
pub fn account(id: &str, email: &str) -> EmailAccountSummary {
    EmailAccountSummary {
        id: id.to_string(),
        email: email.to_string(),
        display_name: String::new(),
        provider: "gmail".to_string(),
        imap_server: "imap.example.com".to_string(),
        imap_port: 993,
        smtp_server: "smtp.example.com".to_string(),
        smtp_port: 587,
        uses_oauth: false,
    }
}

/// A listed message.
pub fn message(uid: u32, from: &str, address: &str, subject: &str) -> EmailSummary {
    EmailSummary {
        id: uid.to_string(),
        from: from.to_string(),
        to: Vec::new(),
        subject: subject.to_string(),
        snippet: String::new(),
        date: "2026-10-30 09:15".to_string(),
        is_read: false,
        is_starred: false,
        has_attachments: false,
        folder: "INBOX".to_string(),
        thread_id: None,
        from_address: address.to_string(),
        message_id: format!("<m{uid}@example.com>"),
    }
}

impl MailPort for FakeMail {
    fn accounts(&self) -> Result<Vec<EmailAccountSummary>, ServiceError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push("accounts".to_string());
        Ok(state.accounts.clone())
    }

    fn list_folders(&self, account_id: &str) -> Result<Vec<EmailFolder>, ServiceError> {
        self.touch(format!("list_folders {account_id}"))?;
        Ok(self.state.lock().unwrap().folders.clone())
    }

    fn list_messages(&self, account_id: &str, folder: &str, _page: u32, _per_page: u32) -> Result<Vec<EmailSummary>, ServiceError> {
        self.touch(format!("list_messages {account_id} {folder}"))?;
        Ok(self.state.lock().unwrap().listed.clone())
    }

    fn search(&self, account_id: &str, folder: &str, query: &str) -> Result<Vec<EmailSummary>, ServiceError> {
        self.touch(format!("search {account_id} {folder} {query}"))?;
        Ok(self.state.lock().unwrap().found.clone())
    }

    fn get_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<EmailDetail, ServiceError> {
        self.touch(format!("get_message {account_id} {folder} {message_id}"))?;
        self.state
            .lock()
            .unwrap()
            .detail
            .clone()
            .ok_or(ServiceError { code: -32000, message: format!("Message not found: {message_id}") })
    }

    fn send(&self, account_id: &str, compose: &ComposeRequest) -> Result<(), ServiceError> {
        self.touch(format!("send {account_id}"))?;
        self.state.lock().unwrap().sent.push((account_id.to_string(), compose.clone()));
        Ok(())
    }

    fn mark_read(&self, account_id: &str, folder: &str, message_ids: &[String], read: bool) -> Result<(), ServiceError> {
        self.touch(format!("mark_read {account_id} {folder} {} {read}", message_ids.join(",")))
    }

    fn mark_starred(&self, account_id: &str, folder: &str, message_id: &str, starred: bool) -> Result<(), ServiceError> {
        self.touch(format!("mark_starred {account_id} {folder} {message_id} {starred}"))
    }

    fn move_message(&self, account_id: &str, folder: &str, message_id: &str, target: &str) -> Result<(), ServiceError> {
        self.touch(format!("move_message {account_id} {folder} {message_id} {target}"))
    }

    fn delete_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<(), ServiceError> {
        self.touch(format!("delete_message {account_id} {folder} {message_id}"))
    }

    fn import_account(&self, import: &ImportAccount) -> Result<ImportResult, ServiceError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(format!("import_account {}", import.email));
        // The same order as the service: an address it has is answered first and changes nothing,
        // so a shut vault does not matter to an account that is already there.
        if let Some(have) = state.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(import.email.trim())) {
            return Ok(ImportResult { account: have.clone(), created: false });
        }
        if import.imap_server.is_empty() || import.smtp_server.is_empty() {
            return Err(ServiceError { code: -32602, message: "no IMAP server was given".to_string() });
        }
        match state.vault {
            Vault::Shut => return Err(ServiceError { code: ERR_LOCKED, message: WAITING_FOR_VAULT.to_string() }),
            Vault::Failing => {
                return Err(ServiceError { code: -32000, message: "The credential vault did not answer.".to_string() })
            }
            Vault::Open => {}
        }
        let id = import.email.trim().to_lowercase().replace('@', "-at-");
        let secret = if import.uses_oauth() {
            format!(
                "{}|{}",
                import.oauth_access_token.clone().unwrap_or_default(),
                import.oauth_refresh_token.clone().unwrap_or_default()
            )
        } else {
            import.password.clone()
        };
        state.kept.insert(format!("mail:{id}"), secret);
        let mut summary = account(&id, import.email.trim());
        summary.imap_server = import.imap_server.clone();
        summary.smtp_server = import.smtp_server.clone();
        summary.uses_oauth = import.uses_oauth();
        state.accounts.push(summary.clone());
        Ok(ImportResult { account: summary, created: true })
    }
}
