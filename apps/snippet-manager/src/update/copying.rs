//! Copy, which is the whole point of a snippet manager: the code onto the clipboard in a job, then
//! the use count kept.
//!
//! The copy itself is the thing being asked for and it either happened or it did not; recording
//! that it happened is bookkeeping, and a bookkeeping failure is not reported as a failed copy —
//! the code really is on the clipboard by then. The code goes to the clipboard and nowhere else:
//! no answer, notice or log line carries it, only its length.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

use super::browsing::named;
use super::changes::commit;
use super::{fail, say, worked, written, Fx};
use crate::answers;
use crate::command::{After, Command, Copied, Work};
use crate::model::{Model, PART_COPY, PART_DETAIL, PART_LIST};

/// "Copied!" stays on the button this long, from wherever the copy came from: a window that reads
/// `Copied!` for ever because a mind copied without pressing anything is a small lie.
pub const COPY_FEEDBACK: Duration = Duration::from_millis(1400);
pub const COPY_FEEDBACK_TIMER: TimerId = TimerId("snippets.copied");

pub(super) fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::Copy(id) => {
            // Copy takes the code from the store, so the editor goes in first: a person who typed
            // and then pressed Copy means the words in front of them. A caller on the control
            // surface gets the stored snippet, because it asked for the stored snippet and is not
            // looking at the editor.
            let restore = m.restore_point();
            match commit(m) {
                Ok(true) => written::write(m, fx, After::Answer(Value::Null), restore),
                Ok(false) => {}
                Err(why) => {
                    m.copied = false;
                    fx.changed(PART_COPY);
                    fail(m, fx, &why);
                    return Ok(());
                }
            }
            start(m, fx, id, true);
        }
        Command::CopyNamed(needle) => {
            let id = named(m, &needle)?;
            start(m, fx, id, false);
        }
        Command::Copied(copied) => landed(m, *copied, fx),
        Command::CopyFeedbackOver => {
            m.copied = false;
            fx.changed(PART_COPY);
        }
        _ => {}
    }
    Ok(())
}

/// The code of `id` onto the clipboard, in a job.
fn start(m: &mut Model, fx: &mut Fx, id: i32, from_window: bool) {
    let Some(snippet) = m.library.get(id).cloned() else {
        // Nothing was copied, so the button must not say it was.
        m.copied = false;
        fx.changed(PART_COPY);
        fail(m, fx, &format!("no snippet with id {id}"));
        return;
    };
    if snippet.code.is_empty() {
        m.copied = false;
        fx.changed(PART_COPY);
        fail(m, fx, &format!("`{}` has no code to copy", snippet.title));
        return;
    }
    m.work += 1;
    let clipboard = m.clipboard.clone();
    let failed = Command::WorkFailed(Box::new(Work::Copy { from_window }));
    fx.spawn_or(failed, move |_| {
        let result = clipboard.copy(&snippet.code);
        Command::Copied(Box::new(Copied {
            id: snippet.id,
            title: snippet.title,
            bytes: snippet.code.len(),
            use_count: snippet.use_count,
            result,
            from_window,
        }))
    });
}

fn landed(m: &mut Model, copied: Copied, fx: &mut Fx) {
    fx.changed(PART_COPY);
    if let Err(why) = &copied.result {
        m.copied = false;
        fail(m, fx, why);
        return;
    }
    m.copied = true;
    fx.after(COPY_FEEDBACK_TIMER, COPY_FEEDBACK, Command::CopyFeedbackOver);
    let unsaved = answers::copied(copied.id, &copied.title, copied.bytes, copied.use_count, false);
    let restore = m.restore_point();
    match m.library.mark_used(copied.id) {
        Ok(used) => {
            let saved = answers::copied(copied.id, &copied.title, copied.bytes, used.use_count, true);
            let after = After::Counted { saved, unsaved, from_window: copied.from_window };
            written::write(m, fx, after, restore);
            fx.changed(PART_LIST);
            fx.changed(PART_DETAIL);
        }
        // Gone from under the copy: the code is on the clipboard, and there is nothing to count.
        Err(_) => {
            if copied.from_window {
                say(m, fx, "The code is on the clipboard, but the use count could not be saved.");
            } else {
                worked(m, fx);
            }
            answers::settle(m, fx, unsaved);
        }
    }
}
