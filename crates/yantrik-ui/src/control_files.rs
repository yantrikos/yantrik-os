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

use crate::control_files_cursor as files_cursor;
use crate::control_files_make::make_here;
use crate::control_files_mind as mind;
use crate::App;

/// The Files screen id.
const FILES_SCREEN: i32 = 8;

/// Put the shell on the Files screen if it is not already, so the browser state is live and a
/// following describe reports the directory rather than whatever else was up.
pub(crate) fn ensure_files_screen(ui: &App) {
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

/// What `files_open` needs of the Files browser, so the order of its checks and the screen switch
/// can be tested without a window.
pub(crate) trait Browser {
    fn label(&self) -> String;
    fn has_entry(&self, name: &str) -> bool;
    fn show_files(&self);
}

struct UiBrowser<'a>(&'a App);

impl Browser for UiBrowser<'_> {
    fn label(&self) -> String {
        self.0.get_file_browser_path().to_string()
    }
    fn has_entry(&self, name: &str) -> bool {
        has_entry(self.0, name)
    }
    fn show_files(&self) {
        ensure_files_screen(self.0);
    }
}

/// The checks before `files_open` goes to Files: the refusals, then that the entry is in the
/// listing. Only when all pass is the person's screen switched; on any `Err` it is where it was.
pub(crate) fn begin_open<B: Browser>(
    browser: &B,
    name: &str,
    refusals: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let label = browser.label();
    refusals(&label)?;
    if !browser.has_entry(name) {
        return Err(format!("nothing called `{name}` in {label}"));
    }
    browser.show_files();
    Ok(())
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
                if let Some(answer) = files_cursor::intercept("files_go", args) {
                    return answer;
                }
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
                    // Checked again on what the path is now, links resolved (the person's own is not).
                    let listing = files_cursor::read_folder(&path, for_mind)?;
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
                if let Some(answer) = files_cursor::intercept("files_enter", args) {
                    return answer;
                }
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
                if let Some(answer) = files_cursor::intercept("files_open", args) {
                    return answer;
                }
                let ui = up(&for_open)?;
                // The file opens in a window of its own (editor, viewer, mpv, the browser), which
                // comes up over a waiting card (card_watch). First, so a refusal leaves even the
                // screen where it was.
                crate::card_watch::hold_windows("files_open")?;
                let name = args["name"].as_str().unwrap_or_default().to_string();
                // The answer names the app the one rule picked for this file, so a caller
                // learns where it went and not only that it went. `invoke_file_open` runs the
                // same `classify` on the callback side; asking it here too is the cheap way to
                // keep the answer and the launch on one rule (#233).
                let action = crate::mime_dispatch::classify(&name);
                let app = crate::mime_dispatch::app_name(&action);
                // Every refusal, and the check that the entry is there, come before the person's
                // screen is touched: a refused or failed action of a Mind's must not take the
                // person to Files (the 520 gate, and the review of efafef94, F4). A refusal added
                // later belongs in the closure.
                begin_open(&UiBrowser(&ui), &name, |label| {
                    mind::here(label)?;
                    // Before anything opens: the person's LibreOffice is closed to a mind (#953).
                    mind::may_open_in_default_app(&action, &crate::apps::Catalogue::shared().get())?;
                    // The entry too, links followed: opening puts its contents in a window the
                    // mind can read back, and a link in the home may lead anywhere.
                    mind::may_open_entry(label, &name)
                })?;
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
            move |args| {
                if let Some(answer) = files_cursor::intercept("files_up", args) {
                    return answer;
                }
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
            .arg(Param::text("name").describe("A name, or a/b, or ~/a/b; in the folder you are in, or in `parent`").optional())
            .arg(Param::text("path").describe("The folder to make, e.g. /home/user/a/b; missing parents are made too. Touches nobody's screen").optional())
            .arg(Param::text("parent").describe("The folder to make `name` in. Touches nobody's screen").optional()),
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
            .arg(Param::text("name").describe("The new file's name; in the folder you are in, or in `parent`").optional())
            .arg(Param::text("path").describe("The file to make, e.g. /home/user/a/b.txt. Touches nobody's screen").optional())
            .arg(Param::text("parent").describe("The folder to make `name` in. Touches nobody's screen").optional()),
            move |args| make_here(&for_file, args, false),
        )
        .action(
            // Preserve the existing permission classification for automation callers.
            Action::new("files_delete", "Move a file or folder to recoverable Trash").defers()
                .risk("dangerous")
                .arg(Param::text("name").describe("The name to delete, shown in the current listing (the person's form; a Mind gives `path`)").optional())
                .arg(Param::text("path").describe("The entry to delete, e.g. /home/user/a/b.txt (a Mind's form, required of a Mind so the approval names the whole target; touches nobody's screen)").optional())
                .arg(Param::text("parent").describe("The folder `name` is in (a Mind's form, with `name`)").optional()),
            move |args| {
                if let Some(answer) = files_cursor::intercept("files_delete", args) {
                    return answer;
                }
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
                if let Some(answer) = files_cursor::intercept("files_select", args) {
                    return answer;
                }
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
             kind, size, modified and `changed` (unix seconds; changed is ctime, which cannot be set back), `real` (where it resolves) and `via_link` \
             (whether a link took it there) when it does. `~` is the person's home",
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
    // What a folder holds, for anyone, without touching a screen (#1033): a Mind's way to look
    // in a folder now that its `files_go` moves only its own cursor.
    let weak = ui.as_weak();
    let surface = surface.action(
        Action::new(
            "files_list",
            "What a folder holds: how many entries, the first few names (folders end in /). Reads              only; no screen moves. With no `path`, the folder you are in (a Mind's cursor, set by              files_go; the Files screen's folder for the person)",
        )
        .risk("safe")
        .arg(Param::text("path").describe("An absolute directory path or ~/…").optional()),
        move |args| {
            let on_screen = || weak.upgrade().map(|ui| ui.get_file_browser_path().to_string()).unwrap_or_else(|| "~".into());
            files_cursor::list(args, on_screen)
        },
    );
    // The grid/list switch in the toolbar, for a caller. Grid is the folder tiles with their
    // item counts and times and the recent row under them; list is one row per entry.
    let weak = ui.as_weak();
    let surface = surface.action(
        Action::new("files_view", "Show the folder as tiles (grid) or as rows (list)")
            .arg(Param::text("view").describe("grid or list")),
        move |args| {
                if let Some(answer) = files_cursor::intercept("files_view", args) {
                    return answer;
                }
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
        .arg(Param::text("name").describe("The entry to rename, in the current listing (the person's form; a Mind gives `path`)").optional())
        .arg(Param::text("new_name"))
        .arg(Param::text("path").describe("The entry to rename, e.g. /home/user/a/b.txt (a Mind's form; touches nobody's screen)").optional())
        .arg(Param::text("parent").describe("The folder `name` is in (a Mind's form)").optional()),
        move |args| {
                if let Some(answer) = files_cursor::intercept("files_rename", args) {
                    return answer;
                }
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
            let $surface = $surface.action(spec, move |args| {
                if let Some(answer) = files_cursor::intercept($name, args) {
                    return answer;
                }
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

#[cfg(test)]
mod order_tests {
    use super::{begin_open, Browser};
    use std::cell::Cell;

    /// A Files browser that only records whether the person's screen was taken to Files.
    struct Fake {
        entries: &'static [&'static str],
        switched: Cell<bool>,
    }

    impl Browser for Fake {
        fn label(&self) -> String {
            "~".into()
        }
        fn has_entry(&self, name: &str) -> bool {
            self.entries.contains(&name)
        }
        fn show_files(&self) {
            self.switched.set(true);
        }
    }

    fn fake() -> Fake {
        Fake { entries: &["notes.txt"], switched: Cell::new(false) }
    }

    /// The person's screen is unchanged after every refused or failed `files_open`, whatever the
    /// refusal, and moves only when the open goes ahead.
    #[test]
    fn the_screen_is_unchanged_after_every_refused_or_failed_files_open() {
        let refused = [
            Err("outside the folders a mind may see".to_string()),
            Err("closed to a mind".to_string()),
            Err("a protected entry".to_string()),
        ];
        for refusal in refused {
            let b = fake();
            assert!(begin_open(&b, "notes.txt", |_| refusal).is_err());
            assert!(!b.switched.get(), "a refusal left the person's screen alone");
        }
        let b = fake();
        let said = begin_open(&b, "gone.txt", |_| Ok(())).unwrap_err();
        assert!(said.starts_with("nothing called `gone.txt`"), "{said}");
        assert!(!b.switched.get(), "a name that is not there left the person's screen alone");
        let b = fake();
        begin_open(&b, "notes.txt", |_| Ok(())).expect("an open that is allowed");
        assert!(b.switched.get(), "and an allowed open goes to Files");
    }
}
