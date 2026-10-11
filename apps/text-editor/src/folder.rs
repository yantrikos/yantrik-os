//! The folder open beside the tabs (#731), in the window and on the control surface.
//!
//! `project` decides which rows are on screen; this puts them in the `YTree`, opens a file the
//! person clicks through the editor's own `open` (so a file already in a tab comes forward
//! rather than opening twice), follows the tab in front, and answers `open_folder`,
//! `close_folder` and `describe.folder`.

use crate::project::{Node, Project, Saved};
use crate::{
    act, agent_rule, arg, checkpoint, expanded, needed, no_dialog, open, refuse, State,
    TextEditorApp, TreeRow, Workbench,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::path::{Path, PathBuf};
use yantrik_app_runtime::control::{agent_is_calling, Action};

/// How many rows `describe.folder.rows` holds; `truncated` says when there were more.
const DESCRIBE_ROWS: usize = 200;

/// Put the open folder's rows on screen, the tab in front's file selected.
pub fn show(ui: &TextEditorApp, b: &mut Workbench) {
    let Some(project) = &b.folder else {
        b.rows.clear();
        ui.set_folder_open(false);
        ui.set_tree_rows(ModelRc::default());
        return;
    };
    let rows = project.rows();
    let active = b.docs[b.active].path.as_deref();
    let mut parents: Vec<(usize, usize)> = Vec::new();
    let mut current = -1;
    let shown: Vec<TreeRow> = rows
        .iter()
        .enumerate()
        .map(|(i, n)| {
            while parents.last().is_some_and(|&(depth, _)| depth >= n.depth) {
                parents.pop();
            }
            let parent = parents.last().map_or(-1, |&(_, at)| at as i32);
            if n.is_dir {
                parents.push((n.depth, i));
            }
            let selected = !n.dim && Some(n.path.as_path()) == active;
            if selected {
                current = i as i32;
            }
            TreeRow {
                name: n.name.clone().into(),
                depth: n.depth as i32,
                is_dir: n.is_dir,
                expanded: n.is_dir && project.is_expanded(&n.path),
                selected,
                dim: n.dim,
                parent,
            }
        })
        .collect();
    ui.set_project_name(project.name().into());
    ui.set_tree_filter(project.filter().into());
    ui.set_tree_rows(ModelRc::new(VecModel::from(shown)));
    if current >= 0 || ui.get_tree_current() >= rows.len() as i32 {
        ui.set_tree_current(current);
    }
    ui.set_folder_open(true);
    b.rows = rows;
}

/// The tab in front changed: reveal and select its file, if it is inside the folder.
pub fn follow_tab(ui: &TextEditorApp, b: &mut Workbench) {
    let Some(path) = b.docs[b.active].path.clone() else { return show(ui, b) };
    if let Some(project) = &mut b.folder {
        project.expand_to(&path);
    }
    show(ui, b);
}

/// The folder kept in the recovery file, for `checkpoint`.
pub fn saved(b: &Workbench) -> Option<Saved> {
    b.folder.as_ref().and_then(Project::saved)
}

/// Why `path` is not a folder this caller may have listed, if it is not. An agent goes through
/// the same rule as `open`: in the home, outside its protected places. That rule answers
/// "is not a file" for a folder that passes it, and only for one that passes it.
fn may_list(path: &Path) -> Result<(), String> {
    agent_rule::may_read(path).or_else(|e| if e.ends_with(" is not a file") { Ok(()) } else { Err(e) })?;
    if !path.is_dir() {
        return Err(format!("{} is not a folder; `open` opens a file.", path.display()));
    }
    std::fs::read_dir(path)
        .map(|_| ())
        .map_err(|e| format!("{} can't be read: {e}", path.display()))
}

/// Open `path` as the project beside the tabs, or say why not.
pub fn open_folder(ui: &TextEditorApp, s: &State, path: PathBuf) -> Result<(), String> {
    may_list(&path)?;
    {
        let mut b = s.borrow_mut();
        let mut project = Project::new(path);
        if let Some(file) = b.docs[b.active].path.clone() {
            project.expand_to(&file);
        }
        b.folder = Some(project);
        show(ui, &mut b);
    }
    ui.set_show_tree(true);
    ui.set_notice("".into());
    checkpoint(ui, s);
    Ok(())
}

pub fn close_folder(ui: &TextEditorApp, s: &State) {
    let mut b = s.borrow_mut();
    b.folder = None;
    show(ui, &mut b);
    drop(b);
    checkpoint(ui, s);
}

/// The folder as `describe` reads it: its root, its first rows as paths under it, folders with
/// a "/" after them, and whether there were more. Null with no folder open.
pub fn describe(b: &Workbench) -> serde_json::Value {
    let Some(project) = &b.folder else { return serde_json::Value::Null };
    // A folder the person opened where an agent may not read is not listed back to one.
    if agent_is_calling() && may_list(&project.root).is_err() {
        return serde_json::json!({ "root": crate::HIDDEN_TAB, "rows": [], "truncated": false });
    }
    let entries: Vec<&Node> = b.rows.iter().filter(|n| !n.dim).collect();
    serde_json::json!({
        "root": project.root.display().to_string(),
        "filter": project.filter(),
        "rows": entries.iter().take(DESCRIBE_ROWS).map(|n| project.relative(&n.path, n.is_dir)).collect::<Vec<_>>(),
        "truncated": entries.len() > DESCRIBE_ROWS || b.rows.iter().any(|n| n.dim && n.name.ends_with(" more")),
    })
}

/// Wire the panel's callbacks and show the folder restored from the recovery file.
pub fn wire(ui: &TextEditorApp, s: &State) {
    let weak = ui.as_weak();
    let state = s.clone();
    ui.on_tree_activated(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(node) = state.borrow().rows.get(i as usize).cloned() else { return };
        if node.dim || node.is_dir {
            return;
        }
        let tab = state.borrow().docs.iter().position(|d| d.path.as_ref() == Some(&node.path));
        match tab {
            Some(index) => ui.invoke_select_tab(index as i32),
            None => open(&ui, &state, node.path),
        }
    });
    let weak = ui.as_weak();
    let state = s.clone();
    ui.on_tree_toggled(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let mut b = state.borrow_mut();
        let Some(node) = b.rows.get(i as usize).cloned() else { return };
        let Some(project) = b.folder.as_mut().filter(|_| node.is_dir) else { return };
        project.toggle(&node.path);
        show(&ui, &mut b);
        drop(b);
        checkpoint(&ui, &state);
    });
    let weak = ui.as_weak();
    let state = s.clone();
    ui.on_tree_filtered(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        let mut b = state.borrow_mut();
        let Some(project) = b.folder.as_mut() else { return };
        project.set_filter(&text);
        ui.set_tree_current(-1);
        show(&ui, &mut b);
    });
    show(ui, &mut s.borrow_mut());
}

/// A handler on the control surface, as `surface` takes it.
type Run = fn(&TextEditorApp, &State, &serde_json::Value) -> Result<serde_json::Value, String>;

/// `open_folder` and `close_folder`.
pub fn actions() -> Vec<(Action, Run)> {
    vec![
        (
            act(
                "open_folder",
                "Open a folder beside the tabs as a project: its files are listed as a tree the \
                 person can click to open, and `describe.folder` reads its first rows back. \
                 Nothing in it is changed. Replaces any folder already open.",
            )
            .arg(arg(
                "path",
                "Absolute path to the folder, or one starting `~/`. Something that is not a \
                 readable folder is refused and the open folder stays as it was.",
            )),
            |ui, s, args| {
                no_dialog(ui, "open_folder")?;
                let path = needed(ui, args, "open_folder", "path", "An absolute path to a folder.")?;
                open_folder(ui, s, expanded(path.trim())).map_err(|e| refuse(ui, e))?;
                Ok(describe(&s.borrow()))
            },
        ),
        (
            act(
                "close_folder",
                "Close the folder open beside the tabs and hide its tree. The tabs and the files \
                 in the folder are left as they are.",
            ),
            |ui, s, _| {
                no_dialog(ui, "close_folder")?;
                if s.borrow().folder.is_none() {
                    return Err(refuse(ui, "No folder is open; `open_folder` opens one."));
                }
                close_folder(ui, s);
                Ok(serde_json::json!({ "folder": null }))
            },
        ),
    ]
}
