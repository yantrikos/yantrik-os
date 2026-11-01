//! Email tools — full email management: check, list, read, send, reply, reply-all, forward,
//! search, mark read/unread, flag/unflag, delete, move, archive.
//!
//! One mail stack (#1022). Every tool here asks `email-service` over its IPC, with the same
//! methods the Email app uses, and the service signs in with the account's secret from the
//! credential vault for the length of one call. The companion keeps no mail password, no token and
//! no IMAP or SMTP client: this directory holds none, and `tests::no_mail_client_is_left` pins it.
//!
//! The calls are blocking and run on the calling thread; the agent loop already runs tool
//! executions on a worker.
//!
//! Grades and approvals are the tools' own and unchanged by where the mail goes: `Safe` to read,
//! `Standard` to send, reply, forward, flag, move or archive, `Sensitive` to delete. The approval
//! and policy lists key on the tool names, which are the same.
//!
//! Reach: the tools ask the mail service, so they reach every account it holds, not only the ones
//! once listed in `config.yaml`. See the PR for #1022.

mod change;
mod read;
mod send;
mod support;
#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::email::approval::{Approving, ShellApprover};
use crate::email::port::{MailPort, ServiceMail};

/// Register all email tools, over the real mail service.
///
/// The port every tool is given asks the person before it sends (`Approving`, over the shell's
/// approval card): a send is outward and cannot be undone, in every mode. Nothing registers the
/// bare `ServiceMail`; `approval_tests` reads this function to keep it so.
pub fn register(reg: &mut ToolRegistry) {
    register_with(reg, Arc::new(Approving::new(Arc::new(ServiceMail::live()), ShellApprover)));
}

/// Register all email tools over `mail`. The tests pass a fake.
pub fn register_with(reg: &mut ToolRegistry, mail: Arc<dyn MailPort>) {
    reg.register(Box::new(read::EmailCheckTool { mail: mail.clone() }));
    reg.register(Box::new(read::EmailListTool { mail: mail.clone() }));
    reg.register(Box::new(read::EmailReadTool { mail: mail.clone() }));
    reg.register(Box::new(send::EmailSendTool { mail: mail.clone() }));
    reg.register(Box::new(send::EmailReplyTool { mail: mail.clone() }));
    reg.register(Box::new(send::EmailReplyAllTool { mail: mail.clone() }));
    reg.register(Box::new(send::EmailForwardTool { mail: mail.clone() }));
    reg.register(Box::new(read::EmailSearchTool { mail: mail.clone() }));
    reg.register(Box::new(change::EmailMarkReadTool { mail: mail.clone() }));
    reg.register(Box::new(change::EmailMarkUnreadTool { mail: mail.clone() }));
    reg.register(Box::new(change::EmailFlagTool { mail: mail.clone() }));
    reg.register(Box::new(change::EmailUnflagTool { mail: mail.clone() }));
    reg.register(Box::new(change::EmailDeleteTool { mail: mail.clone() }));
    reg.register(Box::new(change::EmailMoveTool { mail: mail.clone() }));
    reg.register(Box::new(change::EmailArchiveTool { mail: mail.clone() }));
    reg.register(Box::new(read::EmailListFoldersTool { mail }));
}
