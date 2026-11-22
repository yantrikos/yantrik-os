//! What a request may be, so that the approval card can show all of it — and the refusal when it
//! cannot.
//!
//! # Why this exists
//!
//! Third security review of #639: the card pins every argument above its buttons, which is what
//! keeps a mind from hiding `&& rm -rf ~/x` behind a cut line — and it made the arguments' size the
//! card's size. A value is cut to sixty characters before its control characters are shown as
//! escapes, and each escape is up to nine characters (`<U+E0041>`); a key is never cut at all. So a
//! caller could send arguments that push Decline and Allow off the screen, and hold it: the shell
//! keeps itself in front while a card waits, and an expiry is not a denial.
//!
//! Two layers, both here:
//!
//! 1. [`refusal`] — at request time, before anything is raised or sent: a request the card could
//!    not show in full is not asked at all, and the caller is told which rule it broke. It changes
//!    what may be ASKED, never what a grant is: binding, single use, the TTLs and spending are the
//!    approvals store's and are untouched.
//! 2. [`clip_row`] and [`clip_rows`] — at drawing time, after escaping, keys included, to the same
//!    limits, so the card's worst case is a fixed size whatever reaches it.

use crate::approval_wording::visible;
use crate::approvals;

/// The longest argument name a card shows. Published parameter names on this desktop are a word
/// or two (`pid`, `channel`, `args_json`).
pub const KEY_CHARS: usize = 40;

/// The longest one argument row may be once escaped: `key: value`, a forty-character key and the
/// longest value a card that shows its arguments whole carries.
pub const ROW_CHARS: usize = KEY_CHARS + 2 + approvals::WHOLE_VALUE_CHARS;

/// What all the rows together may come to once escaped. Eight sixty-character values under short
/// names (the most `args_rows` shows of ordinary arguments, ~580 characters) fit under it; at the
/// card's 14px on its ~380px width it is about twelve lines (~220px). The card itself keeps its
/// buttons inside whatever room its host gives it, at any screen size — it scrolls the rest and
/// holds Allow until the end has been in view (intent_lens.slint, `allow-ready`); this bound is
/// what keeps that rare.
pub const TOTAL_CHARS: usize = 640;

/// The longest command a card for an action that runs whatever it is given shows, whole, in its
/// scrolling box (#1353). Its row is not counted in [`TOTAL_CHARS`]; every other argument is.
pub const COMMAND_CHARS: usize = 4000;

/// The parameters an action that runs whatever it is given takes its command in. The room goes to
/// one of these and to nothing else: never to "the longest argument", whatever the caller names
/// (round 2 of #1357).
pub const COMMAND_PARAMS: [&str; 2] = ["command", "text"];

/// The argument that is the command: the longest of [`COMMAND_PARAMS`] present as a string.
pub fn command_key(args: &serde_json::Value) -> Option<String> {
    let map = args.as_object()?;
    COMMAND_PARAMS.iter().filter(|k| map.get(**k).is_some_and(|v| v.is_string())).max_by_key(|k| visible(map[**k].as_str().unwrap_or("")).chars().count()).map(|k| k.to_string())
}

/// Which of the drawn `key: value` rows is the command (the longest [`COMMAND_PARAMS`] row).
pub fn command_row(rows: &[String]) -> Option<usize> {
    let named = |r: &String| COMMAND_PARAMS.iter().any(|k| r.starts_with(&format!("{k}: ")));
    (0..rows.len()).filter(|i| named(&rows[*i])).max_by_key(|i| rows[*i].chars().count())
}

/// What a mind is told when its command is past [`COMMAND_CHARS`].
const SPLIT_ADVICE: &str = "split it into shorter commands, or write it to a file in a visible folder and run that file";

/// Why a request cannot be put in front of a person in full, or `None` when it can.
///
/// - (a) an argument name longer than [`KEY_CHARS`], or with a character [`visible`] would escape;
/// - (b) an argument the app does not publish, when the app publishes its parameters — the
///   action's own dispatch would refuse the call anyway (`check_arguments`);
/// - (c) the rows, escaped, longer than [`TOTAL_CHARS`] together;
/// - (d) on a card that shows its arguments whole (`whole`: graded dangerous, cannot be undone, or
///   runs whatever it is given — `approvals::shown_whole`), more than `ARG_ROWS` arguments, or a
///   value longer than `approvals::WHOLE_VALUE_CHARS` once escaped. Such a card has no "and N
///   more" and no "(N characters in full)": a cut-off part is exactly where `; rm -rf ~` goes, so
///   it is shown whole or not asked about. An ordinary card keeps the cut under its marker.
pub fn refusal(args: &serde_json::Value, params: Option<&[String]>, whole: bool) -> Option<String> {
    refusal_for(args, params, whole, false)
}

/// [`refusal`], where `open` says the app's own published sentence declares that the action runs
/// whatever it is given (never the caller's words): its command ([`command_key`]) is shown whole
/// up to [`COMMAND_CHARS`] and left out of the [`TOTAL_CHARS`] count; past that it is refused with
/// [`SPLIT_ADVICE`]. Every other argument keeps its own bound.
pub fn refusal_for(args: &serde_json::Value, params: Option<&[String]>, whole: bool, open: bool) -> Option<String> {
    let drawn = |v: &serde_json::Value| visible(&match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    });
    let command = command_key(args).filter(|_| open);
    if let Some(map) = args.as_object() {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for key in &keys {
            if key.chars().count() > KEY_CHARS {
                return Some(format!("an argument name is {} characters long, and a card shows names of at most {KEY_CHARS}", key.chars().count()));
            }
            if visible(key) != **key {
                return Some(format!("the argument name `{}` carries characters a card can only show as escapes", visible(key)));
            }
            if let Some(published) = params {
                if !published.iter().any(|p| p == *key) {
                    return Some(format!("`{key}` is not an argument the app publishes for this action"));
                }
            }
        }
        if whole {
            if keys.len() > approvals::ARG_ROWS {
                return Some(format!(
                    "there are {} arguments, and a card for an action that cannot be taken back or runs whatever it is \
                     given shows every one of them or none (at most {})",
                    keys.len(),
                    approvals::ARG_ROWS
                ));
            }
            for key in &keys {
                let escaped = drawn(&map[*key]).chars().count();
                if command.as_deref() == Some(key.as_str()) && escaped > COMMAND_CHARS {
                    return Some(format!("`{key}` is {escaped} characters as the card would draw it and a card shows a command of at most {COMMAND_CHARS}: {SPLIT_ADVICE}"));
                }
                if escaped > approvals::WHOLE_VALUE_CHARS && command.as_deref() != Some(key.as_str()) {
                    return Some(format!(
                        "`{key}` is {escaped} characters as the card would draw it, and on a card for an action that \
                         cannot be taken back or runs whatever it is given a value is shown whole or not asked about \
                         (at most {})",
                        approvals::WHOLE_VALUE_CHARS
                    ));
                }
            }
        }
    }
    let value_chars = if whole { approvals::WHOLE_VALUE_CHARS } else { approvals::ARG_VALUE_CHARS };
    // Measured exactly as the card will draw it: the same rows, the same escapes, the same join.
    // The command's own row is not part of it: it has its box.
    let counted = match (&command, args.as_object()) {
        (Some(key), Some(map)) => serde_json::Value::Object(map.iter().filter(|(k, _)| *k != key).map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => args.clone(),
    };
    let total = joined(&approvals::args_rows_with(&counted, value_chars)).chars().count();
    if total > TOTAL_CHARS {
        return Some(format!("the arguments come to {total} characters as the card would draw them, past the {TOTAL_CHARS} a card shows"));
    }
    None
}

/// The sentence a refused request is answered with.
pub fn refused(rule: &str) -> String {
    format!("this request is too long to put in front of a person in full, so nothing was asked: {rule}")
}

/// One escaped row cut to [`ROW_CHARS`], the cut named. Defence in depth: [`refusal`] already
/// turned away anything longer, so on a request that was asked this changes nothing.
pub fn clip_row(escaped: &str) -> String {
    clip(escaped, ROW_CHARS)
}

/// [`clip_row`] for a card of an action that runs whatever it is given, whose command row has
/// room for [`COMMAND_CHARS`].
pub fn clip_row_for(open: bool, escaped: &str) -> String {
    if open { clip(escaped, KEY_CHARS + 2 + COMMAND_CHARS) } else { clip_row(escaped) }
}

/// The rows joined as the card's "Exactly:" line, cut to [`TOTAL_CHARS`], the cut named.
pub fn clip_rows(joined: &str) -> String {
    clip(joined, TOTAL_CHARS)
}

/// The argument rows as the card's "Exactly:" line draws them before its final cut: each escaped
/// and cut to [`ROW_CHARS`], joined with "; ". The one join, read by [`refusal`] to measure a
/// request and by `approval_wording::consequences` to draw it, so the two cannot drift: the
/// separators count against [`TOTAL_CHARS`] in both (fifth review of #639 — counted only on the
/// card, they let a request at the limit through and cut its last argument).
pub fn joined(rows: &[String]) -> String {
    joined_for(false, rows)
}

/// [`joined`] for a question's rows (`question`), whose quoted values are drawn as one value each;
/// every other card's rows are escaped plainly.
pub fn joined_for(question: bool, rows: &[String]) -> String {
    let draw = if question { crate::approval_wording::visible_row } else { crate::approval_wording::visible_row_raw };
    rows.iter().map(|row| clip_row(&draw(row))).collect::<Vec<_>>().join("; ")
}

fn clip(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}\u{2026} ({count} characters in full)")
}

#[cfg(test)]
#[path = "approval_bounds_tests.rs"]
mod approval_bounds_tests;

#[cfg(test)]
#[path = "approval_command_tests.rs"]
mod approval_command_tests;
