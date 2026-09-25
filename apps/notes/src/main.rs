//! Native Notes workbench. Filesystem work is serialized on one sleeping worker.
mod store;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::mpsc, time::Duration};
use store::Note;
use yantrik_app_runtime::control::{Action, App, Param, View};
use yantrik_app_runtime::prelude::*;
slint::include_modules!();
enum Job {
    Load,
    Save(Note),
    Trash(Note),
    Restore(Note),
    Import(PathBuf, usize),
    Export(PathBuf, String),
    Stop,
}
enum Event {
    Loaded(Result<(Vec<Note>, String), String>),
    Saved(Result<Note, String>),
    Trashed(String, Result<(), String>),
    Restored(Result<Note, String>),
    Imported(Result<Note, String>),
    Exported(Result<(), String>),
}
struct Workbench {
    notes: Vec<Note>,
    current: Option<Note>,
    /// Where the library lives on disk.
    ///
    /// The worker owns the only other copy and answers in `Note`s, which carry a filename and no
    /// directory — so an action could say it had made "note-0199….md" and not where. A caller
    /// told a filename it cannot find is a caller that has to guess, and `yos act notes new_note`
    /// is meant to be checkable with `cat`.
    dir: PathBuf,
    jobs: mpsc::Sender<Job>,
    events: mpsc::Receiver<Event>,
    timer: slint::Timer,
    worker: Option<std::thread::JoinHandle<()>>,
    pending: Option<String>,
    quitting: bool,
    ready: bool,
    failed: bool,
    undo: Vec<String>,
    redo: Vec<String>,
}
type State = Rc<RefCell<Workbench>>;
fn main() {
    init_tracing("yantrik-notes");
    if std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref() == Ok("1")
        && matches!(
            std::env::var("SLINT_BACKEND").as_deref(),
            Ok("winit") | Err(_)
        )
    {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(
            PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"))
                .join("yantrik-notes.lock"),
        )
        .expect("Notes instance lock");
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let _ = std::process::Command::new("wlrctl")
            .args(["toplevel", "focus", "title:Notes"])
            .status();
        return;
    }
    let ui = NotesApp::new().unwrap();
    // The bar is the app's own (#256): moving, minimising, maximising and closing from it.
    yantrik_app_runtime::window_chrome!(ui);
    let prefs = theme::load();
    ui.global::<ThemeMode>().set_dark(prefs.dark);
    ui.global::<AccentPreset>().set_index(prefs.accent_index);
    let dir = std::env::var_os("YANTRIK_NOTES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/share/yantrik/notes")
        });
    let s = wire(&ui, dir, true);
    run_until_closed(&ui, "yantrik-notes");
    let mut b = s.borrow_mut();
    b.timer.stop();
    let _ = b.jobs.send(Job::Stop);
    if let Some(w) = b.worker.take() {
        let _ = w.join();
    }
}
fn wire(ui: &NotesApp, dir: PathBuf, publish: bool) -> State {
    let (jobs, work) = mpsc::channel();
    let (results, events) = mpsc::channel();
    let weak = ui.as_weak();
    let library = dir.clone();
    let worker = std::thread::spawn(move || {
        while let Ok(job) = work.recv() {
            let e = match job {
                Job::Load => Event::Loaded(store::load(&dir)),
                Job::Save(n) => Event::Saved(store::save(&dir, &n)),
                Job::Trash(n) => Event::Trashed(n.id.clone(), store::trash(&dir, &n)),
                Job::Restore(n) => Event::Restored(store::restore(&dir, &n)),
                Job::Import(p, remaining) => Event::Imported((|| {
                    let text = store::read(&p, store::LIMIT)?.ok_or("File not found")?;
                    store::validate(&text)?;
                    if text.len() > remaining {
                        return Err("Library reached its 32 MiB text limit.".into());
                    }
                    let mut n = Note::blank("Imported note");
                    n.text = text;
                    store::save(&dir, &n)
                })()),
                Job::Export(p, t) => Event::Exported(store::atomic(&p, &t, false)),
                Job::Stop => break,
            };
            if results.send(e).is_err() {
                break;
            }
            let _ = weak.upgrade_in_event_loop(|u| u.invoke_refresh());
        }
    });
    let s = Rc::new(RefCell::new(Workbench {
        notes: vec![],
        current: None,
        dir: library,
        jobs,
        events,
        timer: slint::Timer::default(),
        worker: Some(worker),
        pending: None,
        quitting: false,
        ready: false,
        failed: false,
        undo: vec![],
        redo: vec![],
    }));
    let w = ui.as_weak();
    let b = s.clone();
    ui.on_refresh(move || {
        if let Some(u) = w.upgrade() {
            receive(&u, &b)
        }
    });
    let w = ui.as_weak();
    let b = s.clone();
    ui.on_action(move |a| {
        if let Some(u) = w.upgrade() {
            action(&u, &b, a.as_str())
        }
    });
    let w = ui.as_weak();
    let b = s.clone();
    ui.on_choose(move |id| {
        if let Some(u) = w.upgrade() {
            action(&u, &b, &format!("open:{id}"))
        }
    });
    let w = ui.as_weak();
    let b = s.clone();
    ui.on_filter(move || {
        if let Some(u) = w.upgrade() {
            list(&u, &b)
        }
    });
    let w = ui.as_weak();
    let b = s.clone();
    ui.on_edited(move |text| {
        if let Some(u) = w.upgrade() {
            edit(&u, &b, text.to_string())
        }
    });
    let w = ui.as_weak();
    let b = s.clone();
    ui.on_metadata(move || {
        if let Some(u) = w.upgrade() {
            {
                let mut st = b.borrow_mut();
                if let Some(n) = st.current.as_mut() {
                    if n.trash {
                        return;
                    }
                    n.set_field(
                        "notebook",
                        &u.get_notebook().chars().take(80).collect::<String>(),
                    );
                    n.set_field("tags", &u.get_tags().chars().take(512).collect::<String>());
                }
            }
            changed(&u, &b);
        }
    });
    let w = ui.as_weak();
    let b = s.clone();
    ui.window().on_close_requested(move || {
        let Some(u) = w.upgrade() else {
            return slint::CloseRequestResponse::HideWindow;
        };
        if u.get_busy() {
            u.set_notice("Please wait for the current operation before closing.".into());
            return slint::CloseRequestResponse::KeepWindowShown;
        }
        if b.borrow().current.as_ref().is_some_and(Note::dirty) {
            b.borrow_mut().quitting = true;
            save(&u, &b);
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });
    send(ui, &s, Job::Load);
    if publish {
        control(ui, &s)
    }
    s
}
fn send(ui: &NotesApp, s: &State, job: Job) {
    ui.set_busy(true);
    ui.set_status("Working…".into());
    if s.borrow().jobs.send(job).is_err() {
        ui.set_busy(false);
        ui.set_notice(
            "Storage worker stopped. Keep this window open to preserve your draft.".into(),
        );
    }
}
fn changed(ui: &NotesApp, s: &State) {
    let b = s.borrow();
    let Some(n) = &b.current else { return };
    ui.set_modified(n.dirty());
    ui.set_note_title(n.title().into());
    ui.set_words(n.text.split_whitespace().count() as i32);
    ui.set_status(
        if n.dirty() {
            "Unsaved · autosave pending"
        } else {
            "All changes saved"
        }
        .into(),
    );
    let w = ui.as_weak();
    let state = Rc::downgrade(s);
    b.timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(750),
        move || {
            if let (Some(u), Some(s)) = (w.upgrade(), state.upgrade()) {
                save(&u, &s)
            }
        },
    );
}
fn edit(ui: &NotesApp, s: &State, text: String) {
    if s.borrow()
        .notes
        .iter()
        .filter(|n| {
            s.borrow()
                .current
                .as_ref()
                .is_none_or(|c| c.id != n.id || c.trash != n.trash)
        })
        .map(|n| n.text.len())
        .sum::<usize>()
        + text.len()
        > store::VAULT_LIMIT
    {
        ui.set_notice("Library reached its 32 MiB text limit.".into());
        if let Some(n) = &s.borrow().current {
            ui.set_content(n.text.clone().into());
        }
        return;
    }
    if let Err(e) = store::validate(&text) {
        ui.set_notice(e.into());
        if let Some(n) = &s.borrow().current {
            ui.set_content(n.text.clone().into());
        }
        return;
    }
    {
        let mut b = s.borrow_mut();
        let Some(n) = b.current.as_mut() else { return };
        if n.trash {
            return;
        }
        if n.text == text {
            return;
        }
        let previous = std::mem::replace(&mut n.text, text);
        remember(&mut b.undo, previous);
        b.redo.clear();
    }
    changed(ui, s);
}
fn save(ui: &NotesApp, s: &State) {
    if ui.get_busy() {
        return;
    }
    let n = {
        let b = s.borrow();
        b.timer.stop();
        if !b.ready {
            return;
        }
        b.current.clone()
    };
    if let Some(n) = n {
        if n.dirty() && !n.trash {
            send(ui, s, Job::Save(n));
        }
    }
}
fn remember(history: &mut Vec<String>, text: String) {
    history.push(text);
    while history.len() > 64 || history.iter().map(String::len).sum::<usize>() > 2 * 1024 * 1024 {
        history.remove(0);
    }
}
fn undo(ui: &NotesApp, s: &State, redo: bool) {
    let mut b = s.borrow_mut();
    if b.current.as_ref().is_none_or(|n| n.trash) {
        return;
    }
    let text = if redo { b.redo.pop() } else { b.undo.pop() };
    let Some(text) = text else { return };
    let n = b.current.as_mut().unwrap();
    let previous = std::mem::replace(&mut n.text, text.clone());
    if redo {
        remember(&mut b.undo, previous)
    } else {
        remember(&mut b.redo, previous)
    }
    drop(b);
    ui.set_content(text.clone().into());
    ui.invoke_caret(text.len() as i32);
    changed(ui, s);
}
fn show(ui: &NotesApp, s: &State) {
    {
        let mut b = s.borrow_mut();
        b.undo.clear();
        b.redo.clear();
    }
    let b = s.borrow();
    let n = b.current.as_ref();
    ui.set_opened(n.is_some());
    ui.set_content(n.map(|n| n.text.clone()).unwrap_or_default().into());
    ui.set_note_title(n.map(Note::title).unwrap_or_default().into());
    ui.set_notebook(n.map(|n| n.field("notebook")).unwrap_or_default().into());
    ui.set_tags(n.map(|n| n.field("tags")).unwrap_or_default().into());
    ui.set_trashed(n.is_some_and(|n| n.trash));
    ui.set_pinned(n.is_some_and(|n| n.field("pinned") == "true"));
    ui.set_modified(n.is_some_and(Note::dirty));
    ui.set_words(n.map(|n| n.text.split_whitespace().count()).unwrap_or(0) as i32);
    ui.set_status(
        if n.is_some_and(|n| n.trash) {
            "Recoverable deletion"
        } else if n.is_some_and(Note::dirty) {
            "Unsaved · autosave pending"
        } else {
            "All changes saved"
        }
        .into(),
    );
    drop(b);
    list(ui, s);
    if ui.get_preview() {
        preview(ui, s)
    }
    ui.invoke_reset_position();
    ui.invoke_focus_content();
}
fn list(ui: &NotesApp, s: &State) {
    let b = s.borrow();
    let q = ui.get_query().trim().to_lowercase();
    let folder = ui.get_folder();
    let current = b.current.as_ref();
    let all: Vec<_> = b
        .notes
        .iter()
        .map(|n| {
            current
                .filter(|c| c.id == n.id && c.trash == n.trash)
                .unwrap_or(n)
        })
        .collect();
    ui.set_all_count(all.iter().filter(|n| !n.trash).count() as i32);
    ui.set_pin_count(
        all.iter()
            .filter(|n| !n.trash && n.field("pinned") == "true")
            .count() as i32,
    );
    ui.set_trash_count(all.iter().filter(|n| n.trash).count() as i32);
    let mut books = std::collections::BTreeMap::<String, i32>::new();
    for n in &all {
        let name = n.field("notebook");
        if !n.trash && !name.is_empty() {
            *books.entry(name).or_default() += 1;
        }
    }
    ui.set_books(ModelRc::new(VecModel::from(
        books
            .into_iter()
            .map(|(name, count)| BookRow {
                selected: folder == format!("book:{name}"),
                name: name.into(),
                count,
            })
            .collect::<Vec<_>>(),
    )));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut visible: Vec<_> = all
        .into_iter()
        .filter(|n| {
            let scope = match folder.as_str() {
                "trash" => n.trash,
                "favorites" => !n.trash && n.field("pinned") == "true",
                "recent" => !n.trash && now.saturating_sub(n.modified) < 7 * 86400,
                f if f.starts_with("book:") => !n.trash && n.field("notebook") == f[5..],
                _ => !n.trash,
            };
            scope
                && (q.is_empty()
                    || n.text.to_lowercase().contains(&q)
                    || n.meta.to_lowercase().contains(&q))
        })
        .collect();
    visible.sort_by(|a, b| {
        b.field("pinned")
            .cmp(&a.field("pinned"))
            .then(b.modified.cmp(&a.modified))
            .then(a.id.cmp(&b.id))
    });
    ui.set_notes(ModelRc::new(VecModel::from(
        visible
            .into_iter()
            .map(|n| NoteRow {
                id: format!("{}{}", if n.trash { "trash:" } else { "" }, n.id).into(),
                title: n.title().into(),
                preview: n
                    .text
                    .lines()
                    .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                    .unwrap_or("A fresh page")
                    .chars()
                    .take(90)
                    .collect::<String>()
                    .into(),
                date: chrono::DateTime::from_timestamp(n.modified as i64, 0)
                    .map(|d| {
                        d.with_timezone(&chrono::Local)
                            .format("%b %-d · %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default()
                    .into(),
                pinned: n.field("pinned") == "true",
                selected: current.is_some_and(|c| c.id == n.id && c.trash == n.trash),
            })
            .collect::<Vec<_>>(),
    )));
}
fn preview(ui: &NotesApp, s: &State) {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let b = s.borrow();
    let Some(n) = &b.current else { return };
    let mut blocks = vec![];
    let mut text = String::new();
    let mut kind = 0;
    let flush = |blocks: &mut Vec<PreviewBlock>, text: &mut String, kind| {
        if !text.trim().is_empty() {
            blocks.push(PreviewBlock {
                text: std::mem::take(text).into(),
                kind,
            })
        }
    };
    for event in Parser::new_ext(
        &n.text,
        Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    ) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut blocks, &mut text, kind);
                kind = 1;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut blocks, &mut text, kind);
                kind = 2;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut blocks, &mut text, kind);
                kind = 3;
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut text, kind);
                text.push_str("• ");
            }
            Event::End(
                TagEnd::Heading(_)
                | TagEnd::Paragraph
                | TagEnd::CodeBlock
                | TagEnd::Item
                | TagEnd::BlockQuote(_),
            ) => {
                flush(&mut blocks, &mut text, kind);
                kind = 0;
            }
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::TaskListMarker(v) => {
                if text == "• " {
                    text.clear()
                }
                text.push_str(if v { "☑ " } else { "☐ " })
            }
            Event::Rule => {
                flush(&mut blocks, &mut text, kind);
                text.push_str("────────────");
                flush(&mut blocks, &mut text, 0);
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut text, kind);
    ui.set_blocks(ModelRc::new(VecModel::from(blocks)));
}
fn receive(ui: &NotesApp, s: &State) {
    loop {
        let e = { s.borrow().events.try_recv() };
        let Ok(e) = e else { break };
        ui.set_busy(false);
        let result: Result<(), String> = match e {
            Event::Loaded(r) => r.map(|(notes, notice)| {
                let mut b = s.borrow_mut();
                let id = b.current.as_ref().map(|n| (n.id.clone(), n.trash));
                b.notes = notes;
                b.current = id.and_then(|(id, t)| {
                    b.notes.iter().find(|n| n.id == id && n.trash == t).cloned()
                });
                b.ready = true;
                ui.set_notice(notice.into());
                drop(b);
                show(ui, s);
            }),
            Event::Saved(r) => r.map(|saved| {
                let mut b = s.borrow_mut();
                if let Some(n) = b.current.as_mut() {
                    if n.id == saved.id {
                        n.baseline = saved.baseline.clone();
                        n.baseline_meta = saved.baseline_meta.clone();
                        n.modified = saved.modified;
                    }
                }
                if let Some(n) = b.notes.iter_mut().find(|n| n.id == saved.id && !n.trash) {
                    *n = saved;
                } else {
                    b.notes.push(saved);
                }
                b.failed = false;
                drop(b);
                ui.set_notice("".into());
                changed(ui, s);
                list(ui, s);
            }),
            Event::Trashed(id, r) => r.map(|_| {
                let mut b = s.borrow_mut();
                if let Some(n) = b.notes.iter_mut().find(|n| n.id == id && !n.trash) {
                    n.trash = true;
                }
                b.current = None;
                drop(b);
                show(ui, s);
                ui.set_notice("Moved to Trash. You can restore it from the library.".into());
            }),
            Event::Restored(r) | Event::Imported(r) => r.map(|n| {
                let mut b = s.borrow_mut();
                b.notes.retain(|a| a.id != n.id);
                b.notes.push(n.clone());
                b.current = Some(n);
                drop(b);
                ui.set_folder("all".into());
                ui.set_query("".into());
                ui.set_notice("".into());
                ui.set_dialog(0);
                show(ui, s);
            }),
            Event::Exported(r) => r.map(|_| {
                ui.set_dialog(0);
                ui.set_notice("Exported to the requested file.".into());
                ui.set_status("All changes saved".into());
            }),
        };
        if let Err(e) = result {
            let mut b = s.borrow_mut();
            b.pending = None;
            b.quitting = false;
            b.failed = true;
            ui.set_notice(e.into());
            ui.set_status("Needs attention · draft kept".into());
            continue;
        }
        let dirty = s.borrow().current.as_ref().is_some_and(Note::dirty);
        if !dirty {
            let pending = s.borrow_mut().pending.take();
            if let Some(a) = pending {
                action(ui, s, &a)
            }
            if s.borrow().quitting {
                let _ = ui.hide();
            }
        } else if s.borrow().quitting || s.borrow().pending.is_some() {
            save(ui, s)
        }
    }
}
fn expanded(s: &str) -> PathBuf {
    if let Some(p) = s.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(p)
    } else {
        PathBuf::from(s)
    }
}
fn action(ui: &NotesApp, s: &State, id: &str) {
    if id == "focus" {
        ui.set_focus_mode(!ui.get_focus_mode());
        ui.invoke_focus_content();
        return;
    }
    if id == "preview" {
        ui.set_preview(!ui.get_preview());
        if ui.get_preview() {
            preview(ui, s);
            ui.invoke_focus_shortcuts();
        } else {
            ui.invoke_focus_editor()
        }
        return;
    }
    if matches!(id, "undo" | "redo") {
        undo(ui, s, id == "redo");
        return;
    }
    if ui.get_busy() {
        ui.set_notice("Please wait for the current operation.".into());
        return;
    }
    if !s.borrow().ready {
        ui.set_notice(
            "The library is unavailable. Resolve the storage error, then Refresh.".into(),
        );
        if id == "reload" {
            send(ui, s, Job::Load)
        }
        return;
    }
    let navigation = id == "new"
        || id.starts_with("new:")
        || id.starts_with("open:")
        || matches!(id, "reload" | "trash" | "import");
    if navigation && s.borrow().current.as_ref().is_some_and(Note::dirty) {
        s.borrow_mut().pending = Some(id.into());
        save(ui, s);
        return;
    }
    ui.set_notice("".into());
    match id {
        // The button path ignores the new note's filename; the control surface does not — see
        // `new`, which now answers with it so an action can say which note it made.
        "new" => drop(new(ui, s, "Untitled", "")),
        a if a.starts_with("new:") => drop(new(ui, s, &a[4..], "")),
        a if a.starts_with("open:") => {
            let key = &a[5..];
            let (trash, id) = key
                .strip_prefix("trash:")
                .map(|p| (true, p))
                .unwrap_or((false, key));
            let n = s
                .borrow()
                .notes
                .iter()
                .find(|n| n.id == id && n.trash == trash)
                .cloned();
            if n.is_some() {
                s.borrow_mut().current = n;
                s.borrow_mut().failed = false;
                show(ui, s);
            }
        }
        "save" => save(ui, s),
        "reload" => send(ui, s, Job::Load),
        "pin" => {
            if let Some(n) = s.borrow_mut().current.as_mut() {
                if n.trash {
                    return;
                }
                let v = n.field("pinned") != "true";
                n.set_field("pinned", if v { "true" } else { "false" });
                ui.set_pinned(v);
            }
            changed(ui, s);
            save(ui, s);
        }
        "trash" => {
            let n = s.borrow().current.clone();
            if let Some(n) = n {
                if !n.trash {
                    send(ui, s, Job::Trash(n))
                }
            }
        }
        "restore" => {
            let n = s.borrow().current.clone();
            if let Some(n) = n {
                if n.trash {
                    send(ui, s, Job::Restore(n))
                }
            }
        }
        "copy" => {
            let n = s.borrow().current.clone();
            if let Some(mut n) = n {
                let b = s.borrow();
                if b.notes.len() >= 2000
                    || b.notes.iter().map(|n| n.text.len()).sum::<usize>() + n.text.len()
                        > store::VAULT_LIMIT
                {
                    ui.set_notice(
                        "Library reached its note or text limit (2,000 notes / 32 MiB).".into(),
                    );
                    return;
                }
                drop(b);
                n.id = format!("copy-{}.md", uuid7::uuid7());
                n.baseline = None;
                n.baseline_meta = None;
                n.trash = false;
                let mut b = s.borrow_mut();
                b.current = Some(n.clone());
                b.failed = false;
                b.notes.push(n);
                drop(b);
                show(ui, s);
                save(ui, s);
            }
        }
        "import" => {
            ui.set_path("".into());
            ui.set_dialog(1);
        }
        "export" => {
            if ui.get_opened() {
                ui.set_path("".into());
                ui.set_dialog(2);
            }
        }
        "confirm" => {
            let p = expanded(ui.get_path().trim());
            if !p.is_absolute() {
                ui.set_notice("Enter an absolute file path.".into());
                return;
            }
            if ui.get_dialog() == 1 {
                {
                    let b = s.borrow();
                    let remaining = store::VAULT_LIMIT
                        .saturating_sub(b.notes.iter().map(|n| n.text.len()).sum::<usize>());
                    if b.notes.len() >= 2000 {
                        ui.set_notice("The library supports up to 2,000 notes.".into());
                        return;
                    }
                    drop(b);
                    send(ui, s, Job::Import(p, remaining))
                }
            } else if ui.get_dialog() == 2 {
                send(ui, s, Job::Export(p, ui.get_content().to_string()))
            }
        }
        _ => {}
    }
}
/// Start a new note titled `title`, with `body` under the heading, and open it.
///
/// `body` exists because "create a note titled Groceries listing milk, eggs and bread" was three
/// calls on this surface — `new_note`, `append`, `save` — and a 27B model measured on the desktop
/// made the first two and produced a note with no title at all, because nothing it was shown said
/// `new_note` takes the title or that the title is the note's `# heading` line. One call that
/// takes both is the fix a small model can actually follow.
///
/// Returns the new note's filename. The window's own New button has no use for it; every caller
/// that has to report what it did does.
fn new(ui: &NotesApp, s: &State, title: &str, body: &str) -> Result<String, String> {
    if s.borrow().notes.len() >= 2000
        || s.borrow().notes.iter().map(|n| n.text.len()).sum::<usize>() + 1024 + body.len()
            > store::VAULT_LIMIT
    {
        return Err(refuse(
            ui,
            "The library supports up to 2,000 notes and 32 MiB of text. Nothing was created.",
        ));
    }
    let mut n = Note::blank(&title.chars().take(160).collect::<String>());
    if !body.is_empty() {
        n.text.push_str(body);
        if !n.text.ends_with('\n') {
            n.text.push('\n');
        }
    }
    store::validate(&n.text).map_err(|e| refuse(ui, e))?;
    let id = n.id.clone();
    let mut b = s.borrow_mut();
    b.notes.push(n.clone());
    b.current = Some(n);
    b.failed = false;
    drop(b);
    ui.set_folder("all".into());
    ui.set_query("".into());
    ui.set_preview(false);
    show(ui, s);
    ui.invoke_caret(ui.get_content().len() as i32);
    save(ui, s);
    Ok(id)
}

// ── What the mind is shown, and what it is told afterwards ──────────────────
//
// Everything below used to be a `for name in [...]` loop over eighteen action names, which built
// `Action::new(name, &format!("Notes: {name}"))` with one `Param::text(param).optional()` and
// answered every call with `{"accepted": true, "completed": false}`. Three separate faults came
// out of that one loop, and all three were measured on the deployed VM:
//
//   * the description a mind reads was the action's own name, so nothing said that `new_note`
//     takes a title, that the title is the first `# heading` line, or what `append` appends to;
//   * every argument was optional, so `append` with no text was accepted and appended nothing;
//   * the answer reported acceptance, never an observation, so a caller was told a note had been
//     created and could not learn its filename, its title, its size, or whether it was on disk.
//
// A description is now written at the definition site of every action, and `act`/`arg` below
// refuse to build one that is a placeholder.

/// A refusal the person at the window sees too.
///
/// Contract point 4: failure is said twice — once to the caller, once in the app's `notice`,
/// which is both `describe.notice` and the amber line on screen. A mind that cannot save a note
/// and a person who cannot see why it did not save are the same bug counted once.
fn refuse(ui: &NotesApp, message: impl Into<String>) -> String {
    let message = message.into();
    ui.set_notice(message.clone().into());
    message
}

/// One action, with the one sentence a reader who cannot see the screen needs.
///
/// The guard is the point: a description that is only the action's name, or too short to say what
/// the action does and to which note, stops the app before `serve()` — and inside the tests,
/// which build this same list. `Action::new` takes any `&str` and cannot be made to care, and
/// `yantrik-app-runtime` is shared by fourteen apps, so the check lives here.
fn act(name: &'static str, sentence: &'static str) -> Action {
    assert!(
        sentence.len() >= 20 && !sentence.starts_with("Notes:"),
        "notes action `{name}` was given a placeholder description: {sentence:?}"
    );
    Action::new(name, sentence)
}

/// One argument, with the format it takes and what leaving it out means.
///
/// Required unless `.optional()` is added — the opposite of what the loop did, which made every
/// argument optional and so let a call that named nothing be accepted.
fn arg(name: &'static str, sentence: &'static str) -> Param {
    assert!(
        sentence.len() >= 15,
        "notes argument `{name}` was given no usable description: {sentence:?}"
    );
    Param::text(name).describe(sentence)
}

/// A required text argument that has to say something, or a refusal that names it.
///
/// The runtime already refuses a *missing* required argument before the handler runs. This is the
/// other half: an argument that is present and blank. `append text=""` must not report success.
fn needed(
    ui: &NotesApp,
    args: &serde_json::Value,
    action: &str,
    name: &str,
    hint: &str,
) -> Result<String, String> {
    match args.get(name).and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        Some(_) => Err(refuse(ui, format!("`{action}` was given an empty `{name}`. {hint}"))),
        None => Err(refuse(ui, format!("`{action}` needs `{name}`. {hint}"))),
    }
}

/// An optional text argument: absent and empty mean the same thing to every caller here.
fn given(args: &serde_json::Value, name: &str) -> String {
    args.get(name).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

/// Wait, on the UI thread, for the file work this action started.
///
/// Notes does its IO on one worker: `send` sets `busy`, the worker answers on a channel, and
/// `receive` — normally woken by the worker's `invoke_refresh` — applies the result. An action
/// handler also runs on the UI thread, so a handler that returned as soon as it had sent the job
/// could only ever answer "accepted", which is precisely the fault being fixed: the caller is
/// told a note was created and its name, size and saved-ness are all still unknown.
///
/// Draining the channel here, before answering, is what lets the answer report what is on disk.
/// If it has not settled in time the answer says `saved: false` rather than guessing, and the
/// notice already carries the reason.
///
/// The budget is 800 ms because two of these can happen in one call — `flush` saves what was
/// open before `new_note` or `open_note` navigates away from it, and then the action waits for
/// its own write — and both have to fit inside the runtime's 3 s `UI_ROUNDTRIP`, or a caller is
/// told the app did not answer about work that in fact happened. A save is one small file write;
/// 800 ms is already three orders of magnitude of headroom.
fn settle(ui: &NotesApp, s: &State) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    while ui.get_busy() && std::time::Instant::now() < deadline {
        receive(ui, s);
        if !ui.get_busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    receive(ui, s);
    !ui.get_busy()
}

/// Save whatever is open before an action that would navigate away from it.
///
/// The window's own path for this queues the action in `pending` and replays it after the save;
/// an action handler cannot use that, because it has to answer for the result. So it saves and
/// waits here, and refuses if the save did not happen — an `open_note` that silently dropped
/// someone's unsaved paragraph would be a worse outcome than a refusal naming the conflict.
fn flush(ui: &NotesApp, s: &State) -> Result<(), String> {
    if !s.borrow().ready {
        return Err(refuse(
            ui,
            "The library is unavailable; resolve the storage error and run `reload`.",
        ));
    }
    if !s.borrow().current.as_ref().is_some_and(Note::dirty) {
        return Ok(());
    }
    save(ui, s);
    settle(ui, s);
    if s.borrow().current.as_ref().is_some_and(Note::dirty) {
        let why = ui.get_notice().to_string();
        return Err(refuse(
            ui,
            if why.is_empty() {
                "The open note has unsaved changes that could not be written; nothing else was \
                 done."
                    .to_string()
            } else {
                format!("The open note could not be saved, so nothing else was done: {why}")
            },
        ));
    }
    Ok(())
}

/// What the open note now is, read back out of the app and off the disk rather than predicted.
///
/// Every action that touches a note answers with this, so "it worked" is never the app's opinion
/// of its own handler: `matches_disk` is the file compared against the text in the window.
fn written(ui: &NotesApp, s: &State) -> Result<serde_json::Value, String> {
    let b = s.borrow();
    let Some(n) = b.current.as_ref() else {
        return Err(refuse(ui, "No note is open, so there is nothing to report."));
    };
    let path = b.dir.join(&n.id);
    let disk = store::read(&path, store::LIMIT).ok().flatten();
    Ok(serde_json::json!({
        "filename": n.id,
        "title": n.title(),
        "path": path.display().to_string(),
        "lines": n.text.lines().count(),
        "characters": n.text.chars().count(),
        "words": n.text.split_whitespace().count(),
        "saved": !n.dirty(),
        "on_disk": disk.is_some(),
        "matches_disk": disk.as_deref() == Some(n.text.as_str()),
        "trashed": n.trash,
        "notebook": n.field("notebook"),
        "tags": n.field("tags"),
        "notice": ui.get_notice().to_string(),
    }))
}

/// The one line a mind reads first.
///
/// This used to be the word "Notes". The `os_apps` listing prints one summary per open window, so
/// that line was the whole of what a mind knew about this app before deciding to look closer —
/// and it did not say whether anything was open, let alone which note or whether it was saved.
fn view(ui: &NotesApp, s: &State) -> View {
    let b = s.borrow();
    let notes = b.notes.iter().filter(|n| !n.trash).count();
    let trashed = b.notes.iter().filter(|n| n.trash).count();
    let library = format!(
        "{notes} note{}{}",
        if notes == 1 { "" } else { "s" },
        if trashed == 0 { String::new() } else { format!(", {trashed} in trash") }
    );
    let open = b.current.as_ref();
    let mut summary = match open {
        _ if !b.ready => "Notes — the library has not opened".to_string(),
        None => format!("Notes — nothing open, {library}"),
        Some(n) => format!(
            "Notes — \"{}\" open{} ({} line{}, {}), {library}",
            n.title(),
            if n.trash { " from Trash" } else { "" },
            n.text.lines().count(),
            if n.text.lines().count() == 1 { "" } else { "s" },
            if n.trash {
                "deleted"
            } else if n.dirty() {
                "unsaved"
            } else {
                "saved"
            }
        ),
    };
    if ui.get_busy() {
        summary.push_str(" · working");
    }
    let notice = ui.get_notice().to_string();
    if !notice.is_empty() {
        summary.push_str(&format!(" · {notice}"));
    }
    View::new(summary)
        .with("open_note", open.map(|n| n.id.clone()))
        .with("title", open.map(Note::title))
        .with("path", open.map(|n| b.dir.join(&n.id).display().to_string()))
        .with("unsaved", open.is_some_and(Note::dirty))
        .with("lines", open.map(|n| n.text.lines().count() as i64))
        .with("characters", open.map(|n| n.text.chars().count() as i64))
        .with("trashed_note_open", open.is_some_and(|n| n.trash))
        .with("content", open.map(|n| n.text.chars().take(4000).collect::<String>()))
        // The `.meta` sidecar travels with the text — `tags`, `notebook` and `pinned` live in it —
        // so it belongs in the view a revision is hashed from. It did not, and `tags` rewrote a
        // note's metadata while answering with the same revision as the call before it: a caller
        // whose "unchanged since I read it" check compares revisions was told a changed note had
        // not changed.
        .with("metadata", open.map(|n| n.meta.clone()))
        .with("notes_directory", b.dir.display().to_string())
        .with("library_ready", b.ready)
        .with("busy", ui.get_busy())
        .with("notice", notice)
        .with("status", ui.get_status().to_string())
        .with("folder", ui.get_folder().to_string())
        .with("preview", ui.get_preview())
        .with("focus", ui.get_focus_mode())
        .with("note_count", notes as i64)
        .with("trash_count", trashed as i64)
        .with("search_query", ui.get_query().to_string())
        .with("matches", {
            use slint::Model;
            ui.get_notes().row_count() as i64
        })
        .with(
            "notes",
            b.notes
                .iter()
                .take(100)
                .map(|n| {
                    serde_json::json!({
                        "filename": n.id,
                        "title": n.title(),
                        "trash": n.trash,
                        "pinned": n.field("pinned") == "true",
                        "notebook": n.field("notebook"),
                    })
                })
                .collect::<Vec<_>>(),
        )
}

/// One published action: what it says it does, and the code that does it.
type Handler = Box<dyn Fn(&serde_json::Value) -> Result<serde_json::Value, String>>;

/// Everything this window offers a mind.
///
/// Built as a list rather than pushed straight into `App` so the tests can read exactly what a
/// mind is shown — since d73760d, `yos describe` prints every action's description and every
/// argument's type, required flag and description, so what is written here is verbatim what the
/// model reads — and can run a handler without a socket or an event loop.
fn surface(ui: &NotesApp, s: &State) -> Vec<(Action, Handler)> {
    let window = {
        let weak = ui.as_weak();
        move || weak.upgrade().ok_or_else(|| "The Notes window is gone.".to_string())
    };
    let mut out: Vec<(Action, Handler)> = Vec::new();
    let mut add =
        |spec: Action,
         run: fn(&NotesApp, &State, &serde_json::Value) -> Result<serde_json::Value, String>| {
            let window = window.clone();
            let state = s.clone();
            out.push((
                spec,
                Box::new(move |args: &serde_json::Value| {
                    let ui = window()?;
                    if ui.get_busy() {
                        return Err(
                            "Notes is in the middle of a file operation; read `describe` again \
                             in a moment."
                                .to_string(),
                        );
                    }
                    run(&ui, &state, args)
                }) as Handler,
            ));
        };

    add(
        act(
            "new_note",
            "Create a note with this title, put `text` in it if given, save it, and open it. The \
             title is written as the note's first line, `# <title>`, which is what the library \
             lists it by; whatever note was open is saved first.",
        )
        .arg(arg(
            "title",
            "The new note's title. It is written as the first line, `# <title>`, and is what \
             `open_note` and `search` find it by.",
        ))
        .arg(
            arg(
                "text",
                "The body, written under the title on its own lines. Leave it out for a note \
                 that is only a heading; `append` can add to it later.",
            )
            .optional(),
        ),
        |ui, s, args| {
            let title = needed(
                ui,
                args,
                "new_note",
                "title",
                "It becomes the note's `# heading` line — for example title=\"Groceries\".",
            )?;
            let body = given(args, "text");
            flush(ui, s)?;
            new(ui, s, &title, &body)?;
            settle(ui, s);
            written(ui, s)
        },
    );

    add(
        act(
            "open_note",
            "Open an existing note by its exact title or its filename and show it in the editor. \
             The note that was open is saved first; a note in the Trash opens read-only.",
        )
        .arg(arg(
            "title",
            "The note's exact title, or the filename `describe` lists it under (`note-….md`). \
             Trashed notes are matched too.",
        )),
        |ui, s, args| {
            let wanted = needed(
                ui,
                args,
                "open_note",
                "title",
                "Use a title or filename from `describe.notes`.",
            )?;
            let key = {
                let b = s.borrow();
                let n = b
                    .notes
                    .iter()
                    .find(|n| n.id == wanted || n.title() == wanted)
                    .ok_or_else(|| {
                        let titles: Vec<String> = b
                            .notes
                            .iter()
                            .filter(|n| !n.trash)
                            .take(8)
                            .map(|n| format!("\"{}\"", n.title()))
                            .collect();
                        refuse(
                            ui,
                            format!(
                                "No note is called \"{wanted}\". The library holds: {}",
                                if titles.is_empty() {
                                    "nothing yet".to_string()
                                } else {
                                    titles.join(", ")
                                }
                            ),
                        )
                    })?;
                format!("open:{}{}", if n.trash { "trash:" } else { "" }, n.id)
            };
            flush(ui, s)?;
            action(ui, s, &key);
            settle(ui, s);
            written(ui, s)
        },
    );

    add(
        act(
            "set_title",
            "Rename the note that is open by rewriting its first `# heading` line, adding one at \
             the top if it has none, and save it. Nothing else in the note changes.",
        )
        .arg(arg(
            "title",
            "The new title, on one line; any line breaks in it become spaces.",
        )),
        |ui, s, args| {
            let title = needed(ui, args, "set_title", "title", "It replaces the `# heading` line.")?;
            let mut text = open_text(ui, s)?;
            let title = title.replace(['\r', '\n'], " ");
            let mut replaced = false;
            text = text
                .lines()
                .map(|l| {
                    if !replaced && l.starts_with("# ") {
                        replaced = true;
                        format!("# {title}")
                    } else {
                        l.into()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !replaced {
                text = format!("# {title}\n\n{text}");
            }
            commit(ui, s, text)
        },
    );

    add(
        act(
            "append",
            "Add text to the end of the note that is open and save it. Nothing already in the \
             note is removed; use `set_content` to replace it instead.",
        )
        .arg(arg(
            "text",
            "Exactly what to add, appended to the last character already there — begin it with \
             a newline to start it on its own line.",
        )),
        |ui, s, args| {
            let addition = needed(
                ui,
                args,
                "append",
                "text",
                "There is nothing to append without it, and the note was left untouched.",
            )?;
            let mut text = open_text(ui, s)?;
            text.push_str(&addition);
            commit(ui, s, text)
        },
    );

    add(
        // Sensitive: it overwrites the whole note, unsaved paragraphs included. What it replaces
        // is on disk only if it had been saved, and `undo` only reaches it while this window
        // stays on this note.
        act(
            "set_content",
            "Replace everything in the note that is open with this text and save it. What was \
             there is gone from the window; `undo` takes it back while the note stays open.",
        )
        .risk("sensitive")
        .arg(arg(
            "text",
            "The note's entire new text, Markdown included — keep the `# <title>` first line if \
             the note should keep its title. An empty string empties the note.",
        )),
        |ui, s, args| {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| refuse(ui, "`set_content` needs `text`: the note's entire new text."))?
                .to_string();
            open_text(ui, s)?;
            commit(ui, s, text)
        },
    );

    add(
        act(
            "search",
            "Filter the library list to the notes whose text or metadata contains this, without \
             opening anything. `set_folder` decides which section is searched.",
        )
        .arg(arg(
            "query",
            "Matched case-insensitively against each note's text and its notebook and tags; an \
             empty string clears the filter.",
        )),
        |ui, s, args| {
            let query = given(args, "query");
            ui.set_query(query.clone().into());
            list(ui, s);
            let (matches, titles) = listed(ui);
            Ok(serde_json::json!({
                "query": query,
                "folder": ui.get_folder().to_string(),
                "matches": matches,
                "titles": titles,
            }))
        },
    );

    add(
        act(
            "set_folder",
            "Switch the library list to one section of the library. It changes what `search` and \
             `describe.matches` list, and opens nothing.",
        )
        .arg(arg(
            "folder",
            "One of `all`, `recent` (touched in the last seven days), `favorites` (pinned), \
             `trash`, or `book:<notebook name>`.",
        )),
        |ui, s, args| {
            let folder = needed(
                ui,
                args,
                "set_folder",
                "folder",
                "One of `all`, `recent`, `favorites`, `trash`, `book:<name>`.",
            )?;
            ui.set_folder(folder.clone().into());
            list(ui, s);
            let (matches, titles) = listed(ui);
            Ok(serde_json::json!({ "folder": folder, "listed": matches, "titles": titles }))
        },
    );

    add(
        act(
            "notebook",
            "File the note that is open under a notebook, which is the section `set_folder \
             book:<name>` lists, and save it. It replaces whatever notebook it was in.",
        )
        .arg(arg(
            "text",
            "The notebook's name, up to 80 characters; an empty string files the note under none.",
        )),
        |ui, s, args| {
            open_text(ui, s)?;
            ui.set_notebook(given(args, "text").into());
            ui.invoke_metadata();
            save(ui, s);
            settle(ui, s);
            written(ui, s)
        },
    );

    add(
        act(
            "tags",
            "Replace the tags on the note that is open and save it. Tags are searched by \
             `search` along with the note's text.",
        )
        .arg(arg(
            "text",
            "Comma-separated tags, e.g. `work, ideas`, up to 512 characters; an empty string \
             clears them.",
        )),
        |ui, s, args| {
            open_text(ui, s)?;
            ui.set_tags(given(args, "text").into());
            ui.invoke_metadata();
            save(ui, s);
            settle(ui, s);
            written(ui, s)
        },
    );

    add(
        act(
            "save",
            "Write the note that is open to its file now instead of waiting for the 750 ms \
             autosave, and check that the bytes on disk are the text in the window.",
        ),
        |ui, s, _| {
            open_text(ui, s)?;
            save(ui, s);
            settle(ui, s);
            let answer = written(ui, s)?;
            if answer["matches_disk"] != true {
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        format!(
                            "The note was not written to {}; the draft is still open.",
                            answer["path"].as_str().unwrap_or_default()
                        )
                    } else {
                        why
                    },
                ));
            }
            Ok(answer)
        },
    );

    add(
        // Sensitive: the note leaves the library and its file is removed from the notes
        // directory. It is not `dangerous` because `restore` puts back exactly this note from
        // the Trash record written before the file is unlinked.
        act(
            "trash",
            "Move the note that is open to the Trash: it leaves the library, its file is removed \
             from the notes directory, and `restore` puts it back.",
        )
        .risk("sensitive"),
        |ui, s, _| {
            let (id, title, path) = {
                let b = s.borrow();
                let n = b
                    .current
                    .as_ref()
                    .ok_or_else(|| refuse(ui, "No note is open, so there is nothing to trash."))?;
                if n.trash {
                    return Err(refuse(ui, "That note is already in the Trash."));
                }
                (n.id.clone(), n.title(), b.dir.join(&n.id))
            };
            action(ui, s, "trash");
            settle(ui, s);
            let b = s.borrow();
            let gone = b.notes.iter().any(|n| n.id == id && n.trash);
            if !gone {
                let why = ui.get_notice().to_string();
                drop(b);
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        format!("\"{title}\" is still in the library; nothing was trashed.")
                    } else {
                        why
                    },
                ));
            }
            Ok(serde_json::json!({
                "trashed": title,
                "filename": id,
                "file_removed": !path.exists(),
                "in_trash": true,
                "trash_count": b.notes.iter().filter(|n| n.trash).count(),
                "notes": b.notes.iter().filter(|n| !n.trash).count(),
                "open_note": serde_json::Value::Null,
            }))
        },
    );

    add(
        act(
            "restore",
            "Put the trashed note that is open back into the library, writing its file into the \
             notes directory again. It refuses rather than overwrite a note of the same filename.",
        ),
        |ui, s, _| {
            let id = {
                let b = s.borrow();
                let n = b
                    .current
                    .as_ref()
                    .ok_or_else(|| refuse(ui, "No note is open, so there is nothing to restore."))?;
                if !n.trash {
                    return Err(refuse(
                        ui,
                        "That note is in the library already; only a trashed note can be \
                         restored. Open it from `set_folder trash` first.",
                    ));
                }
                n.id.clone()
            };
            action(ui, s, "restore");
            settle(ui, s);
            if s.borrow().notes.iter().any(|n| n.id == id && n.trash) {
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        "The note is still in the Trash; nothing was restored.".to_string()
                    } else {
                        why
                    },
                ));
            }
            written(ui, s)
        },
    );

    add(
        act(
            "copy",
            "Duplicate the note that is open as a new note with its own filename, save that, and \
             open it. Unsaved changes go with the copy — it is the way out of \"this note changed \
             on disk\" — and the original file is left as it is.",
        ),
        |ui, s, _| {
            let from = {
                let b = s.borrow();
                let n = b
                    .current
                    .as_ref()
                    .ok_or_else(|| refuse(ui, "No note is open, so there is nothing to copy."))?;
                n.id.clone()
            };
            action(ui, s, "copy");
            settle(ui, s);
            let mut answer = written(ui, s)?;
            if answer["filename"] == serde_json::json!(from) {
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        "Nothing was copied; the same note is still open.".to_string()
                    } else {
                        why
                    },
                ));
            }
            answer["copied_from"] = serde_json::json!(from);
            Ok(answer)
        },
    );

    add(
        act(
            "reload",
            "Re-read every note from the notes directory, picking up files changed outside this \
             window. The open note is saved first and stays open if its file is still there.",
        ),
        |ui, s, _| {
            // Not `flush(..)?`: a library that failed to open is exactly what `reload` is for,
            // and that is the one case `flush` refuses on. A draft that could not be saved is a
            // different matter — the window queues the reload behind the save, so it never runs —
            // and that is caught below rather than reported as a reload that happened.
            flush(ui, s).ok();
            action(ui, s, "reload");
            settle(ui, s);
            if let Some(queued) = s.borrow().pending.clone() {
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    format!(
                        "Nothing was reloaded: `{queued}` is still waiting on the open note, \
                         which could not be saved{}",
                        if why.is_empty() { ".".to_string() } else { format!(" — {why}") }
                    ),
                ));
            }
            let b = s.borrow();
            Ok(serde_json::json!({
                "notes": b.notes.iter().filter(|n| !n.trash).count(),
                "in_trash": b.notes.iter().filter(|n| n.trash).count(),
                "open_note": b.current.as_ref().map(|n| n.id.clone()),
                "library_ready": b.ready,
                "notice": ui.get_notice().to_string(),
            }))
        },
    );

    add(
        act(
            "preview",
            "Turn the rendered Markdown preview of the open note on or off — it is a toggle, and \
             the answer says which it now is. The note itself is not changed.",
        ),
        |ui, s, _| {
            action(ui, s, "preview");
            let blocks = {
                use slint::Model;
                ui.get_blocks().row_count()
            };
            Ok(serde_json::json!({
                "preview": ui.get_preview(),
                "blocks": blocks,
                "title": s.borrow().current.as_ref().map(Note::title),
            }))
        },
    );

    add(
        act(
            "focus",
            "Turn focus mode, which hides the library list and the side panels, on or off — it \
             is a toggle, and the answer says which it now is.",
        ),
        |ui, s, _| {
            action(ui, s, "focus");
            Ok(serde_json::json!({
                "focus_mode": ui.get_focus_mode(),
                "open_note": s.borrow().current.as_ref().map(Note::title),
            }))
        },
    );

    add(
        act(
            "undo",
            "Take back the last change made to the note that is open in this window, and save \
             the result. The history is cleared whenever another note is opened.",
        ),
        |ui, s, _| {
            let before = open_text(ui, s)?;
            undo(ui, s, false);
            if s.borrow().current.as_ref().is_some_and(|n| n.text == before) {
                return Err(refuse(
                    ui,
                    "There is nothing left to undo on this note in this window.",
                ));
            }
            save(ui, s);
            settle(ui, s);
            written(ui, s)
        },
    );

    add(
        act(
            "redo",
            "Put back the change that `undo` took off the note that is open, and save the \
             result. Editing the note discards what redo was holding.",
        ),
        |ui, s, _| {
            let before = open_text(ui, s)?;
            undo(ui, s, true);
            if s.borrow().current.as_ref().is_some_and(|n| n.text == before) {
                return Err(refuse(ui, "There is nothing to redo on this note."));
            }
            save(ui, s);
            settle(ui, s);
            written(ui, s)
        },
    );

    add(
        // Sensitive: it reads a file from anywhere this user can read, outside the notes
        // directory, and copies its contents into the library.
        act(
            "import",
            "Read a UTF-8 text or Markdown file from anywhere on disk and add its contents to \
             the library as a new note, then open it. The file itself is left where it is.",
        )
        .risk("sensitive")
        .arg(arg(
            "path",
            "Absolute path to the file to read, or one starting `~/`. Up to 256 KiB of text with \
             no control characters.",
        )),
        |ui, s, args| {
            let path = needed(ui, args, "import", "path", "An absolute path to a text file.")?;
            let full = expanded(path.trim());
            if !full.is_absolute() {
                return Err(refuse(ui, format!("`import` needs an absolute path; `{path}` is not one.")));
            }
            flush(ui, s)?;
            ui.set_dialog(1);
            ui.set_path(path.clone().into());
            action(ui, s, "confirm");
            settle(ui, s);
            if ui.get_dialog() != 0 {
                ui.set_dialog(0);
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        format!("Nothing was imported from {}.", full.display())
                    } else {
                        why
                    },
                ));
            }
            let mut answer = written(ui, s)?;
            answer["imported_from"] = serde_json::json!(full.display().to_string());
            Ok(answer)
        },
    );

    add(
        // Sensitive: it writes a file outside the notes directory, at a path the caller chooses.
        // It cannot overwrite — the store publishes the new file with a hard link, which fails if
        // something is already there — so it is not `dangerous`.
        act(
            "export",
            "Write the open note's text to a file outside the library, at this path. It refuses \
             rather than overwrite anything already there, and the note stays open.",
        )
        .risk("sensitive")
        .arg(arg(
            "path",
            "Absolute path of the file to create, or one starting `~/`. Its folder must exist \
             and nothing may already be at that path.",
        )),
        |ui, s, args| {
            let path = needed(ui, args, "export", "path", "An absolute path to write to.")?;
            let full = expanded(path.trim());
            if !full.is_absolute() {
                return Err(refuse(ui, format!("`export` needs an absolute path; `{path}` is not one.")));
            }
            let text = open_text(ui, s)?;
            flush(ui, s)?;
            ui.set_dialog(2);
            ui.set_path(path.clone().into());
            action(ui, s, "confirm");
            settle(ui, s);
            let written_bytes = store::read(&full, store::LIMIT).ok().flatten();
            if ui.get_dialog() != 0 || written_bytes.is_none() {
                ui.set_dialog(0);
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        format!("Nothing was written to {}.", full.display())
                    } else {
                        why
                    },
                ));
            }
            Ok(serde_json::json!({
                "path": full.display().to_string(),
                "bytes": written_bytes.as_deref().map(str::len),
                "matches_note": written_bytes.as_deref() == Some(text.as_str()),
                "title": s.borrow().current.as_ref().map(Note::title),
            }))
        },
    );

    out
}

/// The open note's text, or a refusal that says what to do about it.
///
/// Every editing action starts here rather than reaching into `current` itself, so "no note is
/// open" and "that note is in the Trash" are one sentence each instead of eighteen.
fn open_text(ui: &NotesApp, s: &State) -> Result<String, String> {
    let b = s.borrow();
    match b.current.as_ref() {
        None => Err(refuse(
            ui,
            "No note is open. Open one with `open_note`, or make one with `new_note`.",
        )),
        Some(n) if n.trash => Err(refuse(
            ui,
            "The open note is in the Trash and cannot be edited; `restore` puts it back first.",
        )),
        Some(n) => Ok(n.text.clone()),
    }
}

/// Put new text in the open note, save it, and report what the note now holds.
fn commit(ui: &NotesApp, s: &State, text: String) -> Result<serde_json::Value, String> {
    store::validate(&text).map_err(|e| refuse(ui, e))?;
    ui.set_content(text.clone().into());
    edit(ui, s, text);
    save(ui, s);
    settle(ui, s);
    written(ui, s)
}

/// What the library list is showing now: how many rows, and the first few titles.
fn listed(ui: &NotesApp) -> (usize, Vec<String>) {
    use slint::Model;
    let rows = ui.get_notes();
    let titles = (0..rows.row_count().min(5))
        .filter_map(|i| rows.row_data(i))
        .map(|r| r.title.to_string())
        .collect();
    (rows.row_count(), titles)
}

fn control(ui: &NotesApp, s: &State) {
    let weak = ui.as_weak();
    let state = s.clone();
    let mut app = App::new("notes").describe(move || match weak.upgrade() {
        Some(ui) => view(&ui, &state),
        None => View::new("Notes — the window is closed"),
    });
    for (spec, run) in surface(ui, s) {
        app = app.action(spec, run);
    }
    app.serve();
}
#[cfg(test)]
mod tests;
