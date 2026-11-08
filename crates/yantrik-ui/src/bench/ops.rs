//! What the Bench does to the world: open, create, paste, archive, reveal, pin.
//!
//! Each of these is one thing, called from the menu, from a key or from the control surface, so
//! all three do exactly the same. None deletes: the only way anything leaves the desktop is
//! [`archive_names`], which moves it into `~/Archive/<date>/` and offers Undo.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_bench::archive::{self, Archived};
use yantrik_bench::items::{display_name, Kind};
use yantrik_bench::recent::Target;

use super::{shortcuts, view, Bench, UNDO_TOAST};
use crate::{App, BenchState, ToastActionData, ToastData};

/// Opening more than this at once is a mistake (a selection of the whole desktop and Enter).
const OPEN_AT_ONCE: usize = 8;

/// A plain line on the screen, not stored as a notification. What reaches it can carry a file
/// name (an error from a copy, a rename), so it is cleaned the way a label is.
fn say(ui: &App, summary: &str, body: &str) {
    let clean = |s: &str| if s.is_empty() { String::new() } else { display_name(s) };
    crate::wire::toast::local(&ui.as_weak(), "Desktop", &clean(summary), &clean(body), 1);
}

// ── Opening ──

pub fn open_selection(ui: &App, b: &mut Bench) {
    let names: Vec<String> = b.model.selected_items().iter().take(OPEN_AT_ONCE).map(|i| i.name.clone()).collect();
    for name in names {
        open_one(ui, b, &name);
    }
}

/// Open one icon the way a double-click does: a folder in Files, an app shortcut through the
/// dock's own launch, anything else in the app the one MIME rule picks.
pub fn open_one(ui: &App, b: &mut Bench, name: &str) {
    let Some(item) = b.model.item(name).cloned() else { return };
    let path = b.model.path_of(name);
    match item.kind {
        Kind::Folder => {
            reveal_path(ui, &path);
            b.remember(Target::File { path: path.to_string_lossy().into_owned() }, &item.label(), "Desktop");
        }
        Kind::App => {
            let installed = b.catalogue.get();
            let Some(sc) = b.shortcuts.get(&b.model.desktop, &item) else {
                say(ui, &format!("{} is not an app shortcut", item.label()), "Its file is not a desktop entry for an application.");
                return;
            };
            match shortcuts::resolve(&sc, &installed) {
                Some(target) => {
                    ui.invoke_launch_app(target.launch_id.clone().into());
                    b.remember(Target::App { id: target.launch_id }, &display_name(&sc.name), "App");
                }
                None => say(ui, &format!("{} is not on this machine", display_name(&sc.name)), "The app this shortcut opens is not installed."),
            }
        }
        _ => {
            let action = crate::mime_dispatch::classify(&item.name);
            crate::wire::open_with::launch(&action, &item.name, &path, Some(ui));
            b.remember(Target::File { path: path.to_string_lossy().into_owned() }, &item.label(), "Desktop");
        }
    }
    view::publish_continue(&ui.global::<BenchState>(), b);
}

/// Continue: open the last thing opened, if it can still be.
pub fn open_continue(ui: &App, b: &mut Bench) {
    let Some(rec) = b.last_opened.clone().filter(|r| r.is_open_able()) else { return };
    match rec.target {
        Target::App { id } => ui.invoke_launch_app(id.into()),
        Target::File { path } => {
            let path = PathBuf::from(path);
            if path.is_dir() {
                reveal_path(ui, &path);
            } else {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                crate::wire::open_with::launch(&crate::mime_dispatch::classify(&name), &name, &path, Some(ui));
            }
        }
    }
}

/// Open one file in the app the person chose from Open with.
pub fn open_with(ui: &App, b: &mut Bench, desktop_id: &str) {
    let Some(name) = b.model.selection.iter().next().cloned() else { return };
    let path = b.model.path_of(&name);
    crate::wire::open_with::launch_desktop_id(desktop_id, &name, &path, Some(ui));
    b.remember(Target::File { path: path.to_string_lossy().into_owned() }, &display_name(&name), "Desktop");
}

/// Open with's rows for the one selected file: the same list Files offers, default first.
pub fn open_with_apps(b: &Bench) -> Vec<(String, String, bool)> {
    let Some(name) = b.model.selection.iter().next() else { return Vec::new() };
    let defaults = crate::mime_dispatch::MimeDefaults::read();
    let installed = b.catalogue.get();
    let browser = crate::wire::dock::find_browser().map(|(bin, _)| format!("{bin}.desktop"));
    crate::mime_dispatch::open_with(name, &defaults, &installed, browser.as_deref())
        .into_iter()
        .map(|r| (r.id, r.name, r.is_default))
        .collect()
}

/// Reveal in Files: the Files screen, at the desktop folder (or the folder asked for).
pub fn reveal(ui: &App, b: &Bench) {
    reveal_path(ui, &b.model.desktop);
}

fn reveal_path(ui: &App, path: &std::path::Path) {
    crate::control_files::ensure_files_screen(ui);
    ui.invoke_file_navigate_to_path(crate::filebrowser::collapse_home(path).into());
}

/// Change wallpaper: Settings, at Appearance, where the wallpaper is chosen.
pub fn change_wallpaper(ui: &App) {
    ui.set_settings_category(0);
    ui.set_current_screen(7);
    ui.invoke_navigate(7);
}

/// Pin to dock / Unpin from dock, for the one selected shortcut.
pub fn toggle_pin(ui: &App, b: &mut Bench) {
    let Some(item) = b.model.selected_items().first().map(|i| (*i).clone()) else { return };
    let installed = b.catalogue.get();
    let Some(sc) = b.shortcuts.get(&b.model.desktop, &item) else { return };
    let Some(target) = shortcuts::resolve(&sc, &installed) else { return };
    let pinned = crate::wire::pins::toggle(&target.launch_id);
    crate::wire::pins::publish(ui, &installed);
    say(ui, &format!("{} {} the dock", display_name(&sc.name), if pinned { "is on" } else { "is off" }), "");
}

/// Whether the one selected shortcut can be pinned, and whether it is.
pub fn pin_state(b: &mut Bench) -> (bool, bool) {
    let Some(item) = b.model.selected_items().first().map(|i| (*i).clone()) else { return (false, false) };
    let installed = b.catalogue.get();
    let Some(sc) = b.shortcuts.get(&b.model.desktop, &item) else { return (false, false) };
    match shortcuts::resolve(&sc, &installed) {
        Some(t) if crate::wire::pins::is_pinnable(&t.launch_id) => (true, crate::wire::pins::is_pinned(&t.launch_id)),
        _ => (false, false),
    }
}

// ── Making and renaming ──

/// New folder / New file: made now, selected, and ready to be named.
pub fn new_thing(ui: &App, b: &mut Bench, folder: bool) {
    let name = if folder { b.model.free_name("New folder", "") } else { b.model.free_name("New file", ".txt") };
    let made = if folder {
        crate::fileops::create_folder(&b.model.desktop, &name)
    } else {
        crate::fileops::create_file(&b.model.desktop, &name)
    };
    match made {
        Ok(()) => {
            b.model.rescan();
            b.model.selection.clear();
            b.model.selection.insert(name.clone());
            begin_rename(ui, b, &name);
        }
        Err(e) => say(ui, "Nothing was made", &e),
    }
}

/// F2 / Rename: the one selected icon's name becomes a field.
pub fn start_rename(ui: &App, b: &mut Bench) {
    if b.model.selection.len() != 1 {
        return;
    }
    if let Some(name) = b.model.selection.iter().next().cloned() {
        begin_rename(ui, b, &name);
    }
}

fn begin_rename(ui: &App, b: &mut Bench, name: &str) {
    b.model.renaming = Some(name.to_string());
    ui.global::<BenchState>().set_rename_text(name.into());
}

pub fn commit_rename(ui: &App, b: &mut Bench, old: &str, new: &str) {
    if let Err(e) = b.model.rename(old, new) {
        b.model.renaming = None;
        say(ui, "Not renamed", &e.to_string());
    }
}

// ── Pasting ──

/// Whether the Files clipboard holds something to paste.
pub fn can_paste() -> bool {
    matches!(crate::wire::files::clipboard_now(), Ok(Some((paths, _))) if !paths.is_empty())
}

/// Paste what the Files clipboard holds into the desktop, on a worker, through the transfer Files
/// uses: nothing is replaced, a copy of a folder is made whole or not at all.
pub fn paste(ui: &App, b: &Bench) {
    let Ok(Some((paths, cut))) = crate::wire::files::clipboard_now() else { return };
    let dir = b.model.desktop.clone();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let cancel = AtomicBool::new(false);
        let (mut done, mut failed) = (0usize, Vec::new());
        for src in paths {
            match crate::fileops::transfer(&src, &dir, cut, &cancel, &mut |_| {}) {
                Ok(()) => done += 1,
                Err(e) => failed.push(e),
            }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            super::refresh(&ui);
            let verb = if cut { "Moved" } else { "Copied" };
            match (done, failed.first()) {
                (0, Some(why)) => say(&ui, "Nothing was pasted", why),
                (n, Some(why)) => say(&ui, &format!("{verb} {n}, {} not", failed.len()), why),
                (n, None) => say(&ui, &format!("{verb} {n} to the desktop"), ""),
            }
        });
    });
}

// ── Archiving ──

/// Where archived things go: `~/Archive`.
fn archive_root(b: &Bench) -> PathBuf {
    b.home.join("Archive")
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Archive these names (as the desktop lists them): moved into `~/Archive/<today>/`, never
/// deleted. Returns what moved, or the first refusal if nothing did. The toast with Undo is the
/// caller's to raise, because a mind's `desktop_archive` answers with the result instead.
pub fn archive_names(b: &mut Bench, names: &[String]) -> Result<Vec<Archived>, String> {
    let (root, date) = (archive_root(b), today());
    let mut done = Vec::new();
    let mut first_failure = None;
    for name in names {
        match archive::archive(&b.model.desktop, name, &root, &date) {
            Ok(a) => {
                b.model.selection.remove(name);
                done.push(a);
            }
            Err(e) => {
                tracing::warn!(error = %e, "Archive on the desktop failed");
                first_failure.get_or_insert(e.to_string());
            }
        }
    }
    b.model.rescan();
    match (done.is_empty(), first_failure) {
        (true, Some(why)) => Err(why),
        _ => Ok(done),
    }
}

/// Delete / Archive in the menu: the selection, with a toast that says where it went and offers Undo.
pub fn archive_selection(ui: &App, b: &mut Bench) {
    let names: Vec<String> = b.model.selection.iter().cloned().collect();
    if names.is_empty() {
        return;
    }
    match archive_names(b, &names) {
        Ok(done) => offer_undo(ui, b, done),
        Err(why) => say(ui, "Not archived", &why),
    }
}

pub(super) fn offer_undo(ui: &App, b: &mut Bench, done: Vec<Archived>) {
    if done.is_empty() {
        return;
    }
    let (summary, body) = match done.as_slice() {
        [one] => (
            format!("Archived {}", display_name(&one.from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
            format!("Moved to {}", crate::filebrowser::collapse_home(one.to.parent().unwrap_or(&one.to))),
        ),
        many => (format!("Archived {} items", many.len()), format!("Moved to {}", crate::filebrowser::collapse_home(&archive_root(b).join(today())))),
    };
    let id = format!("{UNDO_TOAST}:{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    b.archived.push((id.clone(), done));
    // Only the last few batches can be undone from a toast.
    if b.archived.len() > 8 {
        b.archived.remove(0);
    }
    crate::wire::toast::push(
        ui,
        ToastData {
            id: id.into(),
            app_name: "Desktop".into(),
            summary: summary.into(),
            body: body.into(),
            urgency: 1,
            icon_char: "D".into(),
            actions: ModelRc::new(VecModel::from(vec![ToastActionData { id: "undo".into(), label: "Undo".into() }])),
            thumbnail: Default::default(),
            has_thumbnail: false,
        },
        1,
    );
}

/// Undo on the toast: put the batch back, never over something that has taken its name since.
pub fn undo_archive(ui: &App, toast_id: &str) {
    // The toast's own id says which batch.
    let restored = super::with(|b| b.archived.iter().position(|(id, _)| id == toast_id).map(|at| b.archived.remove(at))).flatten();
    let Some((_, batch)) = restored else {
        say(ui, "Nothing to put back", "That archive can no longer be undone from here; it is in ~/Archive.");
        return;
    };
    let (mut back, mut why) = (0usize, None);
    for a in &batch {
        match archive::undo(a) {
            Ok(_) => back += 1,
            Err(e) => {
                why.get_or_insert(e.to_string());
            }
        }
    }
    super::with(|b| {
        b.model.rescan();
        view::publish(ui, b);
    });
    match why {
        None => say(ui, &format!("Put back {back} on the desktop"), ""),
        Some(why) => say(ui, &format!("Put back {back} of {}", batch.len()), &why),
    }
}
