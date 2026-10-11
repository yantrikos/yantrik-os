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
                let shown = match &map[*key] {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let escaped = visible(&shown).chars().count();
                if escaped > approvals::WHOLE_VALUE_CHARS {
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
    let total = joined(&approvals::args_rows_with(args, value_chars)).chars().count();
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
    rows.iter().map(|row| clip_row(&visible(row))).collect::<Vec<_>>().join("; ")
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
