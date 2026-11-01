//! The desktop's icons on the shell's control surface: `describe shell` › `desktop`, and the two
//! actions a mind has for them, `desktop_place` and `desktop_archive`.
//!
//! Both are `sensitive`: they change the person's desktop and outlive the turn (the layout is
//! saved; an archived file is gone from where they left it). A mind asks first. Neither can
//! delete: archiving is a move into `~/Archive/<date>/`, with an Undo on screen for the person.
//! Both name an icon by the name `describe` lists under `desktop.items`, never by a path, and
//! refuse a desktop folder the file tools would not let a mind into (`control_files_mind`).

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::App;

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let place_weak = ui.as_weak();
    let archive_weak = ui.as_weak();
    surface
        .action(
            Action::new(
                "desktop_place",
                "Move one icon on the desktop to a cell of its grid (column and row count from 0, from the top left). \
                 `item` is a name from `describe shell` under `desktop.items`; the cell must be free and on the grid \
                 (`desktop.grid`). Answers with where the icon is now.",
            )
            .risk("sensitive")
            .arg(Param::text("item").describe("An icon's name, as `desktop.items` lists it"))
            .arg(Param::integer("column").describe("The column, 0 is the left edge"))
            .arg(Param::integer("row").describe("The row, 0 is the top")),
            move |args| {
                let ui = place_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let item = args["item"].as_str().unwrap_or_default().trim().to_string();
                if item.is_empty() {
                    return Err("`item` is empty".into());
                }
                let column = args["column"].as_i64().ok_or("`column` must be a whole number")?;
                let row = args["row"].as_i64().ok_or("`row` must be a whole number")?;
                crate::bench::describe::place(&ui, &item, column, row)
            },
        )
        .action(
            Action::new(
                "desktop_archive",
                "Put one thing from the desktop away: it is moved to ~/Archive/<today's date>/ and never deleted, and the person is \
                 shown an Undo. `item` is a name from `describe shell` under `desktop.items`. Answers with where it went.",
            )
            .risk("sensitive")
            .arg(Param::text("item").describe("An icon's name, as `desktop.items` lists it")),
            move |args| {
                let ui = archive_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let item = args["item"].as_str().unwrap_or_default().trim().to_string();
                if item.is_empty() {
                    return Err("`item` is empty".into());
                }
                crate::bench::describe::archive(&ui, &item)
            },
        )
}

#[cfg(test)]
mod tests {
    const SOURCE: &str = include_str!("control_bench.rs");

    /// The declaration of one action, from its name to the handler's `move |args|`.
    fn declaration(name: &str) -> &'static str {
        let from = SOURCE.find(&format!("\"{name}\",")).unwrap_or_else(|| panic!("{name} is declared"));
        let rest = &SOURCE[from..];
        &rest[..rest.find("move |args|").expect("its handler")]
    }

    /// A mind moving or putting away what the person left on their desktop is theirs to approve:
    /// the layout is saved and a file leaves where they put it.
    #[test]
    fn placing_and_archiving_are_sensitive() {
        for name in ["desktop_place", "desktop_archive"] {
            let d = declaration(name);
            assert!(d.contains(".risk(\"sensitive\")"), "{name} must be graded sensitive: {d}");
            assert!(!d.contains(".risk(\"safe\")") && !d.contains(".risk(\"standard\")"), "{name} was regraded: {d}");
        }
    }

    /// The path is never a parameter: an icon is named, and the desktop folder is the shell's.
    #[test]
    fn no_action_takes_a_path() {
        for name in ["desktop_place", "desktop_archive"] {
            let d = declaration(name);
            for param in ["\"path\"", "\"file\"", "\"dir\"", "\"folder\"", "\"destination\"", "Param::path"] {
                assert!(!d.contains(param), "{name} takes {param}");
            }
        }
    }

    /// Neither handler deletes or runs anything itself; both go through the Bench's own functions.
    #[test]
    fn the_handlers_only_call_the_bench_and_delete_nothing() {
        let body = SOURCE.split("#[cfg(test)]").next().unwrap();
        for banned in ["remove_file", "remove_dir", "Command::new", "std::fs::"] {
            assert!(!body.contains(banned), "{banned} in control_bench.rs");
        }
        assert!(body.contains("describe::place(") && body.contains("describe::archive("));
    }
}
