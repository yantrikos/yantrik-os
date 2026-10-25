//! What the person looks at: the search, the tag chips, the sidebar, the snippet in the editor,
//! and the editor's typing. Nothing here writes, except the typing kept before the editor moves.

use serde_json::{json, Value};
use yantrik_app::Refused;

use super::{changes, fail, worked, written, Fx};
use crate::answers;
use crate::command::{After, Command};
use crate::model::{Model, PART_DETAIL, PART_LIST, PART_RAISE};

pub(super) fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::Search(query) => {
            m.query = query;
            fx.changed(PART_LIST);
        }
        Command::TagFilter(tag) => {
            // Pressing the active chip again clears it: a filter with no way off is a trap.
            if m.tag.to_lowercase() == tag.to_lowercase() {
                m.tag.clear();
            } else {
                m.tag = tag;
            }
            fx.changed(PART_LIST);
        }
        Command::CollectionSelect(id) => {
            m.collection = id;
            fx.changed(PART_LIST);
        }
        Command::Edited(editor) => {
            // The window already shows it: nothing is drawn, so the caret stays where it is.
            if editor.id >= 0 && editor.id == m.editor.id {
                m.editor = editor;
                m.typed += 1;
            }
        }
        Command::Select(id) => move_to(m, fx, id, false),
        Command::ContextActivated(id) => {
            // The one context row is the open snippet; activating it selects it, which is what
            // makes the row a way back to the thing rather than a label.
            if let Ok(id) = id.parse::<i32>() {
                move_to(m, fx, id, false);
            }
        }
        Command::Open(needle) => {
            let id = named(m, &needle)?;
            move_to(m, fx, id, true);
        }
        Command::Find(query) => {
            m.query = query.clone();
            fx.changed(PART_LIST);
            let answer = answers::searched(m, &query);
            answers::settle(m, fx, answer);
        }
        Command::Show => {
            fx.changed(PART_RAISE);
            let answer = json!({ "showing": m.editor.title });
            answers::settle(m, fx, answer);
        }
        _ => {}
    }
    Ok(())
}

/// The id of the snippet a caller named, or a refusal saying why not.
pub(super) fn named(m: &Model, needle: &str) -> Result<i32, Refused> {
    m.library
        .find(needle)
        .map(|s| s.id)
        .map_err(|why| Refused::new("not_found", why))
}

/// Put `id` in the editor. The typing in it is kept first, and the editor does not move if it
/// cannot be: the typing is still on screen where it can be retried or copied out, which is the
/// one thing the old app took away. `asked`: the Mind's `open`, which brings the window forward
/// and is answered with the snippet.
pub(super) fn move_to(m: &mut Model, fx: &mut Fx, id: i32, asked: bool) {
    let restore = m.restore_point();
    let wrote = match changes::commit(m) {
        Ok(wrote) => wrote,
        Err(why) => {
            fail(m, fx, &why);
            fx.changed(PART_LIST);
            return;
        }
    };
    m.select(id);
    fx.changed(PART_LIST);
    fx.changed(PART_DETAIL);
    let answer = if asked {
        fx.changed(PART_RAISE);
        answers::stored(m, id)
    } else {
        Value::Null
    };
    if wrote {
        written::write(m, fx, After::Answer(answer), restore);
    } else {
        worked(m, fx);
        answers::settle(m, fx, answer);
    }
}
