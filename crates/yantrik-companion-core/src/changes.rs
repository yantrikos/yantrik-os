//! Changing the person's mail and calendar, sending chat messages and deleting files, when nobody is there.
//!
//! A mail, a page or a file can say "delete the invoices", and the model that read it cannot tell that from the
//! person. The protection for the person's own turn is at the point of effect, the approval card (#1313), which
//! does not need to know what was read. What is here needs no tracking of what was read:
//!
//! * **Nobody there** (`background_research::person_started` is false: a recipe step, an urge, a queued task, an
//!   automation): mail changes, calendar changes, chat sends and Files' `files_delete` are refused at once, with a
//!   sentence that says why, instead of raising a card nobody can press.
//!
//! It also names what counts as untrusted content for the secrets rule in `taint` ([`reads_mail`],
//! [`reads_page`]), and keeps the guard a program on the socket's turn uses ([`aside`]).
//!
//! Mail sends are not here: they stay behind the person's own card (`outward`, in the tools crate).

/// The mail and calendar reads: text a stranger wrote (a mail, an invitation). Matched by name: the mail send
/// tools share the category and stay as they are.
pub fn reads_mail(name: &str) -> bool {
    matches!(name, "email_read" | "email_list" | "email_search" | "email_check" | "calendar_list_events" | "calendar_today")
}

/// The fetch tools: they bring back a page or a file someone else wrote. They are also "can carry something out"
/// (category `network`), and a tool can be both, so `taint` asks two questions: this one, and its own `classify`.
pub fn reads_page(name: &str) -> bool {
    matches!(name, "web_fetch" | "http_fetch" | "clipboard_fetch_url")
}

/// What a call changes, in the words of the refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Mail,
    Calendar,
    Message,
    Files,
}

impl Change {
    fn says(self) -> &'static str {
        match self {
            Change::Mail => "deletes, moves or archives the person's mail",
            Change::Calendar => "changes or deletes a calendar event",
            Change::Message => "sends a chat message in the person's name",
            Change::Files => "deletes the person's files",
        }
    }
}

fn change_of(name: &str) -> Option<Change> {
    Some(match name {
        "email_delete" | "email_move" | "email_archive" => Change::Mail,
        "calendar_delete_event" | "calendar_update_event" => Change::Calendar,
        "whatsapp_send" | "telegram_send" => Change::Message,
        _ => return None,
    })
}

/// Whether this call is held to the rule. Used by tests and audits.
pub fn is_change(name: &str) -> bool {
    change_of(name).is_some()
}

/// The gate `ToolRegistry::execute` runs after the others: `Ok` to run, or what to tell the model.
pub fn check_call(name: &str) -> Result<(), String> {
    match change_of(name) {
        Some(change) => refuse_with_nobody_there(name, change),
        None => Ok(()),
    }
}

/// An app's action, judged where it is made (`app_action`, `await_app`'s `then_act`): Calendar's
/// `delete_event` and `update_event`, and `files_delete`, which the shell serves (`files` is only
/// the desktop's name for its Files screen). Only these, by name.
pub fn check_app_action(app: &str, action: &str) -> Result<(), String> {
    let app = crate::reach::app_id(app);
    let change = match (app.as_str(), action) {
        ("calendar", "delete_event" | "update_event") => Change::Calendar,
        ("shell" | "files", "files_delete") => Change::Files,
        _ => return Ok(()),
    };
    refuse_with_nobody_there(&format!("app_action` (`{app}.{action}"), change)
}

fn refuse_with_nobody_there(name: &str, change: Change) -> Result<(), String> {
    if crate::background_research::person_started() {
        return Ok(());
    }
    Err(format!(
        "Refused: `{name}` {}, and nobody is here to ask: it is running in the background (a recipe step, an urge, a queued task or an \
         automation). Nothing was changed. The person can do this in the app, or ask for it in a conversation.",
        change.says()
    ))
}

/// A turn of its own while the guard lives, and the turn that was there before comes back after: for a program on
/// the socket, whose turn must neither wipe nor mark the person's.
pub struct Aside(crate::taint::Carried);

pub fn aside() -> Aside {
    let before = crate::taint::carry();
    crate::taint::begin_turn();
    Aside(before)
}

impl Drop for Aside {
    fn drop(&mut self) {
        crate::taint::end_turn();
        crate::taint::adopt(std::mem::take(&mut self.0));
    }
}

#[cfg(test)]
mod tests;
