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

/// What all the rows of a question's card may come to once escaped.
pub const TOTAL_CHARS: usize = 2400;

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
    if !is_question(app, action) {
        return crate::approval_bounds::refusal(args, params, whole);
    }
    let map = args.as_object()?;
    if map.len() > ROWS {
        return Some(format!("there are {} arguments, and a card for this question shows at most {ROWS}", map.len()));
    }
    let mut total = 0;
    for (key, value) in map {
        if key.chars().count() > crate::approval_bounds::KEY_CHARS || crate::approval_wording::visible(key) != *key {
            return Some(format!("the argument name `{}` is not one a card can show", crate::approval_wording::visible(key)));
        }
        if params.is_some_and(|published| !published.iter().any(|p| p == key)) {
            return Some(format!("`{key}` is not an argument the app publishes for this action"));
        }
        let drawn = crate::approval_wording::visible(&shown(value)).chars().count();
        if drawn > approvals::WHOLE_VALUE_CHARS {
            return Some(format!(
                "`{key}` is {drawn} characters as the card would draw it, and a value is shown whole or not asked about (at most {})",
                approvals::WHOLE_VALUE_CHARS
            ));
        }
        total += key.chars().count() + 2 + drawn + 2;
    }
    (total > TOTAL_CHARS).then(|| format!("the arguments come to {total} characters as the card would draw them, past the {TOTAL_CHARS} a card shows"))
}

/// The argument rows a card draws: every row of a question, each value cut only at the bound the
/// request was refused beyond (so not at all, in practice); otherwise as before.
pub fn rows(app: &str, action: &str, grade: &str, said: &str, args: &serde_json::Value, raised_by_desktop: bool) -> Vec<String> {
    if let (true, Some(map)) = (is_question(app, action), args.as_object()) {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        return keys.into_iter().map(|key| format!("{key}: {}", shown(&map[key]).chars().take(approvals::WHOLE_VALUE_CHARS).collect::<String>())).collect();
    }
    if approvals::shown_whole(grade, said) || raised_by_desktop {
        approvals::args_rows_with(args, approvals::WHOLE_VALUE_CHARS)
    } else {
        approvals::args_rows(args)
    }
}

#[cfg(test)]
#[path = "approval_whole_tests.rs"]
mod tests;
