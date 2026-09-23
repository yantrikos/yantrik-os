//! The file browser, operable as data.
//!
//! The shell already *describes* the Files screen — path, entries, selection, free space — so an
//! agent can see what a directory holds without a screenshot. What it could not do was move: open
//! a folder, go up, make a directory, delete a file. Those are the other half of "operate the OS
//! as data", and the callbacks the file-browser UI drives are right there to reuse, so an agent
//! and a person navigate through exactly the same code.
//!
//! Every action here runs on the Files screen. If the shell is showing something else, the action
//! switches to it first, because operating the file browser *is* being on it, and a describe of
//! the result would report the wrong screen otherwise.

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::App;

/// The Files screen id.
const FILES_SCREEN: i32 = 8;

/// Put the shell on the Files screen if it is not already, so the browser state is live and a
/// following describe reports the directory rather than whatever else was up.
fn ensure_files_screen(ui: &App) {
    if ui.get_current_screen() != FILES_SCREEN {
        ui.set_current_screen(FILES_SCREEN);
        ui.invoke_navigate(FILES_SCREEN);
    }
}

/// The directory and how much it holds, after an action — enough for a caller to know where it
/// landed without a second round trip; the full listing is one `describe shell` away.
fn where_now(ui: &App) -> serde_json::Value {
    use slint::Model;
    let entries = ui.get_file_browser_entries();
    serde_json::json!({
        "path": ui.get_file_browser_path().to_string(),
        "entries": entries.row_count(),
        "free_space": ui.get_file_free_space_text().to_string(),
        "loading": ui.get_file_browser_loading(),
        "operation_busy": ui.get_file_operation_busy(),
        "notice": ui.get_file_notice().to_string(),
        "view": if ui.get_file_grid_view() { "grid" } else { "list" },
    })
}

/// Whether the current listing has an entry by this name, so an action can refuse a name that is
/// not there and say so, rather than invoke a callback that quietly does nothing.
fn has_entry(ui: &App, name: &str) -> bool {
    use slint::Model;
    if ui.get_file_browser_loading() {
        return false;
    }
    let entries = ui.get_file_browser_entries();
    (0..entries.row_count())
        .filter_map(|i| entries.row_data(i))
        .any(|e| e.name == name)
}

/// Add the file-browser actions to the shell's control surface.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let for_go = ui.as_weak();
    let for_enter = ui.as_weak();
    let for_open = ui.as_weak();
    let for_up = ui.as_weak();
    let for_folder = ui.as_weak();
    let for_file = ui.as_weak();
    let for_delete = ui.as_weak();

    let up = |w: &slint::Weak<App>| w.upgrade().ok_or_else(|| "the shell is gone".to_string());

    let surface = surface
        .action(
            Action::new("files_go", "Open the Files screen at an absolute path").defers()
                .arg(Param::text("path").describe("An absolute directory path, e.g. /home/user or /tmp")),
            move |args| {
                let ui = up(&for_go)?;
                let path = args["path"].as_str().unwrap_or_default().to_string();
                if path.is_empty() {
                    return Err("`path` is empty".into());
                }
                ensure_files_screen(&ui);
                ui.invoke_file_navigate_to_path(path.clone().into());
                Ok(serde_json::json!({ "requested_path": path, "now": where_now(&ui) }))
            },
        )
        .action(
            Action::new("files_enter", "Enter a subdirectory of the current one by name").defers()
                .arg(Param::text("name").describe("A directory name shown in the current listing")),
            move |args| {
                let ui = up(&for_enter)?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                ensure_files_screen(&ui);
                if !has_entry(&ui, &name) {
                    return Err(format!("nothing called `{name}` in {}", ui.get_file_browser_path()));
                }
                ui.invoke_file_navigate_dir(name.clone().into());
                Ok(serde_json::json!({ "requested_directory": name, "now": where_now(&ui) }))
            },
        )
        .action(
            // Opening a file leaves the Files screen for a viewer/editor/player, which is the
            // point; the result says where it went so a caller is not surprised the next describe
            // is not the file browser. A web page, an SVG or a PDF leaves it for the browser
            // window instead (#233), so the description names that too.
            Action::new("files_open", "Open a file in the current directory (image, text, audio; a web page, SVG or PDF in the browser)")
                .arg(Param::text("name").describe("A file name shown in the current listing")),
            move |args| {
                let ui = up(&for_open)?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                ensure_files_screen(&ui);
                if !has_entry(&ui, &name) {
                    return Err(format!("nothing called `{name}` in {}", ui.get_file_browser_path()));
                }
                ui.invoke_file_open(name.clone().into());
                Ok(serde_json::json!({ "opened": name, "screen": crate::control::screen_name(ui.get_current_screen()) }))
            },
        )
        .action(
            Action::new("files_up", "Go up to the parent directory").defers(),
            move |_| {
                let ui = up(&for_up)?;
                ensure_files_screen(&ui);
                ui.invoke_file_go_up();
                Ok(serde_json::json!({ "now": where_now(&ui) }))
            },
        )
        .action(
            Action::new("files_new_folder", "Create a folder in the current directory").defers()
                .arg(Param::text("name").describe("The new folder's name")),
            move |args| {
                let ui = up(&for_folder)?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                if name.is_empty() {
                    return Err("`name` is empty".into());
                }
                ensure_files_screen(&ui);
                ui.invoke_file_create_folder(name.clone().into());
                Ok(serde_json::json!({ "requested_folder": name, "now": where_now(&ui) }))
            },
        )
        .action(
            Action::new("files_new_file", "Create an empty file in the current directory").defers()
                .arg(Param::text("name").describe("The new file's name")),
            move |args| {
                let ui = up(&for_file)?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                if name.is_empty() {
                    return Err("`name` is empty".into());
                }
                ensure_files_screen(&ui);
                ui.invoke_file_create_file(name.clone().into());
                Ok(serde_json::json!({ "requested_file": name, "now": where_now(&ui) }))
            },
        )
        .action(
            // Preserve the existing permission classification for automation callers.
            Action::new("files_delete", "Move a file or folder to recoverable Trash").defers()
                .risk("dangerous")
                .arg(Param::text("name").describe("The name to delete, shown in the current listing")),
            move |args| {
                let ui = up(&for_delete)?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                ensure_files_screen(&ui);
                if !has_entry(&ui, &name) {
                    return Err(format!("nothing called `{name}` in {}", ui.get_file_browser_path()));
                }
                ui.invoke_file_delete(name.clone().into());
                Ok(serde_json::json!({ "requested_trash": name, "now": where_now(&ui) }))
            },
        );
    let weak = ui.as_weak();
    let surface = surface.action(
        Action::new(
            "files_select",
            "Select an entry by name; optionally extend selection",
        )
        .arg(Param::text("name"))
        .arg(Param::flag("extend").optional()),
        move |args| {
            use slint::Model;
            let ui = up(&weak)?;
            ready(&ui)?;
            let name = args["name"].as_str().ok_or("name required")?;
            let entries = ui.get_file_browser_entries();
            let index = (0..entries.row_count())
                .find(|i| entries.row_data(*i).is_some_and(|e| e.name == name))
                .ok_or("Entry not found")?;
            ui.invoke_file_multi_select_clicked(
                index as i32,
                args["extend"].as_bool().unwrap_or(false),
                false,
            );
            Ok(where_now(&ui))
        },
    );
    // The grid/list switch in the toolbar, for a caller. Grid is the folder tiles with their
    // item counts and times and the recent row under them; list is one row per entry.
    let weak = ui.as_weak();
    let surface = surface.action(
        Action::new("files_view", "Show the folder as tiles (grid) or as rows (list)")
            .arg(Param::text("view").describe("grid or list")),
        move |args| {
            let ui = up(&weak)?;
            let grid = match args["view"].as_str().unwrap_or_default() {
                "grid" => true,
                "list" => false,
                other => return Err(format!("`view` is grid or list, not `{other}`")),
            };
            ensure_files_screen(&ui);
            ui.set_file_grid_view(grid);
            Ok(where_now(&ui))
        },
    );
    let weak = ui.as_weak();
    let surface = surface.action(
        Action::new(
            "files_rename",
            "Rename an entry without replacing existing files",
        )
        .defers()
        .arg(Param::text("name"))
        .arg(Param::text("new_name")),
        move |args| {
            let ui = up(&weak)?;
            ready(&ui)?;
            let name = args["name"].as_str().ok_or("name required")?;
            let new = args["new_name"].as_str().ok_or("new_name required")?;
            crate::fileops::name(new)?;
            if !has_entry(&ui, name) {
                return Err("Entry not found".into());
            }
            ui.invoke_file_rename(name.into(), new.into());
            Ok(where_now(&ui))
        },
    );
    macro_rules! action {
        ($surface:ident,$name:literal,$description:literal,$callback:ident,$deferred:expr,$needs_ready:expr) => {
            let weak = ui.as_weak();
            let spec = Action::new($name, $description);
            let spec = if $deferred { spec.defers() } else { spec };
            let $surface = $surface.action(spec, move |_| {
                let ui = up(&weak)?;
                if $needs_ready {
                    ready(&ui)?;
                }
                ui.$callback();
                Ok(where_now(&ui))
            });
        };
    }
    action!(
        surface,
        "files_copy",
        "Copy selected entries to the Files clipboard",
        invoke_file_copy_selected,
        false,
        true
    );
    action!(
        surface,
        "files_cut",
        "Prepare selected entries to move",
        invoke_file_cut_selected,
        false,
        true
    );
    action!(
        surface,
        "files_paste",
        "Paste into this folder without replacement",
        invoke_file_paste,
        true,
        true
    );
    action!(
        surface,
        "files_trash_selected",
        "Move selected entries to recoverable Trash",
        invoke_file_delete_selected,
        true,
        true
    );
    action!(
        surface,
        "files_undo_trash",
        "Restore the most recently trashed entries",
        invoke_file_undo_trash,
        true,
        true
    );
    action!(
        surface,
        "files_toggle_trash",
        "Show Trash or return to the folder",
        invoke_file_toggle_trash,
        true,
        true
    );
    action!(
        surface,
        "files_refresh",
        "Reload the folder",
        invoke_file_refresh,
        true,
        false
    );
    action!(
        surface,
        "files_cancel",
        "Request cancellation of the active operation",
        invoke_file_cancel_operation,
        true,
        false
    );
    action!(
        surface,
        "files_terminal",
        "Open a Terminal tab in this folder",
        invoke_file_context_open_terminal,
        true,
        true
    );
    surface
}
fn ready(ui: &App) -> Result<(), String> {
    if ui.get_current_screen() != FILES_SCREEN {
        return Err("Open Files first".into());
    }
    if ui.get_file_browser_loading() {
        return Err("Folder is still loading; read describe and retry".into());
    }
    if ui.get_file_operation_busy() {
        return Err("A file operation is running".into());
    }
    Ok(())
}
