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
//!   The mark lasts as long as the text does: the conversation (`CONVERSATION`), cleared where the history is.
//!   The person's own turn is protected at the point of effect by the approval card (#1313); until then a request
//!   the companion must look something up for is refused, and the sentence says what works instead.
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
/// The clean list is short, exact (name, category) pairs (no prefix, and a plugin that takes a clean name under
/// another category still marks), and a tool is on it only if its result is made entirely from its own
/// arguments, the clock and arithmetic. Nothing that reads a file, a store, a device, a network, a window, a
/// memory or another program's output is on it. `word_count` is not: it counts a file's words too. When unsure,
/// leave it off.
pub const CLEAN: [(&str, &str); 7] = [
    // arithmetic on the arguments
    ("calculate", "calculator"),
    ("unit_convert", "calculator"),
    // transforms of the text given
    ("base64_encode", "encoding"),
    ("base64_decode", "encoding"),
    ("url_encode", "encoding"),
    ("json_format", "encoding"),
    // the clock and the arguments
    ("date_calc", "time"),
];

/// The guarded tools whose answer is made only from their own arguments, so they do not mark: each says "created"
/// or "sent" with what the caller gave it (`create_recipe`: the name and id; `create_schedule`: the label, type,
/// next time and id; `create_automation`: the name; the two chat sends: that the message went and to whom). The
/// others echo what they found (`calendar_delete_event` and `calendar_update_event` the event's title,
/// `email_delete`, `email_move` and `email_archive` the mail's subject, `run_recipe` its steps' output), so they
/// mark, and a second guarded call in the message is refused naming the first.
const UNMARKING: [&str; 5] = ["create_recipe", "create_schedule", "create_automation", "whatsapp_send", "telegram_send"];

/// Whether a tool running marks the message as having read foreign text.
pub fn marks(name: &str, category: &str) -> bool {
    !CLEAN.contains(&(name, category)) && !UNMARKING.contains(&name)
}

/// What must happen after every call, once, from `ToolRegistry::execute`: the narrow note for the credential
/// rules, and the broad mark for the change rules.
pub fn after_call(name: &str, category: &str, args: &Value) {
    crate::taint::note_call(name, category, args);
    if marks(name, category) {
        mark(name);
    }
}

thread_local! {
    /// The first tool that marked a message of this conversation, kept until the conversation is cleared: the
    /// text is in the history the model reads in the next message, where no tool need run. Only a turn the
    /// person started writes or reads it (an urge, a recipe step or a program on the socket has its own turn).
    static CONVERSATION: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// The companion's own earlier notice, put in front of the person's words for the next turn.
    static NOTICE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The turn `taint::begin_turn` starts. A turn the person started begins with the conversation's mark, if it has one.
pub(crate) fn new_turn() -> crate::taint::Turn {
    let mut turn = crate::taint::Turn::default();
    if crate::background_research::person_started() {
        turn.broad_earlier = CONVERSATION.with(|c| c.borrow().clone());
        if turn.broad_earlier.is_none() && NOTICE.with(|n| n.replace(false)) {
            turn.broad_from = Some("the companion's own earlier notice".to_string());
        }
    }
    turn
}

/// The text a turn is given in front of the person's words that the companion wrote earlier (a proactive
/// message): the next turn is marked, naming it.
pub fn notice_in_context(text: String) -> String {
    NOTICE.with(|n| n.set(true));
    text
}

/// The conversation is cleared (a new conversation, or the idle reset): what it read is forgotten with it.
pub fn forget_conversation() {
    CONVERSATION.with(|c| *c.borrow_mut() = None);
    NOTICE.with(|n| n.set(false));
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

/// Mark this turn as having read foreign text, by `name` (also what a sub-agent ran, merged into its parent).
pub fn mark(name: &str) {
    let marked = crate::taint::TURN.with(|cell| match cell.borrow_mut().as_mut() {
        Some(turn) => {
            turn.broad_from.get_or_insert_with(|| name.to_string());
            true
        }
        None => false,
    });
    if marked && crate::background_research::person_started() {
        CONVERSATION.with(|c| {
            c.borrow_mut().get_or_insert_with(|| name.to_string());
        });
    }
}

/// What marked this message, if anything has (not what an earlier message of the conversation did).
pub fn broad() -> Option<String> {
    crate::taint::TURN.with(|cell| cell.borrow().as_ref().and_then(|t| t.broad_from.clone()))
}

/// Where the mark on this turn came from, in the words of a refusal.
fn marked_by() -> Option<String> {
    if let Some(narrow) = crate::taint::state().0 {
        return Some(format!("`{narrow}`"));
    }
    crate::taint::TURN.with(|cell| {
        let guard = cell.borrow();
        let turn = guard.as_ref()?;
        match (&turn.broad_from, &turn.broad_earlier) {
            (Some(now), _) if now.starts_with("the ") => Some(now.clone()),
            (Some(now), _) => Some(format!("`{now}`")),
            (None, Some(earlier)) => Some(format!("an earlier message in this conversation, with `{earlier}`")),
            (None, None) => None,
        }
    })
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
    let Some(change) = app_change(app, action) else { return Ok(()) };
    refuse_if_held(&format!("app_action` (`{}.{}", crate::reach::app_id(app), action), change)
}

/// The one normaliser for an app action's name: the app as `reach` names it, the action in lower case with its
/// spaces and dashes as underscores. Only these, by name.
fn app_change(app: &str, action: &str) -> Option<Change> {
    let app = crate::reach::app_id(app);
    let action = action.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match (app.as_str(), action.as_str()) {
        ("calendar", "delete_event" | "update_event") => Some(Change::Calendar),
        ("shell" | "files", "files_delete") => Some(Change::Files),
        _ => None,
    }
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
        if let Some(source) = marked_by() {
            return Err(format!(
                "Refused: `{name}` {}, and this conversation has already read text someone else wrote ({source}). That text \
                 can ask an agent to do this; it must not be able to. Nothing was changed. The person can do it in the app, or \
                 start a new conversation and ask for only that.",
                change.says()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
