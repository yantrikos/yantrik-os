//! The companion's four questions are drawn whole (#1311, round 2): decided by the question, not
//! by reading its sentence.
//!
//! A card shows its arguments whole when the grade is `dangerous` or the app's sentence says the
//! action cannot be undone or runs whatever it is given (`approvals::shown_whole`). Two of the
//! four questions (`plan_work`, `change_item`) say neither, and an ordinary card cuts a value at
//! sixty characters and counts eight rows: a mail subject of sixty-one characters could never be
//! moved and a recipe of six steps never made. What the person is asked to allow is exactly the
//! values the companion sends, so for these four the card shows every row, each value up to
//! [`approvals::WHOLE_VALUE_CHARS`], and a request that would not fit is refused before it is
//! asked, never cut. The rows are more than eight because a plan lists a row for each step.
//!
//! Which requests are these is [`is_question`]: the shell's own four actions, by name. The
//! grade, the sentence and who is asking change nothing about it. Everything else is as it was
//! (`approval_bounds`).

use crate::approvals;

/// How many rows a question's card shows. A recipe is a row for the tool, the name, the trigger
/// and each of up to twelve steps, with a few more for what else it carries.
pub const ROWS: usize = 20;

/// What all the rows of a question's card may come to once escaped and joined: the same number as
/// the pinned "Exactly:" line is cut at (`approval_bounds::TOTAL_CHARS`), so what is accepted is
/// what that line carries, whole (security review of #1313, H1: 2400 accepted, 640 drawn).
pub const TOTAL_CHARS: usize = crate::approval_bounds::TOTAL_CHARS;

/// Whether this is one of the shell's four questions for the companion.
pub fn is_question(app: &str, action: &str) -> bool {
    app == "shell" && crate::control_questions::QUESTIONS.iter().any(|q| q.name == action)
}

/// [`approvals::shown_whole`], and always for a question.
pub fn shown_whole(app: &str, action: &str, grade: &str, said: &str) -> bool {
    is_question(app, action) || approvals::shown_whole(grade, said)
}

fn shown(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Why a request cannot be put in front of a person in full, or `None`. For a question: a name
/// the card can show, a field the question publishes, at most [`ROWS`] rows, each value at most
/// [`approvals::WHOLE_VALUE_CHARS`] as the card draws it, and at most [`TOTAL_CHARS`] together.
/// For anything else, `approval_bounds::refusal`.
pub fn refusal(app: &str, action: &str, args: &serde_json::Value, params: Option<&[String]>, whole: bool) -> Option<String> {
    refusal_for(app, action, args, params, whole, false)
}

/// [`refusal`], where `open` says the action runs whatever it is given (`approval_bounds::refusal_for`).
pub fn refusal_for(app: &str, action: &str, args: &serde_json::Value, params: Option<&[String]>, whole: bool, open: bool) -> Option<String> {
    if !is_question(app, action) {
        return crate::approval_bounds::refusal_for(args, params, whole, open);
    }
    let map = args.as_object()?;
    if map.len() > ROWS {
        return Some(format!("there are {} arguments, and a card for this question shows at most {ROWS}", map.len()));
    }
    for (key, value) in map {
        if key.chars().count() > crate::approval_bounds::KEY_CHARS || crate::approval_wording::visible(key) != *key {
            return Some(format!("the argument name `{}` is not one a card can show", crate::approval_wording::visible(key)));
        }
        if params.is_some_and(|published| !published.iter().any(|p| p == key)) {
            return Some(format!("`{key}` is not an argument the app publishes for this action"));
        }
        let drawn = crate::approval_wording::visible_value(&shown(value)).chars().count();
        if drawn > approvals::WHOLE_VALUE_CHARS {
            return Some(format!(
                "`{key}` is {drawn} characters as the card would draw it, and a value is shown whole or not asked about (at most {})",
                approvals::WHOLE_VALUE_CHARS
            ));
        }
    }
    // Measured as the card draws it: the same rows, escapes and join as the pinned "Exactly:" line.
    let total = crate::approval_bounds::joined_for(true, &rows(app, action, "", "", args, false)).chars().count();
    (total > TOTAL_CHARS).then(|| format!("it is too long to show whole: the arguments come to {total} characters as the card would draw them, and a card shows {TOTAL_CHARS}; make it in the app"))
}

/// The request bound for the card about to be raised. The card is shown whole when either sentence
/// says it must be (the caller's words can only tighten); the 4,000-character room for a command
/// is the app's own published sentence's alone, never the caller's purpose (#1357, round 4).
pub fn request_refusal(app: &str, action: &str, args: &serde_json::Value, params: Option<&[String]>, grade: &str, published: &str, purpose: &str) -> Option<String> {
    let whole = shown_whole(app, action, grade, &format!("{published} {purpose}"));
    refusal_for(app, action, args, params, whole, approvals::open_ended(published))
}

/// The argument rows a card draws: every row of a question, each value cut only at the bound the
/// request was refused beyond (so not at all, in practice); otherwise as before.
pub fn rows(app: &str, action: &str, grade: &str, said: &str, args: &serde_json::Value, raised_by_desktop: bool, open: bool) -> Vec<String> {
    if let (true, Some(map)) = (is_question(app, action), args.as_object()) {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        return keys.into_iter().map(|key| format!("{key}: {}", shown(&map[key]).chars().take(approvals::WHOLE_VALUE_CHARS).collect::<String>())).collect();
    }
    if approvals::shown_whole(grade, said) || raised_by_desktop {
        let room = if open { crate::approval_bounds::COMMAND_CHARS } else { approvals::WHOLE_VALUE_CHARS };
        approvals::args_rows_with(args, room)
    } else {
        approvals::args_rows(args)
    }
}

#[cfg(test)]
#[path = "approval_whole_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "approval_whole_draw_tests.rs"]
mod draw_tests;
