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

/// Whether this tool hands a secret back to its caller.
///
/// Used by the audit log, which records what every tool returned: for these it must record that
/// something was returned and nothing of what it was.
pub fn returns_secret(name: &str, category: &str) -> bool {
    classify(name, category) == Sensitivity::ReturnsSecret
}

#[derive(Default)]
struct Turn {
    /// The tool that first brought untrusted content in, kept so a refusal can name it. A refusal
    /// that does not say what caused it is indistinguishable from a bug, and gets worked around.
    untrusted_from: Option<String>,
    secret_from: Option<String>,
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

/// Start a fresh conversation turn.
///
/// Called when the companion begins handling a message. Everything before this is forgotten:
/// the rule is about what happened *within* one exchange, because that is the span in which a
/// page can influence what the model does next.
pub fn begin_turn() {
    TURN.with(|t| *t.borrow_mut() = Some(Turn::default()));
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
}

/// What this turn has taken in, for a caller that wants to show it.
pub fn state() -> (Option<String>, Option<String>) {
    TURN.with(|cell| match cell.borrow().as_ref() {
        Some(turn) => (turn.untrusted_from.clone(), turn.secret_from.clone()),
        None => (None, None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() {
        begin_turn();
    }

    #[test]
    fn the_april_2026_attack_is_refused() {
        // The shape that hit three products at once: the agent reads something a stranger wrote,
        // the text tells it to fetch credentials, and it does.
        fresh();
        note("browse", "browser");
        let refused = check("vault_get", "vault").expect_err("a page must not be able to ask for a password");
        assert!(refused.contains("browse"), "the refusal must name what tainted the turn: {refused}");
        assert!(refused.contains("vault_get"));
    }

    #[test]
    fn fetching_a_credential_first_is_fine() {
        // The ordering that is safe, and the one a login actually uses.
        fresh();
        assert!(check("vault_get", "vault").is_ok());
        note("vault_get", "vault");
        assert!(check("browse", "browser").is_ok());
        note("browse", "browser");
    }

    #[test]
    fn reading_the_web_and_sending_a_summary_still_works() {
        // The rule has to leave the ordinary job alone, or it will be switched off. No credential
        // has entered this conversation, so there is nothing to leak.
        fresh();
        note("browse", "browser");
        note("browser_read", "browser");
        assert!(check("send_email", "email").is_ok());
        assert!(check("run_command", "system").is_ok());
    }

    #[test]
    fn the_trifecta_closing_is_refused() {
        // All three legs, in the order that matters.
        fresh();
        note("vault_get", "vault");
        note("browse", "browser");
        let refused = check("run_command", "system").expect_err("secret + untrusted + egress");
        assert!(refused.contains("vault_get"), "{refused}");
        assert!(refused.contains("browse"), "{refused}");
    }

    #[test]
    fn a_tool_that_uses_a_secret_without_revealing_it_stays_allowed() {
        // Logging in during a browsing session must remain possible. A password typed into a form
        // and never returned cannot be asked for by the page it was typed into.
        fresh();
        note("browse", "browser");
        assert!(
            check("browser_login", "browser").is_ok(),
            "blocking this would make the safe way to log in impossible, and the rule would be turned off"
        );
    }

    #[test]
    fn a_new_turn_forgets() {
        fresh();
        note("browse", "browser");
        assert!(check("vault_get", "vault").is_err());

        fresh();
        assert!(check("vault_get", "vault").is_ok(), "the rule is about one exchange, not forever");
    }

    #[test]
    fn nothing_is_refused_before_a_turn_begins() {
        // Background work and callers that never announce turns must keep working.
        TURN.with(|t| *t.borrow_mut() = None);
        assert!(check("vault_get", "vault").is_ok());
        assert!(check("run_command", "system").is_ok());
    }

    #[test]
    fn a_refusal_says_what_to_do_instead() {
        // A refusal that only says no gets worked around; one that says how to proceed gets
        // followed.
        fresh();
        note("browser_read", "browser");
        let msg = check("vault_get", "vault").unwrap_err();
        assert!(msg.contains("before browsing"), "{msg}");
    }

    #[test]
    fn the_classifier_is_specific_before_general() {
        // browser_login lives in the browser category, which is otherwise untrusted content.
        assert_eq!(classify("browser_login", "browser"), Sensitivity::UsesSecretsPrivately);
        assert_eq!(classify("browser_read", "browser"), Sensitivity::ReturnsUntrustedContent);
        assert_eq!(classify("vault_get", "vault"), Sensitivity::ReturnsSecret);
        assert_eq!(classify("list_notes", "notes"), Sensitivity::Ordinary);
    }
}
