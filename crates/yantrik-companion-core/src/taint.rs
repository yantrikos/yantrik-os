//! Keeping secrets and untrusted content out of the same conversation.
//!
//! # The failure this exists for
//!
//! In April 2026 the same bug landed in three products at once: Claude Code Security Review,
//! Gemini CLI Action and GitHub Copilot Agent were each made to exfiltrate repository and API
//! secrets by prompt injection hidden in pull request titles and issue bodies. Nobody had a bug in
//! their sandbox. The agents did exactly what the text in front of them said.
//!
//! The pattern has a name — the lethal trifecta — and it needs three legs:
//!
//!   1. access to private data
//!   2. exposure to untrusted content
//!   3. the ability to communicate outward
//!
//! Yantrik has all three. It holds a credential vault and a memory of everything you have ever
//! told it; it reads web pages, foreign application windows, email bodies and files; and it drives
//! a browser, a shell and a network stack. Any one of those is fine. The three together, in one
//! conversation, is the shape that gets exploited.
//!
//! # Why this is not "be careful in the prompt"
//!
//! Because the model is the thing being attacked. A rule the model is asked to follow is a rule
//! the attacker gets to argue with, and the attacker writes the page. This lives at
//! [`crate::tools::ToolRegistry::execute`] — below the model, where a refusal is not negotiable.
//!
//! # What it refuses, and what it deliberately does not
//!
//! A blanket "no network after reading a page" would be safe and useless: *read this article and
//! email me a summary* is the job. So the rule is narrower, and tracks two things separately —
//! whether a secret has entered the conversation, and whether untrusted content has:
//!
//! **A secret-returning tool, after untrusted content.** By then the model may be acting on
//! instructions it read rather than instructions it was given, and asking for a credential is the
//! first move of the attack. Refused.
//!
//! **An outbound tool, once both have happened.** This is the trifecta closing. Refused.
//!
//! Everything else runs. Reading ten pages is fine. Fetching a credential and using it is fine.
//! Emailing a summary of a page you just read is fine. What is not fine is the specific ordering
//! that lets a page tell the agent to go and get your password.
//!
//! # The exemption that matters
//!
//! A tool that *uses* a secret without returning it — types a password into a login form and
//! reports only "signed in" — never puts the secret where a hostile page can ask for it. Those are
//! [`Sensitivity::UsesSecretsPrivately`] and they stay allowed, because the safe way to log in
//! during a browsing session has to remain possible or the rule will simply be turned off.

use std::cell::RefCell;

/// What a tool does that this policy cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    /// Nothing of interest. Almost everything.
    Ordinary,
    /// Returns content someone else wrote: a web page, a foreign window's UI, an email body, a
    /// file. The model cannot tell instructions in that text from instructions from the user, and
    /// neither can we.
    ReturnsUntrustedContent,
    /// Puts a secret into the conversation, where anything downstream can read it — including the
    /// model's next message and the memory it gets written to.
    ReturnsSecret,
    /// Uses a secret without revealing it. Safe during a browsing session, which is the point.
    UsesSecretsPrivately,
    /// Can send data somewhere it will not come back from.
    SendsOutward,
}

/// The policy, in one place on purpose.
///
/// Spread across the tools as a trait method this would be forty-seven files to audit and one
/// forgotten `impl` away from a hole. Here it can be read end to end in a minute, which is the
/// only way anyone will ever check it.
///
/// Matched most specific first: an exact tool name beats its category.
fn classify(name: &str, category: &str) -> Sensitivity {
    // ── Tools that hand a secret to the model ──
    if matches!(name, "vault_get" | "vault_search" | "read_env" | "get_credential") {
        return Sensitivity::ReturnsSecret;
    }
    // ── Tools that use one without telling ──
    if matches!(name, "browser_login" | "vault_fill") {
        return Sensitivity::UsesSecretsPrivately;
    }

    // ── Tools that read what someone else wrote ──
    if matches!(
        name,
        "browse"
            | "browser_read"
            | "browser_snapshot"
            | "browser_see"
            | "browser_tabs"
            | "web_search"
            | "browser_search"
            | "read_file"
            | "grep"
            | "describe_window"
            | "list_readable_windows"
            | "analyze_screen"
            | "describe_image"
            | "read_clipboard"
    ) {
        return Sensitivity::ReturnsUntrustedContent;
    }
    // A whole category of them. `email` bodies and `rss` items are written by strangers by
    // definition; the browser category is other people's pages almost entirely.
    if matches!(category, "browser" | "vision" | "rss") {
        return Sensitivity::ReturnsUntrustedContent;
    }

    // ── Tools that can carry something out ──
    if matches!(
        name,
        "run_command"
            | "script_run"
            | "run_background"
            | "code_execute"
            | "claude_code"
            | "ssh_run"
            | "send_email"
            | "browser_type"
            | "browser_type_element"
            | "browser_type_xy"
            | "http_request"
            | "post_webhook"
            | "telegram_send"
    ) {
        return Sensitivity::SendsOutward;
    }
    if matches!(category, "network" | "networking" | "ssh" | "github" | "home_assistant") {
        return Sensitivity::SendsOutward;
    }

    Sensitivity::Ordinary
}

/// Whether a tool reads something of the person's: their memory, mail, files, clipboard, calendar,
/// terminal, GitHub (their token), home, tasks and schedules, or a page in the companion's browser,
/// which is signed in as them (`browser_login`): its webmail is theirs, not a stranger's page
/// (#1063 review, M5). After one of these, a read the model aims at an address it chose can carry
/// what it read in the address itself, and is asked about (`outward::reading`).
pub fn reads_private(name: &str, category: &str) -> bool {
    matches!(
        category,
        "memory" | "memory_hygiene" | "knowledge" | "life_management" | "life_assistant" | "email" | "calendar" | "clipboard" | "vault"
            | "files" | "terminal" | "project" | "workspace" | "vision" | "github"
    ) && !matches!(name, "remember" | "save_user_fact" | "set_reminder")
        || matches!(name, "read_file" | "list_files" | "read_clipboard" | "recall" | "describe_window" | "list_readable_windows" | "ha_get_state")
        || matches!(name, "list_tasks" | "list_schedules" | "browse" | "browser_read" | "browser_snapshot" | "browser_see" | "browser_screenshot")
        || matches!(name, "browser_tabs" | "browser_login" | "vault_fill")
}

/// Whether this tool hands a secret back to its caller.
///
/// Used by the audit log, which records what every tool returned: for these it must record that
/// something was returned and nothing of what it was.
pub fn returns_secret(name: &str, category: &str) -> bool {
    classify(name, category) == Sensitivity::ReturnsSecret
}

#[derive(Clone, Default)]
struct Turn {
    /// The tool that first brought untrusted content in, kept so a refusal can name it. A refusal
    /// that does not say what caused it is indistinguishable from a bug, and gets worked around.
    untrusted_from: Option<String>,
    secret_from: Option<String>,
    /// The tool that first put something of the person's into the conversation: their memory, mail,
    /// files, clipboard, calendar or terminal ([`reads_private`]). Separate from `secret_from`
    /// (credentials) because it does not stop a tool running; it decides whether a read aimed at an
    /// address the model chose is asked about (#1063).
    private_from: Option<String>,
    /// Addresses this turn has already read from, so an address that carries nothing in its query
    /// and was already visited is not asked about again.
    visited: Vec<String>,
    /// The turn was asked for by a program on the companion's socket (`mark_outside`).
    outside_from: Option<String>,
}

thread_local! {
    /// Per thread, not per process, and that is a design decision rather than a convenience.
    ///
    /// A turn belongs to the thread handling it. The companion's worker runs one conversation at
    /// a time, so the state follows the conversation exactly — while background cognition, which
    /// runs on its own thread, cannot taint the user's conversation with something it read, and
    /// cannot be tainted by it. Two conversations that never share a thread never share a verdict.
    ///
    /// It also means the tests below are independent of each other, which a process-wide mutex
    /// would not have given: each test thread gets its own turn, and they passed under the old
    /// design partly by scheduling luck.
    static TURN: RefCell<Option<Turn>> = const { RefCell::new(None) };
}

/// Whether a tool reads, writes or uses a secret: the vault's own tools, and any that hand a
/// credential to the model or use one without saying. A program that reaches the companion from
/// outside it (its socket) may run none of these: nothing outside the companion needs the vault,
/// and one call to `vault_get` read every secret in it (security review, 30 Sep 2026).
pub fn touches_secrets(name: &str, category: &str) -> bool {
    category == "vault" || matches!(classify(name, category), Sensitivity::ReturnsSecret | Sensitivity::UsesSecretsPrivately)
}

/// Whether a tool reads or changes the vault, or hands a credential to the model. A recipe may run
/// none of these; it may still use a stored password without seeing it (`browser_login`, which
/// types it only into the site it was saved for).
pub fn reads_or_writes_secrets(name: &str, category: &str) -> bool {
    category == "vault" || classify(name, category) == Sensitivity::ReturnsSecret
}

/// The tools a program outside the companion (one on its socket) may have run for it, directly
/// or in a turn it asked for: those that hold nothing of the person's and reach nothing of theirs.
/// Arithmetic, encodings, dates, the weather and a web search.
///
/// An allow-list, because four reviews of a deny-list (30 Sep - 1 Oct 2026) each found another
/// door: the vault, the clipboard, the shell's chat, then the screen, terminals, email, files and
/// memory. Everything not named here is the person's until it is shown not to be.
pub const OUTSIDE_CATEGORIES: &[&str] = &["calculator", "encoding", "time", "weather"];
/// Single tools allowed beside `OUTSIDE_CATEGORIES`, whose category also holds the person's
/// things (`web_search` sits with the browser, which holds their logged-in pages).
pub const OUTSIDE_TOOLS: &[&str] = &["web_search"];

/// Why a program outside the companion may never have this tool run for it, or `None` when it
/// may (`OUTSIDE_CATEGORIES`, `OUTSIDE_TOOLS`). Outside callers are held to `OUTSIDE_CEILING` too.
pub fn outside_refusal(name: &str, category: &str) -> Option<String> {
    if OUTSIDE_TOOLS.contains(&name) || (OUTSIDE_CATEGORIES.contains(&category) && !touches_secrets(name, category)) {
        return None;
    }
    Some(format!(
        "`{name}` is not run for a program on the companion's socket: it may only use what holds nothing of the person's ({}, {}).",
        OUTSIDE_CATEGORIES.join(", "),
        OUTSIDE_TOOLS.join(", ")
    ))
}

/// The highest grade a program on the companion's socket reaches, directly or through a turn it
/// asked for: what a phone's turn reaches. It reads and changes nothing.
pub const OUTSIDE_CEILING: crate::permission::PermissionLevel = crate::permission::PermissionLevel::Safe;

/// This turn was asked for by a program outside the companion (on its socket), not the person.
/// Its words count as untrusted from the first one, and nothing in `outside_refusal` runs in it,
/// whatever the words say. Call after `begin_turn`.
pub fn mark_outside(source: &str) {
    TURN.with(|cell| {
        if let Some(turn) = cell.borrow_mut().as_mut() {
            turn.outside_from = Some(source.to_string());
            if turn.untrusted_from.is_none() {
                turn.untrusted_from = Some(source.to_string());
            }
        }
    });
}

/// This thread's turn, to be carried onto a thread that works for it (`adopt`). The rule is per
/// thread, so a sub-agent started without it would begin with nothing read and nothing refused:
/// an outside turn's sub-agent could have read the vault (security review, 30 Sep 2026).
#[derive(Clone, Default)]
pub struct Carried(Option<Turn>);

/// What this thread's turn has taken in, for `adopt` on another thread.
pub fn carry() -> Carried {
    TURN.with(|cell| Carried(cell.borrow().clone()))
}

/// Continue a turn carried from another thread: what it read and who asked for it hold here too.
pub fn adopt(carried: Carried) {
    TURN.with(|cell| *cell.borrow_mut() = carried.0);
}

/// End the turn: what it read, and who asked for it, do not carry into whatever this thread does
/// next.
pub fn end_turn() {
    TURN.with(|t| *t.borrow_mut() = None);
}

/// Start a fresh conversation turn.
///
/// Called when the companion begins handling a message. Everything before this is forgotten:
/// the rule is about what happened *within* one exchange, because that is the span in which a
/// page can influence what the model does next.
pub fn begin_turn() {
    TURN.with(|t| *t.borrow_mut() = Some(Turn::default()));
}

/// The tool a call amounts to, for the tools whose reach depends on their arguments.
///
/// `app_action` drives any app on the desktop, so its name says nothing: driving the browser's
/// `go` or `type` sends as `browser_type` does, reading a page brings in what a stranger wrote as
/// `browser_read` does, and the shell's `decide` and the hand-offs to other minds carry text out
/// of this conversation (security review, 29 Sep 2026). Every other call is itself.
pub fn effective<'a>(name: &'a str, category: &'a str, args: &serde_json::Value) -> (&'a str, &'a str) {
    if name != "app_action" {
        return (name, category);
    }
    let fold = |k: &str| args.get(k).and_then(serde_json::Value::as_str).unwrap_or("").trim().to_ascii_lowercase().replace(['_', ' '], "-");
    let app = fold("app");
    let app = app.strip_prefix("app-").unwrap_or(&app).to_string();
    let action = fold("action").replace('-', "_");
    match app.as_str() {
        "browser" | "chromium" => match action.as_str() {
            "read" | "text" | "find" | "tabs" | "look" | "where" | "watch" | "changes" | "wait" | "scroll" | "media" => {
                ("browser_read", "browser")
            }
            // Not the browser category: that reads as untrusted content before anything else.
            _ => ("browser_type", "network"),
        },
        "shell" => match action.as_str() {
            "decide" | "send_message" | "send_to_agent" | "new_agent" | "hand_off" => ("http_request", "network"),
            // A picture of a display is a read of it, and holds more than its text; so is a
            // saved one, opened again (security review of #828, finding 1).
            "read_screen" | "read_mind_view" | "screenshot_screen" | "screenshot_mind_view" | "open_capture" => {
                ("analyze_screen", "vision")
            }
            _ => (name, category),
        },
        _ => (name, category),
    }
}

/// `check` for one call, its arguments read by `effective`.
pub fn check_call(name: &str, category: &str, args: &serde_json::Value) -> Result<(), String> {
    let outside = TURN.with(|cell| cell.borrow().as_ref().and_then(|t| t.outside_from.clone()));
    if let Some(source) = outside {
        if let Some(why) = outside_refusal(name, category) {
            return Err(format!("Refused: {why} This was asked for by {source}."));
        }
    }
    let (as_name, as_category) = effective(name, category, args);
    check(as_name, as_category).map_err(|why| if as_name == name { why } else { format!("{why} (`{name}` here is `{as_name}`)") })
}

/// `note` for one call, its arguments read by `effective`.
pub fn note_call(name: &str, category: &str, args: &serde_json::Value) {
    let (as_name, as_category) = effective(name, category, args);
    note(as_name, as_category);
}

/// Whether a tool may run, and why not.
pub fn check(name: &str, category: &str) -> Result<(), String> {
    TURN.with(|cell| {
        let guard = cell.borrow();
        let Some(turn) = guard.as_ref() else {
            // No turn has begun on this thread, so nothing has been read. This is the path for
            // background work and for callers that never announce turns.
            return Ok(());
        };
        check_against(turn, name, category)
    })
}

fn check_against(turn: &Turn, name: &str, category: &str) -> Result<(), String> {
    if let Some(source) = &turn.outside_from {
        if let Some(why) = outside_refusal(name, category) {
            return Err(format!("Refused: {why} This turn was asked for by {source}."));
        }
    }
    match classify(name, category) {
        Sensitivity::ReturnsSecret => {
            if let Some(source) = &turn.untrusted_from {
                return Err(format!(
                    "Refused: `{name}` returns a credential, and this conversation has already \
                     read untrusted content (`{source}`). A page can ask an agent to fetch a \
                     password; it must not be able to get one. Fetch what you need before \
                     browsing, or use a tool that uses the credential without revealing it."
                ));
            }
        }
        Sensitivity::SendsOutward => {
            if let (Some(untrusted), Some(secret)) = (&turn.untrusted_from, &turn.secret_from) {
                return Err(format!(
                    "Refused: `{name}` can send data outward, and this conversation holds both a \
                     credential (from `{secret}`) and untrusted content (from `{untrusted}`). \
                     That combination is how agents are made to leak secrets. Start a new \
                     conversation for this step."
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Record what a tool just did, after it has run.
pub fn note(name: &str, category: &str) {
    TURN.with(|cell| {
        let mut guard = cell.borrow_mut();
        let Some(turn) = guard.as_mut() else { return };
        remember(turn, name, category);
    });
}

fn remember(turn: &mut Turn, name: &str, category: &str) {
    match classify(name, category) {
        Sensitivity::ReturnsUntrustedContent => {
            // The first one is kept, not the last: what matters is when the conversation stopped
            // being trustworthy, and a refusal should name the thing that started it.
            if turn.untrusted_from.is_none() {
                turn.untrusted_from = Some(name.to_string());
                tracing::debug!(tool = name, "conversation now holds untrusted content");
            }
        }
        Sensitivity::ReturnsSecret => {
            if turn.secret_from.is_none() {
                turn.secret_from = Some(name.to_string());
                tracing::debug!(tool = name, "conversation now holds a credential");
            }
        }
        _ => {}
    }
    if turn.private_from.is_none() && reads_private(name, category) {
        turn.private_from = Some(name.to_string());
        tracing::debug!(tool = name, "conversation now holds something of the person's");
    }
}

/// What the conversation holds of the person's that an address could carry out, if anything: the
/// first credential or private read, named. `None` only in a clean turn. With no turn begun on this
/// thread nothing was tracked, so it is not known to be clean: background work (proactive
/// messages, scheduled runs) reads mail and memory with no turn around it (#1063 review, M5).
pub fn holds_private_data() -> Option<String> {
    TURN.with(|cell| match cell.borrow().as_ref() {
        Some(t) => t.secret_from.clone().or_else(|| t.private_from.clone()),
        None => Some(NO_TURN.to_string()),
    })
}

/// What [`holds_private_data`] names when no turn was begun, so nothing was tracked.
pub const NO_TURN: &str = "work outside any conversation turn, where what was read is not tracked";

/// Whether this turn has already read from `address`.
pub fn has_visited(address: &str) -> bool {
    TURN.with(|cell| cell.borrow().as_ref().is_some_and(|t| t.visited.iter().any(|v| v == address)))
}

/// Remember that this turn read from `address`.
pub fn note_visited(address: &str) {
    TURN.with(|cell| {
        if let Some(turn) = cell.borrow_mut().as_mut() {
            if !turn.visited.iter().any(|v| v == address) {
                turn.visited.push(address.to_string());
            }
        }
    });
}

/// What this turn has taken in, for a caller that wants to show it.
pub fn state() -> (Option<String>, Option<String>) {
    TURN.with(|cell| match cell.borrow().as_ref() {
        Some(turn) => (turn.untrusted_from.clone(), turn.secret_from.clone()),
        None => (None, None),
    })
}

#[cfg(test)]
#[path = "taint_tests.rs"]
mod tests;
