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
//! * **The same turn** (the broad mark below, with what a sub-agent ran merged back into its parent): mail changes, calendar changes, chat sends, the tools that create a recipe, a
//!   schedule or an automation, and `run_recipe` are refused once any tool that cannot be shown to return only its own arguments has run in the turn.
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

/// Why there are two marks on a turn. `taint`'s `untrusted_from` is the narrow one: it names the tools known to
/// return text someone else wrote, and feeds the credential rules (a secret after untrusted content, sending out
/// after both), which are a different judgement and are not widened here. The change rules below use the broad
/// one, `broad_from` (kept beside it in `taint`'s `Turn`, so a new turn, a sub-agent's carry and an outside turn's
/// restore treat it the same): once any tool has run that is not on [`CLEAN`], the message has read foreign text.
/// Unknown means foreign, because a list of sources must name every danger (fetches, downloads, GitHub, ssh and
/// command output, docker logs, a git log, an MCP server's reply, a file downloaded yesterday, the next tool
/// added) and fails open the first time it misses one.
///
/// The clean list is short, exact names (no category, no prefix), and a tool is on it only if its result is made
/// entirely from its own arguments, the clock and arithmetic. Nothing that reads a file, a store, a device, a
/// network, a window, a memory or another program's output is on it. When unsure, leave it off.
pub const CLEAN: [&str; 8] = [
    // arithmetic on the arguments
    "calculate",
    "unit_convert",
    // transforms of the text given
    "base64_encode",
    "base64_decode",
    "url_encode",
    "json_format",
    "word_count",
    // the clock and the arguments
    "date_calc",
];

/// Whether a tool running marks the message as having read foreign text: anything not on [`CLEAN`] and not one
/// of the guarded change and planning tools themselves (their result is their own confirmation). `run_recipe`
/// is guarded but marks, because its result is what its steps returned.
pub fn marks(name: &str) -> bool {
    !CLEAN.contains(&name) && !(change_of(name).is_some() && name != "run_recipe")
}

/// After a call: the first tool that marks the turn is kept, to be named in a refusal.
pub fn note_call(name: &str, args: &Value) {
    if marks(name) && !is_named_app_action(name, args) {
        mark(name);
    }
}

/// `app_action` naming one of the guarded app actions: its result is its own confirmation, as the other guarded tools'.
fn is_named_app_action(name: &str, args: &Value) -> bool {
    let word = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").trim().to_ascii_lowercase().replace(['_', ' '], "-");
    name == "app_action" && app_change(&crate::reach::app_id(&word("app")), &word("action").replace('-', "_")).is_some()
}

fn app_change(app: &str, action: &str) -> Option<Change> {
    match (app, action) {
        ("calendar", "delete_event" | "update_event") => Some(Change::Calendar),
        ("shell" | "files", "files_delete") => Some(Change::Files),
        _ => None,
    }
}

/// Mark this turn as having read foreign text, by `name` (what a sub-agent ran, merged into its parent).
pub fn mark(name: &str) {
    crate::taint::TURN.with(|cell| {
        if let Some(turn) = cell.borrow_mut().as_mut() {
            turn.broad_from.get_or_insert_with(|| name.to_string());
        }
    });
}

/// What marked this turn, if anything has.
pub fn broad() -> Option<String> {
    crate::taint::TURN.with(|cell| cell.borrow().as_ref().and_then(|t| t.broad_from.clone()))
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
/// `delete_event` and `update_event`, and `files_delete`, which the shell serves (`files` is only
/// the desktop's name for its Files screen). Only these, by name.
pub fn check_app_action(app: &str, action: &str) -> Result<(), String> {
    let app = crate::reach::app_id(app);
    let Some(change) = app_change(&app, action) else { return Ok(()) };
    refuse_if_held(&format!("app_action` (`{app}.{action}"), change)
}

fn refuse_if_held(name: &str, change: Change) -> Result<(), String> {
    if change.needs_the_person() && !crate::background_research::person_started() {
        return Err(format!(
            "Refused: `{name}` {}, and nobody is here to ask: it is running in the background (a recipe step, an urge, a queued task or an \
             automation). Nothing was changed. The person can do this in the app, or ask for it in a conversation.",
            change.says()
        ));
    }
    if change.after_untrusted() {
        if let Some(source) = crate::taint::state().0.or_else(broad) {
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
