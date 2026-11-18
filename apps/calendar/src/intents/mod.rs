//! What the Mind may ask Calendar by name: the same eleven actions, with the same names,
//! descriptions, parameters and grades as before, in the same order, each the command a button
//! runs where there is a button for it.
//!
//! Split by family: `reading` moves the window and reads the store, `writing` puts events on,
//! moves them and takes them off.
//!
//! # Settling
//!
//! Every read of the store and every write runs in a job now, so each intent is answered when
//! its own result lands (`Settle::Explicit`), in the command that carries it (`answers.rs`): the
//! answer is the store read back, as it always was, and never the arguments repeated. A reading
//! intent's answer is built from where it put the window and what its own read listed, so nothing
//! the person does meanwhile, and no other call, can change it; if either has moved the window
//! since, the answer says so in `window_moved_since`.

use std::time::Duration;

use yantrik_app::intents::{Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::Calendar;

mod reading;
mod writing;

/// How long an intent may take to answer: 14 s.
///
/// The ceiling is the Mind's. Its `app_action` client gives up after `ACT_TIMEOUT`, 15 s
/// (`crates/yantrik-companion-tools/src/app_ui.rs`). An answer later than that reaches nobody, and
/// the Mind sees a transport timeout for a change that may well have gone through, and may try it
/// again. Settling within 14 s means the app always answers first, with the outcome or with its
/// own "did not settle" refusal. The floor is the work: a delete by title is a listing, a read, a
/// delete and a read back, each bounded by the client's own timeout.
pub const WITHIN: Duration = Duration::from_secs(14);

pub fn table(table: &mut Intents<Calendar>) {
    reading::days_and_months(table);
    writing::table(table);
    reading::view_switch(table);
}

/// An intent answered when its work has landed.
fn explicit(intent: Intent<Calendar>) -> Intent<Calendar> {
    intent.settles(Settle::Explicit { within: WITHIN })
}

/// A sentence from the argument checks, as a refusal.
fn refused(sentence: String) -> Refused {
    Refused::new("bad_argument", sentence)
}
