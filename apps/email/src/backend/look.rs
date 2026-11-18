//! Everything the mail service is asked at once.

use yantrik_ipc_contracts::email::{EmailFolder, EmailSummary};

use super::{calls, MailService};
use crate::state::{self, MailState};

/// What one look at the mail service found.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub state: MailState,
    pub folders: Vec<EmailFolder>,
    pub messages: Vec<EmailSummary>,
    /// The account is configured and the mailbox still would not open. A different failure from
    /// the service being unreachable, and it has to be said differently.
    pub mailbox_error: Option<String>,
}

/// Ask the service what it has, and then — only if there is an account — open a folder.
///
/// This is the one path both the first load and the Refresh button take, so the screen cannot
/// come to believe two different things about the same machine depending on which one ran. It
/// touches no window: it runs in a job.
pub fn look_at_mail(api: &dyn MailService, folder: &str) -> Loaded {
    let state = state::decide(calls::accounts(api));
    let account = state.account_id();
    // No account, or the credential vault is shut and the password is in it: the mail server is
    // not asked. In the second case the service would refuse every call with the same sentence,
    // and one that was never asked cannot be mistaken for a mailbox that would not open.
    if account.is_empty() || state.waiting_for_vault() {
        return Loaded { state, folders: Vec::new(), messages: Vec::new(), mailbox_error: None };
    }

    let folders = match calls::list_folders(api, &account) {
        Ok(folders) => folders,
        Err(e) => {
            return Loaded {
                state,
                folders: Vec::new(),
                messages: Vec::new(),
                mailbox_error: Some(e),
            }
        }
    };
    let messages = match calls::list_messages(api, &account, folder, 1) {
        Ok(messages) => messages,
        Err(e) => return Loaded { state, folders, messages: Vec::new(), mailbox_error: Some(e) },
    };
    Loaded { state, folders, messages, mailbox_error: None }
}
