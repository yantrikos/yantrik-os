//! What a mind may ask Notes by name: the twenty-one actions worded, argued and graded as they
//! always were, the three choices about an outside change (step 8d), the two about unsaved text
//! kept from before Notes closed (8d-3) and a note in a new window (8d-3b), each the command its
//! window action runs.
//!
//! A description is written at the definition site of every action, and [`act`] and [`arg`]
//! refuse to build one that is a placeholder: every description used to be `"Notes: {name}"`
//! and every argument optional, which is why a model asked for a note titled Groceries made a
//! note with no title. `yos describe` prints each description and each argument's type, required
//! flag and description, so what is written here is verbatim what a model reads.
//!
//! Every call is answered once its work has landed (`Settle::Explicit`): "it worked" is what the
//! disk says afterwards, never the handler's opinion of itself.
//!
//! The actions are written by family, one file each. The order a mind reads them in is the one
//! Notes has always published (tests/parity.rs pins it), so each family is a run of that order and
//! [`table`] adds them one after another.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::command::{Act, Command};
use crate::model::Model;
use crate::Notes;

mod files;
mod library;
mod note;
mod outside;
mod recovery;
mod windows;
mod writing;

/// How long a call may wait for its saves and its file work. A save is one small file write; the
/// old window waited at most 1.6 s for two of them inside a 3 s round trip.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);

/// One action, with the one sentence a reader who cannot see the screen needs. The guard stops
/// the app (and the tests, which build this same table) on a placeholder.
fn act(name: &'static str, sentence: &'static str) -> Action {
    assert!(
        sentence.len() >= 20 && !sentence.starts_with("Notes:"),
        "notes action `{name}` was given a placeholder description: {sentence:?}"
    );
    Action::new(name, sentence)
}

/// One argument, with the format it takes and what leaving it out means. Required unless
/// `.optional()` is added.
fn arg(name: &'static str, sentence: &'static str) -> Param {
    assert!(
        sentence.len() >= 15,
        "notes argument `{name}` was given no usable description: {sentence:?}"
    );
    Param::text(name).describe(sentence)
}

/// A required text that has to say something. The dispatch refuses a missing one before this
/// runs; this is the other half, a present and blank one, refused by name with the window's
/// notice saying it too. The text itself is kept as given, not trimmed.
fn needed(a: &Args<'_>, action: &str, name: &str, hint: &str) -> Result<String, Command> {
    match a.raw().get(name).and_then(Value::as_str) {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        Some(_) => Err(refuse(format!("`{action}` was given an empty `{name}`. {hint}"))),
        None => Err(refuse(format!("`{action}` needs `{name}`. {hint}"))),
    }
}

fn refuse(sentence: String) -> Command {
    Command::Surface(Act::Refuse(sentence))
}

/// An optional text: absent and empty mean the same thing to every caller here.
fn given(a: &Args<'_>, name: &str) -> String {
    a.raw().get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A call, parsed: its command, or the refusal of a blank argument.
fn call(a: &Args<'_>, parse: impl FnOnce(&Args<'_>) -> Result<Act, Command>) -> Result<Command, Refused> {
    Ok(parse(a).map(Command::Surface).unwrap_or_else(|refusal| refusal))
}

fn add(
    table: &mut Intents<Notes>,
    action: Action,
    parse: impl Fn(&Args<'_>) -> Result<Act, Command> + 'static,
) {
    table.add(
        Intent::new(action, move |a| call(a, &parse))
            .settles(Settle::Explicit { within: ANSWER_WITHIN })
            .reply(|m: &Model| m.answer.clone()),
    );
}

pub fn table(table: &mut Intents<Notes>) {
    // A call while a file operation runs is refused, as it always was: the answer it would give
    // is about a note that is about to change.
    table.state_rule(|m: &Model| match m.busy {
        Some(_) => Err(Refused::new(
            "busy",
            "Notes is in the middle of a file operation; read `describe` again in a moment.",
        )),
        None => Ok(()),
    });

    writing::table(table);
    library::table(table);
    note::table(table);
    files::table(table);
    outside::table(table);
    recovery::table(table);
    windows::table(table);
}
