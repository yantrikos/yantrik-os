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

use crate::control_files_mind as mind;
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
    if let Some(hidden) = mind::hidden_here(&ui.get_file_browser_path()) {
        // As `describe shell` hides it: a mind that went somewhere allowed is answered before
        // the new listing arrives, while the screen still shows where the person left it.
        return serde_json::json!({
            "path": serde_json::Value::Null,
            "hidden": hidden,
            "loading": ui.get_file_browser_loading(),
            "operation_busy": ui.get_file_operation_busy(),
        });
    }
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

/// `files_new_folder` / `files_new_file`: the checks on the UI thread, the disk on the socket's
/// side of `answer_later`, so a slow or hung mount cannot freeze the desktop. The answer is the
/// file-system fact (`created` / `existed`) and is settled; only the listing catches up after, by
/// a refresh queued onto the UI thread once the disk has changed.
fn make_here(
    weak: &slint::Weak<App>,
    args: &serde_json::Value,
    folder: bool,
) -> Result<serde_json::Value, String> {
    let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
    let name = args["name"].as_str().unwrap_or_default().to_string();
    if name.is_empty() {
        return Err("`name` is empty".into());
    }
    let nested = crate::control_files_create::is_nested(&name);
    if nested && !folder {
        return Err(format!(
            "`{name}` has folders in it; make the folder first with files_new_folder, then create the file there"
        ));
    }
    // A `~/…` path does not depend on the folder on screen; anything else is made inside it, which
    // a mind must be allowed to see. A nested path's levels are each judged on the worker, where
    // they exist one after another (`make_nested_checked`).
    if !name.starts_with("~/") {
        mind::here(&ui.get_file_browser_path())?;
    }
    ensure_files_screen(&ui);
    if !nested {
        mind::may_make(&ui.get_file_browser_path(), &name)?;
    }
    let dir = crate::filebrowser::expand_home(&ui.get_file_browser_path());
    let weak = ui.as_weak();
    // Who is asking is known here; the worker may not be able to tell, so it is carried in.
    let mind = mind::a_mind_is_calling();
    let work = move || {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let answer = if nested {
            crate::control_files_create::make_nested_checked(&dir, &name, mind, &home)?
        } else {
            crate::control_files_create::make_checked(&dir, &name, folder, mind, &home)?
        };
        if answer.get("created").is_some() {
            let _ = weak.upgrade_in_event_loop(|ui| ui.invoke_file_refresh());
        }
        Ok(answer)
    };
    yantrik_app_runtime::control::answer_later(work)
        .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
        .or_else(|work| work())
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

/// The names of the entries selected in the current listing.
fn selected_names(ui: &App) -> Vec<String> {
    use slint::Model;
    let entries = ui.get_file_browser_entries();
    (0..entries.row_count())
        .filter_map(|i| entries.row_data(i))
        .filter(|e| e.selected)
        .map(|e| e.name.to_string())
        .collect()
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
            Action::new("files_go", "Open Files at a folder; answers what is in it")
                .arg(Param::text("path").describe("An absolute directory path, e.g. /home/user or ~/Documents")),
            move |args| {
                let ui = up(&for_go)?;
                let path = args["path"].as_str().unwrap_or_default().to_string();
                if path.is_empty() {
                    return Err("`path` is empty".into());
                }
                mind::may_open(&path)?;
                // The folder is read on the worker and the answer is what it holds, settled. Before,
                // the answer was "requested" and unsettled, with nothing about the folder; on VM 520 a
                // mind, unsure it had arrived, went to the same folder five more times and made nothing.
                let weak = ui.as_weak();
                // Who is asking is known here; the worker may not be able to tell, so it is carried in.
                let for_mind = mind::a_mind_is_calling();
                let work = move || {
                    // Checked again on what the path is now, links resolved: the listing names what is
                    // inside, and a folder swapped for a link since the check above would otherwise be
                    // read wherever it now leads.
                    if for_mind {
                        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
                        let home = std::fs::canonicalize(&home).unwrap_or(home);
                        let full = crate::filebrowser::expand_home(&path);
                        if let Ok(resolved) = std::fs::canonicalize(&full) {
                            mind::open_verdict(&resolved.to_string_lossy(), &home)?;
                        }
                    }
                    let listing = crate::control_files_create::listing(&path)?;
                    let go = path.clone();
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ensure_files_screen(&ui);
                        ui.invoke_file_navigate_to_path(go.into());
                    });
                    Ok(listing)
                };
                yantrik_app_runtime::control::answer_later(work)
                    .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                    .or_else(|work| work())
            },
        )
        .action(
            Action::new("files_enter", "Enter a subdirectory of the current one by name").defers()
                .arg(Param::text("name").describe("A directory name shown in the current listing")),
            move |args| {
                let ui = up(&for_enter)?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                ensure_files_screen(&ui);
                // Checked first, and never naming the folder, so a refusal is no way to learn
                // what a folder the mind may not see holds.
                mind::here(&ui.get_file_browser_path())?;
                if !has_entry(&ui, &name) {
                    return Err(format!("nothing called `{name}` in {}", ui.get_file_browser_path()));
                }
                // The folder the callback will load, computed the way it computes it.
                mind::may_open(&mind::into(&ui.get_file_browser_path(), &name))?;
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
                .arg(Param::text("name").describe("A file name shown in the current listing"))
                .defers(),
            move |args| {
                let ui = up(&for_open)?;
                // The file opens in a window of its own (editor, viewer, mpv, the browser), which
                // comes up over a waiting card (card_watch). First, so a refusal leaves even the
                // screen where it was.
                crate::card_watch::hold_windows("files_open")?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                ensure_files_screen(&ui);
                mind::here(&ui.get_file_browser_path())?;
                if !has_entry(&ui, &name) {
                    return Err(format!("nothing called `{name}` in {}", ui.get_file_browser_path()));
                }
                // The answer names the app the one rule picked for this file, so a caller
                // learns where it went and not only that it went. `invoke_file_open` runs the
                // same `classify` on the callback side; asking it here too is the cheap way to
                // keep the answer and the launch on one rule (#233).
                let app = crate::mime_dispatch::app_name(&crate::mime_dispatch::classify(&name));
                // The entry too, links followed: opening puts its contents in a window the mind
                // can read back, and a link in the home may lead anywhere.
                mind::may_open_entry(&ui.get_file_browser_path(), &name)?;
                // Asked on this thread, where the caller is in scope: a mind's app opens in Mind
                // View on a worker, which can be refused, and the person's desktop is not the
                // fallback. The answer then waits for the window like `open_app`'s, instead of
                // saying `opened` for a launch that was refused (PR #582 review, S5).
                let for_mind = matches!(crate::mind_view::requester_now(), crate::mind_view::Requester::Mind(_));
                crate::running::clear_launch_failure(&app);
                ui.invoke_file_open(name.clone().into());
                let answer = serde_json::json!({ "opened": name, "app": app, "screen": crate::control::screen_name(ui.get_current_screen()) });
                if !for_mind {
                    return Ok(answer);
                }
                let wait = move || {
                    let mut probe = crate::mind_landing::probe_for(app.clone());
                    let seen = crate::mind_landing::wait_for_window(
                        &mut probe,
                        crate::mind_landing::BUDGET,
                        crate::mind_landing::STEP,
                    );
                    crate::mind_landing::answer(answer, &app, seen)
                };
                yantrik_app_runtime::control::answer_later(wait)
                    .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                    .or_else(|wait| wait())
            },
        )
        .action(
            Action::new("files_up", "Go up to the parent directory").defers(),
            move |_| {
                let ui = up(&for_up)?;
                ensure_files_screen(&ui);
                // From the home, up is /home and every account's name in it. The folder checked is
                // the one the callback loads: the label may be `~`, whose `Path::parent` is "".
                mind::may_open(&mind::up_from(&ui.get_file_browser_path()))?;
                ui.invoke_file_go_up();
                Ok(serde_json::json!({ "now": where_now(&ui) }))
            },
        )
        .action(
            Action::new(
                "files_new_folder",
                "Create a folder here, or a nested path (a/b, ~/a/b) making missing parents; answers \
                 `created` or `existed`, plus `made_parents`",
            )
            .arg(Param::text("name").describe("A name, or a/b, or ~/a/b")),
            move |args| make_here(&for_folder, args, true),
        )
        .action(
            // The shell's own editor_* actions were the way to put text in a file until #253
            // removed them, and a mind asked to "create a text file" reads this action first:
            // the live T6/T7 run found the Files action and never the Editor. So it says where
            // text goes.
            Action::new(
                "files_new_file",
                "Create an empty file in the current directory. Answers `created` or `existed` \
                 (already there, left untouched), with the absolute path and its `kind`. To write \
                 text into a file, use the `editor` app: `new` with `text`, then `save_as` the path",
            )
            .arg(Param::text("name").describe("The new file's name")),
            move |args| make_here(&for_file, args, false),
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
                mind::here(&ui.get_file_browser_path())?;
                if !has_entry(&ui, &name) {
                    return Err(format!("nothing called `{name}` in {}", ui.get_file_browser_path()));
                }
                mind::may_touch(&ui.get_file_browser_path(), std::slice::from_ref(&name))?;
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
            mind::here(&ui.get_file_browser_path())?;
            ready(&ui)?;
            let name = args["name"].as_str().ok_or("name required")?;
            let entries = ui.get_file_browser_entries();
            let index = (0..entries.row_count())
                .find(|i| entries.row_data(*i).is_some_and(|e| e.name == name))
                .ok_or("Entry not found")?;
            // A selection is what copy, cut and trash act on next.
            mind::may_touch(&ui.get_file_browser_path(), &[name.to_string()])?;
            ui.invoke_file_multi_select_clicked(
                index as i32,
                args["extend"].as_bool().unwrap_or(false),
                false,
            );
            Ok(where_now(&ui))
        },
    );
    // Whether a path is there, answered by the desktop, which runs as the person and can see
    // their home: a mind with an account of its own cannot tell "not there" from "hidden from me"
    // (yantrik_ipc_contracts::home_paths). A read: it does not move the Files screen.
    let surface = surface.action(
        Action::new(
            "files_stat",
            "Whether a path in the person's home exists: `exists` true, false, or \"unknown\" with \
             a `reason` (not_found, not_allowed, outside, protected, broken_link, not_a_path); \
             kind, size and modified (unix seconds) when it does. `~` is the person's home",
        )
        .risk("safe")
        .arg(Param::text("path").describe("An absolute path or ~/…, e.g. ~/notes/today.txt")),
        |args| {
            let asked = args["path"].as_str().unwrap_or_default().to_string();
            let home = std::env::var("HOME").unwrap_or_default();
            // Off the UI thread: a path under a hung network mount in the home would otherwise
            // freeze the person's whole desktop in the syscall.
            let work = move || Ok(yantrik_ipc_contracts::home_paths::stat(&asked, std::path::Path::new(&home)));
            yantrik_app_runtime::control::answer_later(work)
                .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                .or_else(|work| work())
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
            mind::here(&ui.get_file_browser_path())?;
            ready(&ui)?;
            let name = args["name"].as_str().ok_or("name required")?;
            let new = args["new_name"].as_str().ok_or("new_name required")?;
            crate::fileops::name(new)?;
            if !has_entry(&ui, name) {
                return Err("Entry not found".into());
            }
            // What is renamed, what it becomes, and what lands under the new name: `.ssh`
            // renamed away, a folder renamed `applications` under ~/.local/share, or `cfg`
            // holding autostart/ renamed `.config`.
            mind::may_rename(&ui.get_file_browser_path(), name, new)?;
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
                // Every one of these acts on the folder on screen, wherever the person left it.
                mind::here(&ui.get_file_browser_path())?;
                match $name {
                    "files_toggle_trash" => mind::may_show_trash()?,
                    "files_undo_trash" => mind::may_undo_trash()?,
                    // The selection may be the person's, made on anything in the folder.
                    "files_copy" | "files_cut" | "files_trash_selected" => {
                        mind::may_touch(&ui.get_file_browser_path(), &selected_names(&ui))?
                    }
                    "files_paste" => mind::may_paste(&ui.get_file_browser_path())?,
                    _ => {}
                }
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
