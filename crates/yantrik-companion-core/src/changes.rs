//! Changing the person's mail and calendar, sending chat messages, or planning work, when the
//! person is not the one asking, or when this message has just read someone else's text.
//!
//! `taint` keeps a secret from leaving with a stranger's words behind it. This is the same question
//! for the person's own data: a mail, a page or a file can say "delete the invoices", and the model
//! that read it cannot tell that from the person. It lives at [`crate::tools::ToolRegistry::execute`],
//! below the model. Only rules that need no tracking across turns are here:
//!
//! * **Nobody there** (`background_research::person_started` is false: a recipe step, an urge, an
//!   automation): mail changes, calendar changes, chat sends and Files' `files_delete` are refused.
//! * **The same turn** (what `taint` already tracks per turn, with what a sub-agent read merged back
//!   into its parent): mail changes, calendar changes, chat sends, the tools that create a recipe, a
//!   schedule or an automation, and `run_recipe` are refused once the turn has read untrusted content.
//!   The refusal says its scope: it is about what *this message* read. A second message is not held
//!   back by it; the person's own turn is protected at the point of effect by the approval card.
//!
//! Mail sends are not here: they stay behind the person's own card (`outward`, in the tools crate).
//! What reads mail or the calendar is an untrusted source ([`reads_mail`], used by `taint::classify`).

use serde_json::Value;

/// The mail and calendar reads: text a stranger wrote (a mail, an invitation). Matched by name: the
/// mail send tools share the category and stay as they are.
pub fn reads_mail(name: &str) -> bool {
    matches!(name, "email_read" | "email_list" | "email_search" | "email_check" | "calendar_list_events" | "calendar_today")
}

/// What a call changes or plans, in the words of the refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Mail,
    Calendar,
    Message,
    Files,
    Plan,
}

impl Change {
    fn says(self) -> &'static str {
        match self {
            Change::Mail => "deletes, moves or archives the person's mail",
            Change::Calendar => "changes or deletes a calendar event",
            Change::Message => "sends a chat message in the person's name",
            Change::Files => "deletes the person's files",
            Change::Plan => "plans or starts work for later (a recipe, schedule or automation)",
        }
    }

    /// Refused with nobody there. Everything but planning (which an unattended recipe may do).
    fn needs_the_person(self) -> bool {
        !matches!(self, Change::Plan)
    }

    /// Refused after the turn read untrusted content. Not file deletion: that waits for the card.
    fn after_untrusted(self) -> bool {
        !matches!(self, Change::Files)
    }
}

fn change_of(name: &str) -> Option<Change> {
    Some(match name {
        "email_delete" | "email_move" | "email_archive" => Change::Mail,
        "calendar_delete_event" | "calendar_update_event" => Change::Calendar,
        "whatsapp_send" | "telegram_send" => Change::Message,
        "create_recipe" | "create_schedule" | "create_automation" | "run_recipe" => Change::Plan,
        _ => return None,
    })
}

/// Whether this call is held to the rules. Used by tests and audits.
pub fn is_change(name: &str) -> bool {
    change_of(name).is_some()
}

/// The gate `ToolRegistry::execute` runs after the others: `Ok` to run, or what to tell the model.
pub fn check_call(name: &str) -> Result<(), String> {
    match change_of(name) {
        Some(change) => refuse_if_held(name, change),
        None => Ok(()),
    }
}

/// An app's action, judged where it is made (`app_action`, `await_app`'s `then_act`): Calendar's
/// `delete_event` and `update_event`, and Files' `files_delete`. Only these, by name.
pub fn check_app_action(app: &str, action: &str) -> Result<(), String> {
    let app = crate::reach::app_id(app);
    let change = match (app.as_str(), action) {
        ("calendar", "delete_event" | "update_event") => Change::Calendar,
        ("files", "files_delete") => Change::Files,
        _ => return Ok(()),
    };
    refuse_if_held(&format!("app_action` (`{app}.{action}"), change)
}

fn refuse_if_held(name: &str, change: Change) -> Result<(), String> {
    if change.needs_the_person() && !crate::background_research::person_started() {
        return Err(format!(
            "Refused: `{name}` {}, and nobody is here to ask: it was not started by the person (a recipe step, an urge or an \
             automation). Nothing was changed. The person can do this in the app, or ask for it in a conversation.",
            change.says()
        ));
    }
    if change.after_untrusted() {
        if let (Some(source), _) = crate::taint::state() {
            return Err(format!(
                "Refused: `{name}` {}, and this message has already read untrusted content (`{source}`). Text someone else \
                 wrote can ask an agent to do this; it must not be able to. Nothing was changed. The person can do it in the \
                 app, or ask again in their next message.",
                change.says()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
