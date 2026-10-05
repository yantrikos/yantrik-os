//! What the approval card says, in words worked out from what the app publishes and what this
//! machine established — never from what the caller wrote about its own call.
//!
//! # Why this is its own module
//!
//! Design sign-off, 4 October (design/ui-review-gpt6-astra-2026-10-02.md, item 6): the card led
//! with the caller's self-declared name, said "Approve once" for a deletion exactly as it did for
//! opening a file, and repeated the app's sentence three times under three labels. The fix is
//! wording, so it lives apart from `approvals` (what a grant is bound to) and `control_approvals`
//! (who may grant): nothing here decides anything, and every function is plain text in, plain
//! text out, so each rule is a test.
//!
//! # The one rule about the button's label
//!
//! It comes from the app's published action — its id, or the first verb phrase of its published
//! description — and never from the caller's purpose. A mind that calls `delete_event` "Tidy up"
//! gets a red button that says "Delete event". [`confirm_label`] takes the description as a
//! [`Published`], which can only be made from the card's own app-published field, so no caller
//! text can be handed to it.
//!
//! # Nothing a caller wrote draws a line of its own
//!
//! Argument values, and the names an app's index gives its handles, are text from outside — an
//! event title can come from an invitation. A newline in one would start a row in the card's own
//! voice ("Undo: possible…") and a right-to-left override would reverse every argument after it
//! (security review of #639, B1). So [`visible`] shows every control, bidi and format character as
//! an escape — `\n`, `<U+202E>` — rather than a space: the grant binds the exact bytes, and the
//! person should see that something is there.

use crate::approval_bounds;
use crate::approvals::{self, Card, Verified};
use crate::notification_sender::{bridged_by, one_line, plain};
use yantrik_ipc_transport::plain_text::is_bidi_control;

/// What the affirmative button says on anything that is not destructive.
pub const ALLOW_ONCE: &str = "Allow once";

/// What a destructive card's undo row says when the app's own sentence says so.
pub const UNDO_APP: &str = "Undo: not possible, the app says so";

/// The same row when only the caller's words said it. A caller can only add caution, and it is
/// believed in that direction (see `approvals::said`), but the card must not credit the app with
/// a sentence the app did not write.
pub const UNDO_CALLER: &str = "Undo: not possible, says the caller";

/// Verbs a published action may lead with and still name what the button does. Phrasal verbs whose
/// particle carries the meaning ("turn … off", "take … off", "throw … away") are left out on
/// purpose; for the ones kept, a phrase cut at a particle is no label at all (see [`PARTICLES`]).
const VERBS: &[&str] = &[
    "add", "apply", "archive", "cancel", "change", "clear", "close", "copy", "create", "delete",
    "disconnect", "discard", "download", "drop", "edit", "empty", "end", "erase", "forget",
    "format", "install", "kill", "leave", "move", "overwrite", "publish", "purge", "reboot",
    "remove", "rename", "replace", "reset", "restart", "restore", "revoke", "run", "save", "send",
    "set", "share", "start", "stop", "switch", "terminate", "trash", "uninstall", "update",
    "upload", "wipe", "write", "unlink",
];

/// Where a verb phrase stops: a preposition, a conjunction or a relative clause. What follows is
/// the rest of the sentence, which the card draws in full.
const STOPS: &[&str] = &[
    "by", "from", "to", "into", "onto", "in", "at", "with", "for", "and", "this", "that", "which",
    "who", "so", "then",
];

/// Words that may be a verb's particle. A phrase cut at one has lost part of its verb — "Leave
/// the radio on", "Switch off the firewall" — so it is not a label (review of #639, N4).
const PARTICLES: &[&str] = &["on", "off", "up", "down", "out", "over", "away", "back"];

/// A label longer than this does not fit the button beside Decline on a 404px card.
const LABEL_CHARS: usize = 26;

/// A phrase longer than this is not a label any more.
const LABEL_WORDS: usize = 5;

/// How much of the caller's self-declared name the claim line repeats.
const CLAIM_CHARS: usize = 40;

/// How much of an executable's file name the identity line repeats.
const EXE_CHARS: usize = 40;

/// The description of the action as the APP publishes it. Made only from the card's own
/// `purpose`, which `approvals` fills from the app's published sentence and never from the
/// caller's words, so the type is the proof that the label and the consequence verbs cannot be
/// handed caller text (review of #639, N3).
#[derive(Clone, Copy, Debug)]
pub struct Published<'a>(&'a str);

impl<'a> Published<'a> {
    /// The app's published description, from the card that carries it.
    pub fn of(card: &'a Card) -> Self {
        Published(&card.purpose)
    }

    /// A description written out in a test, standing for what an app publishes.
    #[cfg(test)]
    pub fn app(text: &'a str) -> Self {
        Published(text)
    }
}

/// `text` with every control, bidi and format character drawn as a visible escape: `\n`, `\r`,
/// `\t`, and `<U+XXXX>` for the rest — and a backslash as `\\`, so a `\n` the caller typed reads
/// `\\n` and cannot pass for a newline it sent. Nothing else changes, so what is shown is what was
/// sent.
pub fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control()
                || is_bidi_control(c)
                || approvals::is_format_char(c)
                || matches!(c, '\u{2028}' | '\u{2029}') =>
            {
                out.push_str(&format!("<U+{:04X}>", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Whether the card asks with a red, action-named button: the grade says `dangerous`, or the
/// app's sentence (or the caller's, which can only add caution) says it cannot be undone. The
/// same reading `approvals::warning_for` draws the red line from, so the two cannot disagree.
pub fn destructive(grade: &str, said: &str) -> bool {
    grade == "dangerous" || approvals::unrecoverable(said)
}

/// What the affirmative button says. Destructive: the action's own verb phrase ([`verb_phrase`]),
/// else "Allow once". Anything else: "Allow once".
pub fn confirm_label(destructive: bool, action: &str, published: Published) -> String {
    if !destructive {
        return ALLOW_ONCE.to_string();
    }
    verb_phrase(action, published).unwrap_or_else(|| ALLOW_ONCE.to_string())
}

/// The action as a short imperative, from what the app publishes:
///
/// 1. the action id when it is `<verb>_<object>` — `delete_event` → "Delete event";
/// 2. else the first verb phrase of the published description, when it starts with a verb —
///    "End a running process by pid" → "End a running process";
/// 3. else the id's verb alone — `remove` → "Remove".
///
/// `None` when neither leads with a verb this module knows, the phrase was cut at a particle, or
/// it would not fit a button.
pub fn verb_phrase(action: &str, published: Published) -> Option<String> {
    let from_id = phrase_from_id(action);
    if let Some(id) = from_id.as_ref().filter(|p| p.contains(' ')) {
        return Some(id.clone());
    }
    phrase_from_description(published.0).or(from_id)
}

fn phrase_from_id(action: &str) -> Option<String> {
    let tokens: Vec<&str> = action.split(['_', '-']).filter(|t| !t.is_empty()).collect();
    let verb = tokens.first()?.to_ascii_lowercase();
    if !VERBS.contains(&verb.as_str()) || tokens.len() > LABEL_WORDS {
        return None;
    }
    if !tokens.iter().all(|t| t.chars().all(|c| c.is_ascii_alphanumeric())) {
        return None;
    }
    if tokens.iter().skip(1).any(|t| PARTICLES.contains(&t.to_ascii_lowercase().as_str())) {
        return None;
    }
    fitting(capitalised(&tokens.join(" ").to_ascii_lowercase()))
}

fn phrase_from_description(published: &str) -> Option<String> {
    let sentence = one_line(&approvals::first_sentence(published));
    let mut words: Vec<String> = Vec::new();
    for raw in sentence.split_whitespace() {
        let word: String =
            raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'').to_string();
        let lower = word.to_lowercase();
        if !words.is_empty() && PARTICLES.contains(&lower.as_str()) {
            return None;
        }
        if word.is_empty() || (!words.is_empty() && STOPS.contains(&lower.as_str())) {
            break;
        }
        words.push(word);
        // A clause ends at its punctuation: "Download, verify, and install" is not "Download".
        if raw.ends_with([',', ';', ':', '.', ')', '\u{2014}']) {
            break;
        }
    }
    let verb = words.first()?.to_lowercase();
    if !VERBS.contains(&verb.as_str()) || words.len() > LABEL_WORDS {
        return None;
    }
    fitting(capitalised(&words.join(" ")))
}

fn fitting(label: String) -> Option<String> {
    (label.chars().count() <= LABEL_CHARS).then_some(label)
}

/// "Deletes", "Ends", "Applies": the phrase's verb, said of the action. "Acts on" when the action
/// names no verb this module knows.
pub fn verb_said(phrase: Option<&str>) -> String {
    let Some(verb) = phrase.and_then(|p| p.split_whitespace().next()) else {
        return "Acts on".to_string();
    };
    let v = capitalised(&verb.to_lowercase());
    let before_y = v.chars().rev().nth(1).unwrap_or('a');
    if v.ends_with('y') && !"aeiou".contains(before_y) {
        format!("{}ies", &v[..v.len() - 1])
    } else if ["s", "sh", "ch", "x", "z"].iter().any(|end| v.ends_with(end)) {
        format!("{v}es")
    } else {
        format!("{v}s")
    }
}

/// What changes, as the card draws it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Consequences {
    /// One line pinned above the buttons: "Deletes: <the app's name for the target>", else
    /// "Deletes: <the arguments>". Empty for a call with neither.
    pub what: String,
    /// Every argument exactly as the grant binds it, shown plainly, `; ` between them. Pinned in
    /// full, wrapped, above the buttons whenever the line above cannot carry it — it names a
    /// target, or it is cut at the card's edge — so no part of what the grant binds is ever only
    /// reachable by scrolling (review of #639). Bounded twice: a request whose rows would not fit
    /// is refused before it is asked (`approval_bounds::refusal`), and what is drawn is cut after
    /// escaping to the same limits (`approval_bounds::ROW_CHARS`, `TOTAL_CHARS`).
    pub exactly: String,
    /// "Undo: not possible, the app says so", the caller-only form, or empty.
    pub undo: String,
}

/// The card's "what changes" lines. `args` are `approvals::args_rows` (already bounded), `said` is
/// the app's sentence and the caller's together, the text the warning is read from. The verb comes
/// from the app only, and every argument and the target are passed through [`visible`] one by one,
/// before they are joined, so no value can break a line or reorder the ones after it.
pub fn consequences(action: &str, published: Published, said: &str, target: &str, args: &[String]) -> Consequences {
    let verb = verb_said(verb_phrase(action, published).as_deref());
    let none = args.is_empty() || (args.len() == 1 && args[0] == "(no arguments)");
    // Escaped value by value, then cut after escaping — each row and all of them together — to the
    // bounds the request was already checked against (approval_bounds), so the pinned lines are a
    // fixed size at worst whatever reaches here.
    let exactly = if none {
        String::new()
    } else {
        approval_bounds::clip_rows(&approval_bounds::joined(args))
    };
    let target = visible(target.trim());
    let what = match (target.is_empty(), exactly.is_empty()) {
        (false, _) => format!("{verb}: {target}"),
        (true, false) => format!("{verb}: {exactly}"),
        (true, true) => String::new(),
    };
    let undo = if approvals::unrecoverable(published.0) {
        UNDO_APP
    } else if approvals::unrecoverable(said) {
        UNDO_CALLER
    } else {
        ""
    };
    Consequences { what, exactly, undo: undo.to_string() }
}

/// The warning line, less what the undo row already says. "The app says this cannot be undone."
/// is the undo row in other words, so it is not said twice; every other warning — the grade, a
/// command that can do anything — still is.
pub fn warning_beside(warning: &str, undo: &str) -> String {
    if !undo.is_empty() && warning == "The app says this cannot be undone." {
        String::new()
    } else {
        warning.to_string()
    }
}

/// Who is asking, as this machine established it.
///
/// Worded so the line says what was checked and by what (review of the UI overhaul by GPT-6
/// Astra, B): "Caller process confirmed: bash · PID 812", not "A program (bash, pid 812) ·
/// verified", which left a person to guess what "verified" covered. Display only: how the caller
/// is identified is `caller_identity.rs`'s and the kernel's, and nothing here changes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// "Caller process confirmed: sshd-session · PID 2290461 · from a terminal": cut at the
    /// card's edge if it must, from the end, so the process and its PID go last.
    pub fact: String,
    /// "not verified" when nothing could be established; else empty, the fact saying it. Drawn
    /// beside the fact, never cut.
    pub tag: &'static str,
}

/// The first identity line, from the walk's structured findings — the kernel's pid, the
/// executable `/proc` resolved, the attached mind the shell matched by pid, whether a bare shell
/// stood in between — and never from re-reading `Verified::line`, whose label is the caller's
/// argv (`exec -a` writes it; review of #639, N2). The program is named by its executable's file
/// name, as `Verified::who` names it; the bridged desktop is "a program yantrik-ui started".
pub fn identity(verified: &Verified) -> Identity {
    // A card the shell raised itself, for a recipe step: a structured fact the shell set where it
    // built the card, never read from the line (fourth review of #639).
    if verified.raised_by_desktop {
        return Identity { fact: "Raised by this desktop (a recipe step)".to_string(), tag: "" };
    }
    if verified.pid <= 0 {
        return Identity { fact: "Caller process could not be identified".to_string(), tag: "not verified" };
    }
    let pid = verified.pid;
    let exe_path = verified.exe.strip_suffix(" (deleted)").unwrap_or(&verified.exe);
    if (verified.attached_mind.trim().is_empty() || !verified.mind_by_pid) && yantrik_ipc_transport::owner::is_installed_desktop_binary(exe_path) {
        return Identity { fact: format!("{CONFIRMED}{} \u{b7} PID {pid}", visible(&bridged_by(exe_path))), tag: "" };
    }
    let exe = clip_chars(&visible(yantrik_ipc_transport::peer_identity::basename(exe_path)), EXE_CHARS);
    let head = if exe.is_empty() { format!("{CONFIRMED}PID {pid}") } else { format!("{CONFIRMED}{exe} \u{b7} PID {pid}") };
    let mind = visible(verified.attached_mind.trim());
    // The attached mind only by the kernel's pid. A word of the program's own name that matches a
    // mind is the program's choice: said as a match, and not verified (security review of #648, M2).
    if !mind.is_empty() && !verified.mind_by_pid {
        // A script typed in a terminal keeps that note too.
        let terminal = if verified.from_terminal { " \u{b7} from a terminal" } else { "" };
        return Identity { fact: format!("{head}{terminal} \u{b7} name matches {mind}"), tag: "not verified" };
    }
    let fact = if !mind.is_empty() {
        format!("{head} \u{b7} the attached mind {mind}")
    } else if verified.from_terminal {
        format!("{head} \u{b7} from a terminal")
    } else {
        head
    };
    Identity { fact, tag: "" }
}

/// How a confirmed caller's line begins.
const CONFIRMED: &str = "Caller process confirmed: ";

/// Every character drawn as a quote mark, which a claim must not use to close the card's own.
fn is_quote(c: char) -> bool {
    matches!(
        c,
        '"' | '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' | '\u{2036}' | '\u{00ab}'
            | '\u{00bb}' | '\u{2039}' | '\u{203a}' | '\u{ff02}' | '\u{301d}' | '\u{301e}' | '\u{301f}'
            | '\u{275d}' | '\u{275e}' | '\u{2e42}' | '\u{02ee}'
    )
}

/// The second identity line, drawn under the first in amber with "· not verified" beside it: the
/// name the caller gave itself, "Claimed name: “design-sweep”". One plain line, nothing invisible left in it, cut short, and every
/// quote mark of its own turned into an apostrophe so it cannot close the quote the card puts
/// round it (review of #639, N1).
pub fn claim(requester: &str) -> String {
    let seen: String = requester.chars().filter(|c| !approvals::is_format_char(*c)).collect();
    let name: String = plain(&seen, CLAIM_CHARS).chars().map(|c| if is_quote(c) { '\'' } else { c }).collect();
    let name = name.trim();
    if name.is_empty() {
        "Claimed name: none given".to_string()
    } else {
        format!("Claimed name: \u{201c}{name}\u{201d}")
    }
}

/// "Expires in 2 min, then declined": the countdown in whole minutes, so nothing on the card
/// ticks. Under a minute it says so rather than counting seconds at the person.
pub fn expires_text(left_secs: u64) -> String {
    if left_secs >= 60 {
        format!("Expires in {} min, then declined", left_secs.div_ceil(60))
    } else {
        "Expires in under a minute, then declined".to_string()
    }
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\u{2026}")
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "approval_wording_tests.rs"]
mod approval_wording_tests;
