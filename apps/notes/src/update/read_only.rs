//! A library open read-only (`Vault::read_only`): every change refused with the reason, before it
//! starts, and every read as before.
//!
//! Refused here, at the door, so a refusal changes nothing at all: no edit applied and then not
//! saved, no note in the list that is not on disk, no call answered as if it had worked. The
//! vault refuses every write too (`store/write.rs`), which is what stops a save already owed: text
//! kept across a reload that found the library read-only fails its save with the same sentence.

use yantrik_app::effects::Effects;

use super::surface;
use super::window::Dialog;
use crate::command::{Command, WindowAction};
use crate::model::Model;
use crate::view::render::{CONTENT, FIELDS, STATUS};
use crate::Notes;

/// Whether `command` is a change a read-only library refuses; if so, it is refused and said.
pub fn refused(m: &mut Model, fx: &mut Effects<Notes>, command: &Command) -> bool {
    let Some(why) = m.vault.read_only().map(str::to_string) else { return false };
    match command {
        // The editor and the fields already show what was typed: they are put back.
        Command::Edit(_) | Command::Metadata { .. } => {
            if m.current().is_some() {
                fx.changed(CONTENT);
                fx.changed(FIELDS);
            }
        }
        Command::Window(action) if action.writes() || imports(m, action) => {}
        Command::Surface(act) if act.writes() => {
            surface::refuse(m, fx, why);
            return true;
        }
        _ => return false,
    }
    m.notice = why;
    fx.changed(STATUS);
    true
}

/// The dialog's Continue on the import dialog: an import, which adds a note to the library.
fn imports(m: &Model, action: &WindowAction) -> bool {
    matches!(action, WindowAction::Confirm { .. }) && m.pane.dialog == Dialog::Import as i32
}
