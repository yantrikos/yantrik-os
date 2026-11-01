//! The person's say before mail leaves (#1026, 520 gate).
//!
//! On VM 520 the companion replied to three emails from the person's account with no card, in
//! `auto` and in `ask`: the tools are graded `standard`, the companion's own ceiling lets
//! `standard` run, and the policy engine's "ask" only writes a log line. A send is outward and
//! cannot be undone.
//!
//! [`Approving`] wraps the mail port every tool is given and asks an [`Approver`] before
//! `send`, the one method any tool puts mail on the wire with (send, reply, reply all and forward
//! all end in it). The real approver is the desktop's own card (`yantrik_companion_tools::outward`,
//! the shell's approvals module): no second permission system, and every mode but the person's
//! full bypass shows the card, plan refuses. A `no`, silence, or a shell that cannot be reached
//! sends nothing. `register` wraps the live port; `approval_tests` pins that, and that no other
//! port method can send.

use yantrik_ipc_contracts::email::{
    ComposeRequest, EmailAccountSummary, EmailDetail, EmailFolder, EmailSummary, ImportAccount,
    ImportResult, ServiceError,
};

use std::sync::Arc;

use yantrik_companion_tools::outward::Permit;

use super::port::MailPort;

/// The code of "the person did not allow this send" (a no, silence, or no way to ask).
pub const NOT_ALLOWED: i32 = -32015;

/// Whether a message may leave. `Err` is the sentence the Mind is told, and nothing was sent.
pub trait Approver: Send + Sync {
    /// `from` is the address the message goes out under.
    ///
    /// The [`Permit`] it returns is finished after the send, so a send that ran without anyone
    /// being asked (the person's full bypass) is still written down.
    fn allow_send(&self, from: &str, compose: &ComposeRequest) -> Result<Permit, String>;
}

/// The desktop's approval card.
pub struct ShellApprover;

impl Approver for ShellApprover {
    fn allow_send(&self, from: &str, compose: &ComposeRequest) -> Result<Permit, String> {
        yantrik_companion_tools::outward::ask_person_before_sending(&yantrik_companion_tools::outward::OutgoingMail {
            account: from,
            to: &compose.to,
            cc: &compose.cc,
            bcc: &compose.bcc,
            subject: &compose.subject,
            body: &compose.body,
        })
    }
}

/// An approver that answers as told and remembers every card it was shown. For tests.
#[cfg(test)]
pub struct Scripted {
    pub answer: Result<(), String>,
    pub cards: std::sync::Mutex<Vec<(String, ComposeRequest)>>,
}

#[cfg(test)]
impl Scripted {
    pub fn allowing() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { answer: Ok(()), cards: Default::default() })
    }

    pub fn refusing(why: &str) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { answer: Err(why.to_string()), cards: Default::default() })
    }

    pub fn shown(&self) -> Vec<(String, ComposeRequest)> {
        self.cards.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Approver for std::sync::Arc<Scripted> {
    fn allow_send(&self, from: &str, compose: &ComposeRequest) -> Result<Permit, String> {
        self.cards.lock().unwrap().push((from.to_string(), compose.clone()));
        self.answer.clone().map(|()| Permit::asked())
    }
}

/// A mail port that asks before it sends. Every other method passes straight through.
pub struct Approving<A: Approver> {
    inner: Arc<dyn MailPort>,
    approver: A,
}

impl<A: Approver> Approving<A> {
    pub fn new(inner: Arc<dyn MailPort>, approver: A) -> Self {
        Self { inner, approver }
    }
}

impl<A: Approver> MailPort for Approving<A> {
    fn accounts(&self) -> Result<Vec<EmailAccountSummary>, ServiceError> {
        self.inner.accounts()
    }

    fn list_folders(&self, account_id: &str) -> Result<Vec<EmailFolder>, ServiceError> {
        self.inner.list_folders(account_id)
    }

    fn list_messages(&self, account_id: &str, folder: &str, page: u32, per_page: u32) -> Result<Vec<EmailSummary>, ServiceError> {
        self.inner.list_messages(account_id, folder, page, per_page)
    }

    fn search(&self, account_id: &str, folder: &str, query: &str) -> Result<Vec<EmailSummary>, ServiceError> {
        self.inner.search(account_id, folder, query)
    }

    fn get_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<EmailDetail, ServiceError> {
        self.inner.get_message(account_id, folder, message_id)
    }

    fn send(&self, account_id: &str, compose: &ComposeRequest) -> Result<(), ServiceError> {
        // The address, for the card: the account's own, else its id.
        let from = self
            .inner
            .accounts()
            .ok()
            .and_then(|all| all.into_iter().find(|a| a.id == account_id))
            .map(|a| a.email)
            .unwrap_or_else(|| account_id.to_string());
        let permit = self
            .approver
            .allow_send(&from, compose)
            .map_err(|message| ServiceError { code: NOT_ALLOWED, message })?;
        let sent = self.inner.send(account_id, compose);
        permit.finish(sent.is_ok());
        sent
    }

    fn mark_read(&self, account_id: &str, folder: &str, message_ids: &[String], read: bool) -> Result<(), ServiceError> {
        self.inner.mark_read(account_id, folder, message_ids, read)
    }

    fn mark_starred(&self, account_id: &str, folder: &str, message_id: &str, starred: bool) -> Result<(), ServiceError> {
        self.inner.mark_starred(account_id, folder, message_id, starred)
    }

    fn move_message(&self, account_id: &str, folder: &str, message_id: &str, target_folder: &str) -> Result<(), ServiceError> {
        self.inner.move_message(account_id, folder, message_id, target_folder)
    }

    fn delete_message(&self, account_id: &str, folder: &str, message_id: &str) -> Result<(), ServiceError> {
        self.inner.delete_message(account_id, folder, message_id)
    }

    fn import_account(&self, account: &ImportAccount) -> Result<ImportResult, ServiceError> {
        self.inner.import_account(account)
    }
}
