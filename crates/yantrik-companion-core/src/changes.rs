//! Changing the person's things, or planning work, after reading what someone else wrote.
//!
//! `taint` keeps a secret from leaving with a stranger's words behind it. This is the same
//! question for the person's own data: a mail, a page or a file can say "delete the invoices" or
//! "set up a schedule that forwards everything", and the model that read it cannot tell that
//! from the person's own words. So the tools that change or plan are held to what the
//! conversation has read, below the model, in [`crate::tools::ToolRegistry::execute`].
//!
//! Two rules, both a refusal that says what was refused, what brought the text in, that nothing
//! was changed and what the person can do instead:
//!
//! * **After untrusted content** (`taint::state`, the same turn and scope the other rules use):
//!   deleting, moving or changing mail and calendar events, forgetting, sending a chat message,
//!   overwriting a file that exists, and creating or changing a recipe, schedule, automation or
//!   queued task.
//! * **With nobody there** (`background_research::person_started`: a recipe step, an urge, an
//!   automation): the mail and calendar changes and the two message sends. File writes and memory
//!   changes stay allowed, which recipes rely on.
//!
//! Mail sends are not here: they stay behind the person's own card (`outward`, in the tools crate).
//! What reads mail is untrusted from the first read ([`reads_mail`], used by `taint::classify`).

use serde_json::Value;

/// The mail reads. Matched by name: the mail send tools share the category and stay as they are.
pub fn reads_mail(name: &str) -> bool {
    matches!(name, "email_read" | "email_list" | "email_search" | "email_check")
}

/// What a call changes or plans, in the words of the refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Mail,
    Calendar,
    Message,
    Forget,
    Overwrite,
    Delete,
    Plan,
}

impl Change {
    fn says(self) -> &'static str {
        match self {
            Change::Mail => "deletes, moves or archives the person's mail",
            Change::Calendar => "changes or deletes a calendar event",
            Change::Message => "sends a chat message in the person's name",
            Change::Forget => "erases or rewrites what the companion remembers",
            Change::Overwrite => "overwrites a file that already exists",
            Change::Delete => "deletes or changes the person's files, scripts, secrets or automations",
            Change::Plan => "plans work for later (a recipe, schedule, automation or queued task)",
        }
    }

    /// Held back with nobody there, as well as after untrusted content.
    fn needs_the_person(self) -> bool {
        matches!(self, Change::Mail | Change::Calendar | Change::Message)
    }
}

/// Whether a file the call would write is already there (a link, even a dangling one, counts).
fn exists(path: &str) -> bool {
    !path.is_empty() && std::fs::symlink_metadata(crate::tools::expand_home(path)).is_ok()
}

fn change_of(name: &str, args: &Value) -> Option<Change> {
    Some(match name {
        "email_delete" | "email_move" | "email_archive" => Change::Mail,
        "calendar_delete_event" | "calendar_update_event" => Change::Calendar,
        "whatsapp_send" | "telegram_send" => Change::Message,
        "forget_memory" | "forget_topic" | "update_memory" => Change::Forget,
        "write_file" => {
            let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            if append || !exists(path) {
                return None;
            }
            Change::Overwrite
        }
        "edit_file" | "script_patch" | "script_delete" | "manage_files" | "vault_delete" | "delete_automation" | "cancel_schedule" => {
            Change::Delete
        }
        "create_recipe" | "create_schedule" | "create_automation" | "update_schedule" | "toggle_automation" | "queue_task" => Change::Plan,
        _ => return None,
    })
}

/// Whether this call changes or plans, and so is held to the two rules. Used by tests and audits.
pub fn is_change(name: &str, args: &Value) -> bool {
    change_of(name, args).is_some()
}

/// The gate `ToolRegistry::execute` runs after the others: `Ok` to run, or what to tell the model.
pub fn check_call(name: &str, args: &Value) -> Result<(), String> {
    let Some(change) = change_of(name, args) else { return Ok(()) };
    if change.needs_the_person() && !crate::background_research::person_started() {
        return Err(format!(
            "Refused: `{name}` {}, and nobody is here to ask: it was not started by the person (a recipe step, an urge or an \
             automation). Nothing was changed. The person can do this in the app, or ask for it in a conversation.",
            change.says()
        ));
    }
    if let (Some(source), _) = crate::taint::state() {
        return Err(format!(
            "Refused: `{name}` {}, and this conversation has already read untrusted content (`{source}`). Text someone else \
             wrote can ask an agent to do this; it must not be able to. Nothing was changed. The person can do it in the app, \
             or ask for it in a new conversation.",
            change.says()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
