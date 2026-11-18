//! What a mind may ask the Text Editor by name: the twenty actions worded, argued and graded as
//! they always were, each the command its window action runs.
//!
//! This surface used to be two `for name in [...]` loops that answered every action `accepted`.
//! Every action is written out with the sentence a reader who cannot see the screen needs, every
//! argument says what goes in it, and every answer is read back out of the document and off the
//! disk after the work has settled (`Settle::Explicit`). [`act`] and [`arg`] refuse to build an
//! action whose description is a placeholder.
//!
//! The actions are written by family, one file each, in the order the editor has always published
//! them (tests/parity.rs pins it).

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::caller::Caller;
use crate::command::{Act, Command};
use crate::model::Model;
use crate::TextEditor;

mod closing;
mod documents;
mod find;
mod text;

/// How long a call may wait for its file work and its read-back. The old window waited at most
/// 800 ms inside a 3 s round trip; a save of the 1 MiB a tab may hold is far below either.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);

/// One action, with the one sentence a reader who cannot see the screen needs. The guard stops
/// the app, and the tests, which build this same table, on a placeholder.
fn act(name: &'static str, sentence: &'static str) -> Action {
    assert!(
        sentence.len() >= 20 && !sentence.starts_with("Editor:"),
        "editor action `{name}` was given a placeholder description: {sentence:?}"
    );
    Action::new(name, sentence)
}

/// One text argument, with the format it takes and what leaving it out means. Required unless
/// `.optional()` is added.
fn arg(name: &'static str, sentence: &'static str) -> Param {
    assert!(sentence.len() >= 15, "editor argument `{name}` was given no usable description: {sentence:?}");
    Param::text(name).describe(sentence)
}

/// A required text that has to say something. The dispatch refuses a missing one before this
/// runs; this is the other half, a blank one, refused by name with the window's notice saying it
/// too.
fn needed(a: &Args<'_>, action: &str, name: &str, hint: &str) -> Result<String, Act> {
    match a.raw().get(name).and_then(Value::as_str) {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        Some(_) => Err(Act::Refuse(format!("`{action}` was given an empty `{name}`. {hint}"))),
        None => Err(Act::Refuse(format!("`{action}` needs `{name}`. {hint}"))),
    }
}

/// A text the action may take empty, refused only when it is not given at all.
fn given(a: &Args<'_>, action: &str, name: &str, what: &str) -> Result<String, Act> {
    a.raw()
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Act::Refuse(format!("`{action}` needs `{name}`: {what}")))
}

/// An optional whole number, given as a JSON number or as digits in a string.
fn number(a: &Args<'_>, name: &str) -> Option<i64> {
    a.raw().get(name).and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()))
}

/// Declares an intent: its arguments read into an [`Act`], raised with who is calling, answered
/// once its work has landed.
fn add(table: &mut Intents<TextEditor>, action: Action, parse: impl Fn(&Args<'_>) -> Result<Act, Act> + 'static) {
    table.add(
        Intent::new(action, move |a| {
            let act = parse(a).unwrap_or_else(|refusal| refusal);
            Ok::<Command, Refused>(Command::Surface { act, caller: Caller::now() })
        })
        .settles(Settle::Explicit { within: ANSWER_WITHIN })
        .reply(|m: &Model| m.answer.clone()),
    );
}

pub fn table(table: &mut Intents<TextEditor>) {
    documents::table(table);
    closing::table(table);
    find::table(table);
    text::table(table);
}
