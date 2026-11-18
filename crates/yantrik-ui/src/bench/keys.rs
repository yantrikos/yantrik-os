//! What a key means on the desktop: Enter opens, F2 renames, Delete archives, the arrows move the
//! selection, Ctrl+A selects all, Escape lets go. A key the Bench does not use is not taken, so
//! it goes on to whoever else wants it.

use super::model::Dir;
use super::{ops, Bench};
use crate::App;

/// Handle one key; `true` when it was used. `name` is what bench.slint calls the key.
pub fn handle(ui: &App, b: &mut Bench, name: &str, ctrl: bool, shift: bool) -> bool {
    // While a name is being typed, the text field has the keys; Escape and Enter reach it first.
    if b.model.renaming.is_some() {
        return false;
    }
    let anything = !b.model.selection.is_empty();
    match (name, ctrl) {
        ("enter", _) if anything => ops::open_selection(ui, b),
        ("f2", _) if b.model.selection.len() == 1 => ops::start_rename(ui, b),
        ("delete", _) if anything => ops::archive_selection(ui, b),
        ("a", true) => b.model.select_all(),
        ("escape", _) if anything || b.root.is_some() => {
            b.model.clear_selection();
            b.root = None;
            super::wiring::close_menu(ui);
        }
        ("up", _) => b.model.arrow(Dir::Up, shift),
        ("down", _) => b.model.arrow(Dir::Down, shift),
        ("left", _) => b.model.arrow(Dir::Left, shift),
        ("right", _) => b.model.arrow(Dir::Right, shift),
        _ => return false,
    }
    super::view::publish(ui, b);
    true
}
