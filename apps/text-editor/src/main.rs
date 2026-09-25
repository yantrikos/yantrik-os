//! Native, bounded text workbench. All document I/O runs on one worker.
mod document;
use document::{Document, MAX_TABS};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};
use yantrik_app_runtime::control::{Action, App, Param, View};
use yantrik_app_runtime::prelude::*;
// The watch on the folder the open file lives in, and the decisions behind following a move.
// Shared with yDoc (#86): the Text Editor's stranded-save pair was the one yDoc had in #75.
use yantrik_file_follow as follow;
slint::include_modules!();

enum Job {
    Open(PathBuf),
    Save(Document, PathBuf, bool),
    Recovery(Vec<Document>, u64),
    Shutdown(Vec<Document>),
}
enum Event {
    Open(Result<Document, String>),
    Saved(Result<Document, String>),
    Recovery(Result<(), String>, u64),
}
struct Workbench {
    docs: Vec<Document>,
    active: usize,
    matches: Vec<(usize, usize)>,
    match_index: usize,
    pending_close: Option<usize>,
    quitting: bool,
    save_close: bool,
    jobs: mpsc::Sender<Job>,
    events: mpsc::Receiver<Event>,
    /// The watch on the folder the active tab's file lives in, re-pointed by `paint` as the tabs
    /// change. See `yantrik-file-follow` for why an editor watches a folder at all.
    follow: follow::Watch,
    /// What that watch has seen, waiting for the UI thread to come and read it.
    moves: mpsc::Receiver<follow::Move>,
    /// The path the watch is pointed at: the active tab's file as `paint` last saw it. A move
    /// event is about THIS file, which by the time it is read may belong to a background tab.
    watched: Option<PathBuf>,
    recovery_timer: slint::Timer,
    recovery_generation: u64,
    worker: Option<std::thread::JoinHandle<()>>,
}
type State = Rc<RefCell<Workbench>>;
fn expanded(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
    } else {
        PathBuf::from(value)
    }
}
fn main() {
    init_tracing("yantrik-text-editor");
    // A VM explicitly forcing software OpenGL has no GPU to accelerate femtovg.
    // Render directly on the CPU instead; retain any explicit renderer choice.
    if std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref() == Ok("1")
        && matches!(
            std::env::var("SLINT_BACKEND").as_deref(),
            Ok("winit") | Err(_)
        )
    {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    // flock closes the launch race and releases automatically after a crash.
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let lock_path =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR is required"))
            .join("yantrik-editor.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(lock_path)
        .expect("editor instance lock");
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let args = path
            .map(|p| serde_json::json!({"action":"open","args":{"path":p}}))
            .unwrap_or_else(|| serde_json::json!({"action":"show","args":{}}));
        let client = SyncRpcClient::for_service("app-editor").with_timeout(Duration::from_secs(3));
        for _ in 0..20 {
            if let Ok(reply) = client.call("app.act", args.clone()) {
                if reply["accepted"] == true {
                    focus();
                    return;
                } else {
                    eprintln!("Editor declined request: {reply}");
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("Text Editor is already starting; try opening the file again.");
        return;
    }
    let ui = TextEditorApp::new().unwrap();
    // The bar is the app's own (#256); its close goes through the unsaved-changes question below.
    yantrik_app_runtime::window_chrome!(ui);
    let prefs = theme::load();
    ui.global::<ThemeMode>().set_dark(prefs.dark);
    ui.global::<AccentPreset>().set_index(prefs.accent_index);
    let state = wire(&ui, document::recovery_path(), true);
    if let Some(path) = path {
        open(&ui, &state, path);
    }
    run_until_closed(&ui, "yantrik-text-editor");
    // Synchronous final checkpoint only after the window event loop has stopped.
    let mut b = state.borrow_mut();
    b.recovery_timer.stop();
    let _ = b.jobs.send(Job::Shutdown(b.docs.clone()));
    if let Some(worker) = b.worker.take() {
        let _ = worker.join();
    }
}
fn focus() {
    let _ = std::process::Command::new("wlrctl")
        .args(["toplevel", "focus", "title:Editor"])
        .status();
}
fn wire(ui: &TextEditorApp, recovery_path: PathBuf, publish: bool) -> State {
    let (jobs, work) = mpsc::channel();
    let (results, events) = mpsc::channel();
    let weak = ui.as_weak();
    let recovery = document::recover(&recovery_path);
    let recovery_ok = recovery.is_ok();
    let docs = match recovery {
        Ok(d) if !d.is_empty() => {
            ui.set_notice("Recovered unsaved drafts. Review them, then Save or Save As.".into());
            d
        }
        Ok(_) => vec![Document::blank()],
        Err(e) => {
            ui.set_notice(e.into());
            vec![Document::blank()]
        }
    };
    let worker = std::thread::spawn(move || {
        while let Ok(job) = work.recv() {
            let event = match job {
                Job::Open(p) => Event::Open(Document::open(&p)),
                Job::Save(d, p, overwrite) => {
                    Event::Saved(if overwrite { d.save_over(&p) } else { d.save(&p) })
                }
                Job::Recovery(d, g) => Event::Recovery(
                    if recovery_ok {
                        document::checkpoint(&recovery_path, &d)
                    } else {
                        Err("Existing recovery file is unreadable and has been preserved.".into())
                    },
                    g,
                ),
                Job::Shutdown(d) => {
                    if recovery_ok {
                        let _ = document::checkpoint(&recovery_path, &d);
                    }
                    break;
                }
            };
            if results.send(event).is_err() {
                break;
            }
            let _ = weak.upgrade_in_event_loop(|u| u.invoke_refresh());
        }
    });
    // The folder watch and its mailbox. The watcher's own thread cannot touch the documents, so
    // it puts what it saw on a channel and pokes `refresh` — the same callback the file worker
    // wakes, so a move is read in `receive` beside the saves and opens.
    let (moves, inbox) = mpsc::channel();
    let wake = ui.as_weak();
    let follow = follow::Watch::new(moves, move || {
        let _ = wake.upgrade_in_event_loop(|u| u.invoke_refresh());
    });
    let state = Rc::new(RefCell::new(Workbench {
        docs,
        active: 0,
        matches: vec![],
        match_index: 0,
        pending_close: None,
        quitting: false,
        save_close: false,
        jobs,
        events,
        follow,
        moves: inbox,
        watched: None,
        recovery_timer: slint::Timer::default(),
        recovery_generation: 0,
        worker: Some(worker),
    }));
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_refresh(move || {
        if let Some(u) = weak.upgrade() {
            receive(&u, &s);
        }
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_action(move |id| {
        if let Some(u) = weak.upgrade() {
            action(&u, &s, &id);
        }
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_select_tab(move |index| {
        if let Some(u) = weak.upgrade() {
            if u.get_busy() || u.get_dialog() != 0 {
                return;
            }
            let mut b = s.borrow_mut();
            if index >= 0 && (index as usize) < b.docs.len() {
                b.active = index as usize;
                paint(&u, &mut b, true);
                drop(b);
                search(&u, &s, false);
            }
        }
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_close_tab(move |index| {
        if let Some(u) = weak.upgrade() {
            request_close(&u, &s, index as usize, false);
        }
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_edited(move |text| {
        if let Some(u) = weak.upgrade() {
            edit(&u, &s, text.to_string());
        }
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_search(move || {
        if let Some(u) = weak.upgrade() {
            search(&u, &s, true);
        }
    });
    let weak = ui.as_weak();
    ui.on_cursor(move |offset| {
        if let Some(u) = weak.upgrade() {
            let text = u.get_content();
            let end = (offset.max(0) as usize).min(text.len());
            if let Some(prefix) = text.get(..end) {
                u.set_cursor_line(prefix.bytes().filter(|b| *b == b'\n').count() as i32 + 1);
                u.set_cursor_column(
                    prefix.rsplit('\n').next().unwrap_or("").chars().count() as i32 + 1,
                );
            }
        }
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.window().on_close_requested(move || {
        if let Some(u) = weak.upgrade() {
            if !u.get_busy() {
                request_close(&u, &s, 0, true);
            }
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    paint(ui, &mut state.borrow_mut(), true);
    ui.invoke_focus_editor();
    if publish {
        publish_control(ui, state.clone());
    }
    state
}
fn paint(ui: &TextEditorApp, b: &mut Workbench, content: bool) {
    let d = &b.docs[b.active];
    ui.set_tabs(ModelRc::new(VecModel::from(
        b.docs
            .iter()
            .enumerate()
            .map(|(i, d)| DocumentTab {
                title: d.title().into(),
                active: i == b.active,
                modified: d.dirty(),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_document_title(d.title().into());
    ui.set_modified(d.dirty());
    ui.set_path_label(
        d.path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Untitled · choose a home with Save As".into())
            .into(),
    );
    ui.set_language(document::language(d.path.as_deref()).into());
    if content {
        ui.set_content(d.text.clone().into());
        ui.invoke_reset_position();
        ui.invoke_focus_editor();
    }
    let lines = d.text.bytes().filter(|b| *b == b'\n').count() + 1;
    ui.set_line_count(lines as i32);
    ui.set_numbers(
        (1..=lines)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .into(),
    );
    ui.set_ending(
        if d.text.contains("\r\n") {
            "CRLF"
        } else {
            "LF"
        }
        .into(),
    );
    let (a, c, e) = highlight(&d.text, document::language(d.path.as_deref()));
    ui.set_has_highlights(!a.is_empty());
    ui.set_keywords(a.into());
    ui.set_strings(c.into());
    ui.set_comments(e.into());
    // Re-point the folder watch at wherever the active tab's file lives now. Called from `paint`
    // rather than from each of the places that change a path, because `paint` is the one function
    // every one of them already goes through; re-pointing at the same folder is a no-op.
    b.watched = b.docs[b.active].path.clone();
    b.follow.point_at(b.watched.as_deref());
}
/// Small lexical highlighter. No parser service, background polling or per-token UI nodes.
fn highlight(text: &str, language: &str) -> (String, String, String) {
    if text.len() > 65536 || language == "Plain text" {
        return Default::default();
    }
    let mut layers = [String::new(), String::new(), String::new()];
    let words = [
        "fn", "let", "mut", "pub", "use", "mod", "struct", "impl", "enum", "match", "if", "else",
        "return", "for", "in", "while", "loop", "true", "false", "None", "Some", "const", "def",
        "class", "import", "from", "as", "with", "try", "except", "async", "await", "function",
        "export", "default", "var", "null", "new", "self",
    ];
    for line in text.split_inclusive('\n') {
        let mut masks = [
            vec![b' '; line.len()],
            vec![b' '; line.len()],
            vec![b' '; line.len()],
        ];
        for (i, b) in line.bytes().enumerate() {
            if b == b'\n' || b == b'\r' {
                for m in &mut masks {
                    m[i] = b;
                }
            }
        }
        if line.is_ascii() && !line.contains('\t') {
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if (bytes[i] == b'#'
                    && matches!(language, "Python" | "Shell" | "TOML" | "Markdown"))
                    || (bytes[i..].starts_with(b"//") && language != "JSON")
                {
                    masks[2][i..].copy_from_slice(&bytes[i..]);
                    break;
                }
                if matches!(bytes[i], b'\'' | b'"') {
                    let start = i;
                    let quote = bytes[i];
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == b'\\' {
                            i = (i + 2).min(bytes.len());
                            continue;
                        }
                        if bytes[i] == quote {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    masks[1][start..i].copy_from_slice(&bytes[start..i]);
                    continue;
                }
                if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                    {
                        i += 1
                    }
                    if words.contains(&&line[start..i]) {
                        masks[0][start..i].copy_from_slice(&bytes[start..i]);
                    }
                    continue;
                }
                i += 1;
            }
        }
        for (out, mask) in layers.iter_mut().zip(masks) {
            out.push_str(&String::from_utf8(mask).unwrap());
        }
    }
    let [a, b, c] = layers;
    (a, b, c)
}
fn checkpoint(ui: &TextEditorApp, state: &State) {
    let mut b = state.borrow_mut();
    b.recovery_generation += 1;
    let generation = b.recovery_generation;
    let docs: Vec<_> = b.docs.iter().map(Document::snapshot).collect();
    let jobs = b.jobs.clone();
    ui.set_recovery_status("Protecting draft…".into());
    b.recovery_timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(750),
        move || {
            let _ = jobs.send(Job::Recovery(docs.clone(), generation));
        },
    );
}
fn edit(ui: &TextEditorApp, s: &State, text: String) {
    if ui.get_busy() {
        return;
    }
    let mut b = s.borrow_mut();
    if let Err(e) = document::validate(&text) {
        ui.set_content(b.docs[b.active].text.clone().into());
        ui.set_notice(e.into());
        return;
    }
    let active = b.active;
    b.docs[active].edit(text);
    paint(ui, &mut b, false);
    drop(b);
    search(ui, s, false);
    checkpoint(ui, s);
}
fn search(ui: &TextEditorApp, s: &State, select: bool) {
    let mut b = s.borrow_mut();
    b.matches = document::matches(&b.docs[b.active].text, &ui.get_query(), ui.get_match_case());
    b.match_index = 0;
    ui.set_match_count(b.matches.len() as i32);
    ui.set_match_index(if b.matches.is_empty() { 0 } else { 1 });
    if select {
        if let Some(&(a, z)) = b.matches.first() {
            ui.invoke_select_range(a as i32, z as i32);
        }
    }
}
fn open(ui: &TextEditorApp, s: &State, path: PathBuf) {
    if ui.get_busy() {
        ui.set_notice("Finish the current file operation first.".into());
        return;
    }
    if ui.get_dialog() == 3 {
        ui.set_notice("Resolve the unsaved changes dialog first.".into());
        return;
    }
    ui.set_busy(true);
    ui.set_notice("Opening file…".into());
    let _ = s.borrow().jobs.send(Job::Open(path));
}
/// Send the active tab's text to the worker to be written.
///
/// `overwrite` is the caller having been told that something is already at `path` and answering
/// anyway; nothing in the window passes it — the Save As prompt still refuses to write over a
/// file it was not expecting, and the one case where that refusal was wrong (the target IS this
/// tab's own file, moved) is recognised inside `Document` and needs no permission.
fn save(ui: &TextEditorApp, s: &State, path: Option<PathBuf>, overwrite: bool) {
    let b = s.borrow();
    let d = b.docs[b.active].snapshot();
    let Some(path) = path.or_else(|| d.path.clone()) else {
        drop(b);
        action(ui, s, "save-as");
        return;
    };
    ui.set_busy(true);
    ui.set_notice("Saving…".into());
    ui.set_dialog_error("".into());
    let _ = b.jobs.send(Job::Save(d, path, overwrite));
}
/// The folder watch has reported a move. Follow it, if it was one of ours.
///
/// This is what closes #86 while the window is open: a file moved in Files goes on being the
/// same tab, saved by Save, named correctly in the window and in `describe`. Only the path
/// changes — the text and the baseline are untouched, because a rename moves the same bytes and
/// the same inode, and a save's conflict check is still asking about the right file afterwards.
fn follow_moves(ui: &TextEditorApp, s: &State) {
    // Everything waiting, read as one decision. A rename arrives as two events — "it left", then
    // "and here is where it went" — and acting on the first would send the app hunting through a
    // directory for a file the second event is about to name.
    let mut waiting = Vec::new();
    loop {
        let next = s.borrow().moves.try_recv();
        match next {
            Ok(moved) => waiting.push(moved),
            Err(_) => break,
        }
    }
    let Some(moved) = follow::latest(waiting) else { return };
    // The event is about the file the watch was pointed at, which may sit in a tab that is no
    // longer the active one — find that tab by the path, not by position.
    let Some(watched) = s.borrow().watched.clone() else { return };
    let now = match moved {
        follow::Move::To(to) => Some(to),
        // The event said only that the file left. Where it went is a look at the disk.
        follow::Move::Away => {
            let baseline = s
                .borrow()
                .docs
                .iter()
                .find(|d| d.path.as_ref() == Some(&watched))
                .map(|d| d.baseline.clone());
            baseline.and_then(|b| document::moved_to(&watched, &b))
        }
    };
    match now {
        Some(now) if now != watched => {
            {
                let mut b = s.borrow_mut();
                let Some(d) = b.docs.iter_mut().find(|d| d.path.as_ref() == Some(&watched)) else {
                    // The tab was closed while the event was in flight; nothing to follow.
                    return;
                };
                d.path = Some(now.clone());
            }
            paint(ui, &mut s.borrow_mut(), false);
            checkpoint(ui, s);
            ui.set_notice(format!("This file moved. It is {} now.", now.display()).into());
        }
        Some(_) => {}
        None => ui.set_notice(
            format!(
                "{} is not there any more, and nothing with the same name and the same contents \
                 was found near it. Your draft is untouched — Save As to give it a file again.",
                watched.display()
            )
            .into(),
        ),
    }
}
fn receive(ui: &TextEditorApp, s: &State) {
    follow_moves(ui, s);
    loop {
        let event = s.borrow().events.try_recv();
        let Ok(event) = event else { break };
        match event {
            Event::Recovery(result, g) => {
                if g == s.borrow().recovery_generation {
                    match result {
                        Ok(()) => ui.set_recovery_status("Draft recovery up to date".into()),
                        Err(e) => {
                            ui.set_recovery_status("Recovery unavailable".into());
                            ui.set_notice(format!("Draft recovery failed: {e}").into());
                        }
                    }
                }
            }
            Event::Open(result) => {
                ui.set_busy(false);
                match result {
                    Ok(d) => {
                        let mut b = s.borrow_mut();
                        if let Some(index) = b.docs.iter().position(|old| old.path == d.path) {
                            b.active = index;
                        } else if b.docs.len() == 1
                            && b.docs[0].path.is_none()
                            && !b.docs[0].dirty()
                        {
                            b.docs[0] = d;
                            b.active = 0;
                        } else if b.docs.len() < MAX_TABS {
                            b.docs.push(d);
                            b.active = b.docs.len() - 1;
                        } else {
                            ui.set_notice(
                                "Eight tabs are open. Close one before opening another file."
                                    .into(),
                            );
                            continue;
                        }
                        ui.set_dialog(0);
                        ui.set_notice("".into());
                        paint(ui, &mut b, true);
                        drop(b);
                        search(ui, s, false);
                    }
                    Err(e) => {
                        ui.set_notice(e.clone().into());
                        ui.set_dialog_error(e.into());
                    }
                }
            }
            Event::Saved(result) => {
                ui.set_busy(false);
                match result {
                    Ok(d) => {
                        let mut b = s.borrow_mut();
                        let active = b.active;
                        let mut d = d;
                        d.undo = std::mem::take(&mut b.docs[active].undo);
                        d.redo = std::mem::take(&mut b.docs[active].redo);
                        b.docs[active] = d;
                        let close = b.save_close;
                        b.save_close = false;
                        ui.set_dialog(0);
                        ui.set_notice("Saved".into());
                        paint(ui, &mut b, false);
                        drop(b);
                        checkpoint(ui, s);
                        if close {
                            finish_close(ui, s);
                        }
                    }
                    Err(e) => {
                        s.borrow_mut().save_close = false;
                        ui.set_notice(e.clone().into());
                        ui.set_dialog_error(e.into());
                    }
                }
            }
        }
    }
}
fn request_close(ui: &TextEditorApp, s: &State, index: usize, quitting: bool) {
    if ui.get_busy() || ui.get_dialog() != 0 {
        return;
    }
    let mut b = s.borrow_mut();
    b.quitting = quitting;
    let index = if quitting {
        b.docs.iter().position(Document::dirty).unwrap_or(0)
    } else {
        index
    };
    if index >= b.docs.len() {
        return;
    }
    b.pending_close = Some(index);
    if b.docs[index].dirty() {
        b.active = index;
        paint(ui, &mut b, true);
        ui.set_close_label(format!("{} has unsaved changes.", b.docs[index].title()).into());
        ui.set_dialog_error("".into());
        ui.set_dialog(3);
    } else {
        drop(b);
        finish_close(ui, s);
    }
}
fn finish_close(ui: &TextEditorApp, s: &State) {
    let mut b = s.borrow_mut();
    let quitting = b.quitting;
    if let Some(index) = b.pending_close.take() {
        if index < b.docs.len() {
            b.docs.remove(index);
            if index < b.active {
                b.active -= 1;
            }
        }
    }
    if b.docs.is_empty() {
        b.docs.push(Document::blank());
    }
    b.active = b.active.min(b.docs.len() - 1);
    ui.set_dialog(0);
    paint(ui, &mut b, true);
    drop(b);
    checkpoint(ui, s);
    if quitting {
        if s.borrow().docs.iter().any(Document::dirty) {
            request_close(ui, s, 0, true);
        } else {
            // Flush an empty recovery snapshot before a deliberate clean exit.
            let b = s.borrow();
            b.recovery_timer.stop();
            let _ = b
                .jobs
                .send(Job::Recovery(b.docs.clone(), b.recovery_generation));
            drop(b);
            let _ = ui.hide();
            let _ = slint::quit_event_loop();
        }
    }
}
fn action(ui: &TextEditorApp, s: &State, id: &str) {
    if ui.get_busy() {
        return;
    }
    if ui.get_dialog() != 0
        && !matches!(
            id,
            "cancel" | "confirm" | "discard" | "save-close" | "save-as"
        )
    {
        return;
    }
    match id {
        "undo" | "redo" => {
            let mut b = s.borrow_mut();
            let active = b.active;
            b.docs[active].undo(id == "redo");
            paint(ui, &mut b, true);
            drop(b);
            search(ui, s, false);
            checkpoint(ui, s);
        }
        "new" => {
            let mut b = s.borrow_mut();
            if b.docs.len() >= MAX_TABS {
                ui.set_notice("Eight tabs are open. Close a tab first.".into());
                return;
            }
            b.docs.push(Document::blank());
            b.active = b.docs.len() - 1;
            ui.set_notice("".into());
            paint(ui, &mut b, true);
        }
        "open" | "save-as" => {
            let b = s.borrow();
            ui.set_dialog_path(if id == "open" {
                "~/".into()
            } else {
                b.docs[b.active]
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "~/untitled.txt".into())
                    .into()
            });
            ui.set_dialog_error("".into());
            ui.set_dialog(if id == "open" { 1 } else { 2 });
        }
        "save" => save(ui, s, None, false),
        "save-close" => {
            s.borrow_mut().save_close = true;
            save(ui, s, None, false);
        }
        "goto" => {
            ui.set_dialog_path(ui.get_cursor_line().to_string().into());
            ui.set_dialog_error("".into());
            ui.set_dialog(4);
        }
        "confirm" => match ui.get_dialog() {
            1 => open(ui, s, expanded(&ui.get_dialog_path())),
            2 => save(ui, s, Some(expanded(&ui.get_dialog_path())), false),
            4 => {
                if let Ok(line) = ui.get_dialog_path().parse::<usize>() {
                    let text = ui.get_content();
                    let offset = if line <= 1 {
                        0
                    } else {
                        text.match_indices('\n')
                            .nth(line - 2)
                            .map(|(i, _)| i + 1)
                            .unwrap_or(text.len())
                    };
                    ui.set_dialog(0);
                    ui.invoke_focus_editor();
                    ui.invoke_select_range(offset as i32, offset as i32);
                } else {
                    ui.set_dialog_error("Enter a whole line number.".into());
                }
            }
            _ => {}
        },
        "cancel" => {
            let mut b = s.borrow_mut();
            b.pending_close = None;
            b.quitting = false;
            b.save_close = false;
            ui.set_dialog(0);
            ui.invoke_focus_editor();
        }
        "discard" => finish_close(ui, s),
        "close" => {
            let i = s.borrow().active;
            request_close(ui, s, i, false);
        }
        "next-tab" | "previous-tab" => {
            let mut b = s.borrow_mut();
            b.active = (b.active
                + if id == "next-tab" {
                    1
                } else {
                    b.docs.len() - 1
                })
                % b.docs.len();
            paint(ui, &mut b, true);
            drop(b);
            search(ui, s, false);
        }
        "find-next" | "find-prev" => {
            let mut b = s.borrow_mut();
            let len = b.matches.len();
            if len == 0 {
                return;
            }
            b.match_index = (b.match_index + if id == "find-next" { 1 } else { len - 1 }) % len;
            let (a, z) = b.matches[b.match_index];
            ui.set_match_index(b.match_index as i32 + 1);
            ui.invoke_select_range(a as i32, z as i32);
        }
        "replace" | "replace-all" => {
            let b = s.borrow();
            let ranges = if id == "replace-all" {
                b.matches.as_slice()
            } else {
                b.matches
                    .get(b.match_index)
                    .map(std::slice::from_ref)
                    .unwrap_or(&[])
            };
            let result = document::replace(&b.docs[b.active].text, ranges, &ui.get_replacement());
            drop(b);
            match result {
                Ok(text) => {
                    ui.set_content(text.clone().into());
                    edit(ui, s, text);
                }
                Err(e) => ui.set_notice(e.into()),
            }
        }
        _ => {}
    }
}
// ── What the mind is shown, and what it is told afterwards ─────────────────
//
// This surface used to be two `for name in [...]` loops. The first built ten actions as
// `Action::new(name, &format!("Editor: {name}"))` with no arguments and answered every one of
// them `{"accepted": true}`; the second built six more, gave each a single argument called
// `path` or `text` with no description — `select_tab` took a "text" that had to be a number —
// and answered `{"accepted": true, "completed": !busy}`. So a mind driving the editor was shown
// "Editor: replace-all" and told, afterwards, that it had been accepted: never which file, never
// how many matches were replaced, never whether anything reached the disk.
//
// Every action is now written out with the sentence a reader who cannot see the screen needs,
// every argument says what goes in it, and every answer is read back out of the document and off
// the disk after the work has settled.

/// A refusal the person at the window sees too.
///
/// Contract point 4: failure is said twice — to the caller, and in the app's `notice`, which is
/// `describe.notice` and the amber line above the editor.
fn refuse(ui: &TextEditorApp, message: impl Into<String>) -> String {
    let message = message.into();
    ui.set_notice(message.clone().into());
    message
}

/// One action, with the one sentence a reader who cannot see the screen needs.
///
/// The guard is the point: a description that is only the action's name, or too short to say
/// what it does and to which document, stops the app before `serve()` — and inside the tests,
/// which build this same list. `Action::new` takes any `&str`; `yantrik-app-runtime` is shared
/// by fourteen apps and is not this change's to alter, so the check lives here.
fn act(name: &'static str, sentence: &'static str) -> Action {
    assert!(
        sentence.len() >= 20 && !sentence.starts_with("Editor:"),
        "editor action `{name}` was given a placeholder description: {sentence:?}"
    );
    Action::new(name, sentence)
}

/// One argument, with the format it takes and what leaving it out means. Required by default.
fn arg(name: &'static str, sentence: &'static str) -> Param {
    assert!(
        sentence.len() >= 15,
        "editor argument `{name}` was given no usable description: {sentence:?}"
    );
    Param::text(name).describe(sentence)
}

/// A required text argument that has to say something, or a refusal that names it.
fn needed(
    ui: &TextEditorApp,
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

/// Wait, on the UI thread, for the file work this action started.
///
/// The editor opens and saves on one worker: the handler sends a job, sets `busy`, and `receive`
/// applies the answer when the worker wakes the event loop. An action handler runs on the UI
/// thread too, so returning as soon as the job was sent is why `save` could only ever answer
/// "accepted" — the caller was told nothing about the file it had asked to write. Draining the
/// channel here is what lets the answer report the bytes on disk. The 800 ms budget leaves the
/// runtime's 3 s `UI_ROUNDTRIP` intact even for a call that waits twice, and a save of the 1 MiB
/// this editor allows is still sub-millisecond; an unsettled call answers `modified: true`
/// rather than guess.
fn settle(ui: &TextEditorApp, s: &State) -> bool {
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

/// Refuse while a dialog is waiting for an answer, naming the answer it wants.
///
/// The window ignores almost every action while a dialog is up. A caller that was told
/// `accepted: true` for an action the window had already dropped on the floor is the fault this
/// closes: the refusal says which dialog is open and which action answers it.
fn no_dialog(ui: &TextEditorApp, name: &str) -> Result<(), String> {
    match ui.get_dialog() {
        0 => Ok(()),
        3 => Err(refuse(
            ui,
            format!(
                "`{name}` cannot run while the editor is asking about unsaved changes ({}). \
                 Answer it first: `save` writes the tab and closes it, `discard` throws the \
                 changes away, `cancel` leaves it open.",
                ui.get_close_label()
            ),
        )),
        other => Err(refuse(
            ui,
            format!(
                "`{name}` cannot run while the {} dialog is open; `cancel` closes it.",
                match other {
                    1 => "Open file",
                    2 => "Save As",
                    _ => "Go to line",
                }
            ),
        )),
    }
}

/// What the active tab now is, read back out of the document and off the disk.
///
/// Every action that touches a document answers with this, so "it worked" is never the app's
/// opinion of its own handler: `matches_disk` is the file compared against the text in the tab.
fn document_now(ui: &TextEditorApp, s: &State) -> serde_json::Value {
    let b = s.borrow();
    let d = &b.docs[b.active];
    let disk = d.path.as_deref().and_then(|p| document::read(p).ok());
    serde_json::json!({
        "title": d.title(),
        "path": d.path.as_ref().map(|p| p.display().to_string()),
        "tab": b.active,
        "tabs": b.docs.len(),
        "lines": d.text.bytes().filter(|c| *c == b'\n').count() + 1,
        "characters": d.text.chars().count(),
        "bytes": d.text.len(),
        "modified": d.dirty(),
        "on_disk": disk.is_some(),
        "matches_disk": disk.as_deref() == Some(d.text.as_str()),
        "language": document::language(d.path.as_deref()),
        "notice": ui.get_notice().to_string(),
    })
}

/// The one line a mind reads first.
///
/// This used to be `Text Editor — <filename>` and nothing else: not whether the file had unsaved
/// changes, not how much was in it, not how many tabs were open, and not that a dialog was
/// blocking every action the caller was about to try.
fn view(ui: &TextEditorApp, s: &State) -> View {
    let b = s.borrow();
    let d = &b.docs[b.active];
    let lines = d.text.bytes().filter(|c| *c == b'\n').count() + 1;
    let mut summary = format!(
        "Text Editor — {}{}, {} line{}, {}",
        d.title(),
        if d.path.is_none() { " (no file yet)" } else { "" },
        lines,
        if lines == 1 { "" } else { "s" },
        if d.dirty() { "unsaved" } else { "saved" }
    );
    if b.docs.len() > 1 {
        summary.push_str(&format!(" · tab {} of {}", b.active + 1, b.docs.len()));
    }
    let unsaved = b.docs.iter().filter(|d| d.dirty()).count();
    if unsaved > 1 {
        summary.push_str(&format!(" ({unsaved} tabs unsaved)"));
    }
    match ui.get_dialog() {
        0 => {}
        3 => summary.push_str(" · asking about unsaved changes: save, discard or cancel"),
        1 => summary.push_str(" · Open file dialog is up"),
        2 => summary.push_str(" · Save As dialog is up"),
        _ => summary.push_str(" · Go to line dialog is up"),
    }
    if ui.get_busy() {
        summary.push_str(" · working");
    }
    let notice = ui.get_notice().to_string();
    if !notice.is_empty() {
        summary.push_str(&format!(" · {notice}"));
    }
    View::new(summary)
        .with("path", serde_json::json!(d.path))
        .with("title", d.title())
        .with("modified", d.dirty())
        .with("lines", lines as i64)
        .with("characters", d.text.chars().count() as i64)
        .with("content", d.text.chars().take(4000).collect::<String>())
        .with("bytes", d.text.len())
        .with("language", document::language(d.path.as_deref()))
        .with(
            "tabs",
            b.docs
                .iter()
                .map(|d| serde_json::json!({"name": d.title(), "path": d.path, "modified": d.dirty()}))
                .collect::<Vec<_>>(),
        )
        .with("active_tab", b.active)
        .with("busy", ui.get_busy())
        .with("notice", notice)
        .with("dialog", ui.get_dialog())
        .with("dialog_is", ui.get_close_label().to_string())
        .with("recovery", ui.get_recovery_status().to_string())
        .with("cursor_line", ui.get_cursor_line())
        .with("cursor_column", ui.get_cursor_column())
        .with("find_query", ui.get_query().to_string())
        .with("find_count", ui.get_match_count())
        .with("replacement", ui.get_replacement().to_string())
}

/// One published action: what it says it does, and the code that does it.
type Handler = Box<dyn Fn(&serde_json::Value) -> Result<serde_json::Value, String>>;

/// Everything this window offers a mind.
///
/// Built as a list rather than pushed straight into `App` so the tests can read exactly what a
/// mind is shown and run a handler without a socket or an event loop.
fn surface(ui: &TextEditorApp, s: &State) -> Vec<(Action, Handler)> {
    let window = {
        let weak = ui.as_weak();
        move || weak.upgrade().ok_or_else(|| "The Editor window is gone.".to_string())
    };
    let mut out: Vec<(Action, Handler)> = Vec::new();
    let mut add = |spec: Action,
                   run: fn(
        &TextEditorApp,
        &State,
        &serde_json::Value,
    ) -> Result<serde_json::Value, String>| {
        let window = window.clone();
        let state = s.clone();
        let name = spec.name.clone();
        out.push((
            spec,
            Box::new(move |args: &serde_json::Value| {
                let ui = window()?;
                if ui.get_busy() {
                    return Err(format!(
                        "`{name}` cannot run while the editor is reading or writing a file; \
                         read `describe` again in a moment."
                    ));
                }
                run(&ui, &state, args)
            }) as Handler,
        ));
    };

    add(
        // Standard, with or without `text`: a new tab replaces nothing. With `save_as` it is the
        // way to write a file's text without asking anyone — the route a mind running unattended
        // needs, since `set_content` is sensitive and waits for a person (#253).
        act(
            "new",
            "Open a new tab and make it the active one, empty or holding the text given. Nothing \
             is written to disk until `save_as` gives it a path; up to eight tabs can be open.",
        )
        .arg(
            arg(
                "text",
                "What the new tab holds, up to 1 MiB and 20,000 lines of UTF-8 with no control \
                 characters. Leave it out for an empty tab.",
            )
            .optional(),
        ),
        |ui, s, args| {
            no_dialog(ui, "new")?;
            // Checked before the tab opens, so text that is refused leaves no empty tab behind.
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            document::validate(&text).map_err(|e| refuse(ui, e))?;
            let before = s.borrow().docs.len();
            action(ui, s, "new");
            if s.borrow().docs.len() == before {
                return Err(refuse(
                    ui,
                    "Eight tabs are already open; close one before opening another.",
                ));
            }
            if !text.is_empty() {
                ui.set_content(text.clone().into());
                edit(ui, s, text.clone());
                if s.borrow().docs[s.borrow().active].text != text {
                    return Err(refuse(ui, "The new tab is open but empty; the text was rejected."));
                }
            }
            Ok(document_now(ui, s))
        },
    );

    add(
        act(
            "open",
            "Open the UTF-8 text file at this path in a tab and make it active; a file already \
             open is brought forward instead of opened twice. The file is not changed.",
        )
        .arg(arg(
            "path",
            "Absolute path to the file, or one starting `~/`. Up to 1 MiB of UTF-8 text; a \
             binary or larger file is refused and nothing is opened.",
        )),
        |ui, s, args| {
            no_dialog(ui, "open")?;
            let path = needed(ui, args, "open", "path", "An absolute path to a text file.")?;
            let full = expanded(path.trim());
            let before = s.borrow().docs[s.borrow().active].path.clone();
            open(ui, s, full.clone());
            settle(ui, s);
            let answer = document_now(ui, s);
            if answer["path"] != serde_json::json!(full.display().to_string())
                && s.borrow().docs[s.borrow().active].path == before
            {
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        format!("{} was not opened.", full.display())
                    } else {
                        why
                    },
                ));
            }
            Ok(answer)
        },
    );

    add(
        act(
            "save",
            "Write the active tab back to the file it was opened from and check the bytes on \
             disk. A tab with no file is refused — give `save_as` a path. When the editor is \
             asking about unsaved changes, this is the answer that writes the tab and closes it.",
        ),
        |ui, s, _| {
            let answering = ui.get_dialog() == 3;
            if !answering {
                no_dialog(ui, "save")?;
            }
            if s.borrow().docs[s.borrow().active].path.is_none() {
                return Err(refuse(
                    ui,
                    "This tab has never been written anywhere; `save_as` with a path decides \
                     where it goes.",
                ));
            }
            let path = s.borrow().docs[s.borrow().active].path.clone();
            action(ui, s, if answering { "save-close" } else { "save" });
            settle(ui, s);
            // Read back through the tab that holds that path — a save that closed its tab has
            // moved the active one, and answering about whatever is now in front would be a
            // different document.
            let b = s.borrow();
            let d = b.docs.iter().find(|d| d.path == path);
            let wrote = d.map(|d| !d.dirty()).unwrap_or(true);
            let on_disk = path.as_deref().and_then(|p| document::read(p).ok());
            drop(b);
            if !wrote || on_disk.is_none() {
                let why = ui.get_notice().to_string();
                return Err(refuse(
                    ui,
                    if why.is_empty() {
                        "The file was not written; the draft is still open.".to_string()
                    } else {
                        why
                    },
                ));
            }
            Ok(serde_json::json!({
                "path": path.as_ref().map(|p| p.display().to_string()),
                "bytes": on_disk.as_deref().map(str::len),
                "lines": on_disk.as_deref().map(|t| t.bytes().filter(|c| *c == b'\n').count() + 1),
                "saved": true,
                "closed_the_tab": answering,
                "tabs": s.borrow().docs.len(),
                "now": document_now(ui, s),
            }))
        },
    );

    add(
        // Standard, like `save`: writing a file the caller named is this app's one job, and an
        // existing path is refused rather than silently replaced — unless the caller says
        // `overwrite=true`, having read the refusal that names it (#86).
        act(
            "save_as",
            "Write the active tab to this path and keep the tab on it from now on. A file \
             already at that path is refused unless `overwrite` says to replace it.",
        )
        .arg(arg(
            "path",
            "Absolute path of the file to write, or one starting `~/`. Its folder must exist; a \
             file already at the path needs `overwrite=true`.",
        ))
        .arg(
            Param::flag("overwrite")
                .optional()
                .describe(
                    "Replace a file that is already at that path. Leave it out and an existing \
                     file is refused rather than overwritten.",
                ),
        ),
        |ui, s, args| {
            if ui.get_dialog() != 0 && ui.get_dialog() != 2 && ui.get_dialog() != 3 {
                no_dialog(ui, "save_as")?;
            }
            let path = needed(ui, args, "save_as", "path", "An absolute path to write to.")?;
            let full = expanded(path.trim());
            let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);
            save(ui, s, Some(full.clone()), overwrite);
            settle(ui, s);
            let answer = document_now(ui, s);
            if answer["path"] != serde_json::json!(full.display().to_string())
                || answer["matches_disk"] != true
            {
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
            Ok(answer)
        },
    );

    add(
        act(
            "show",
            "Bring the Editor window to the front. Nothing in any tab is read or changed.",
        ),
        |ui, s, _| {
            let _ = ui.show();
            Ok(serde_json::json!({ "shown": true, "now": document_now(ui, s) }))
        },
    );

    add(
        act(
            "close",
            "Close the active tab. A tab with unsaved changes is not closed: the window asks, \
             and `save`, `discard` or `cancel` answers it. The last tab closed leaves an empty one.",
        ),
        |ui, s, _| {
            no_dialog(ui, "close")?;
            let (before, title) = {
                let b = s.borrow();
                (b.docs.len(), b.docs[b.active].title())
            };
            action(ui, s, "close");
            let asking = ui.get_dialog() == 3;
            Ok(serde_json::json!({
                "closed": !asking && s.borrow().docs.len() < before,
                "was": title,
                "awaiting_answer": asking,
                "asking": if asking { ui.get_close_label().to_string() } else { String::new() },
                "tabs": s.borrow().docs.len(),
                "now": document_now(ui, s),
            }))
        },
    );

    add(
        act(
            "cancel",
            "Dismiss whichever dialog is open and leave every tab exactly as it was. Nothing is \
             closed, saved or discarded.",
        ),
        |ui, s, _| {
            let was = ui.get_dialog();
            if was == 0 {
                return Err(refuse(ui, "No dialog is open, so there is nothing to cancel."));
            }
            action(ui, s, "cancel");
            Ok(serde_json::json!({
                "dialog_closed": ui.get_dialog() == 0,
                "was_asking": was,
                "now": document_now(ui, s),
            }))
        },
    );

    add(
        // Sensitive: this is the action that throws away work. The tab's unsaved text is gone
        // when it returns — the draft recovery file is rewritten without it — and nothing on
        // this machine keeps a copy.
        act(
            "discard",
            "Throw away the unsaved changes in the tab the window is asking about and close it. \
             The text that was never saved is gone and cannot be recovered.",
        )
        .risk("sensitive"),
        |ui, s, _| {
            if ui.get_dialog() != 3 {
                return Err(refuse(
                    ui,
                    "Nothing is waiting to be discarded; `close` asks first, and only a tab with \
                     unsaved changes is ever asked about.",
                ));
            }
            let (before, title) = {
                let b = s.borrow();
                (b.docs.len(), b.docs[b.active].title())
            };
            action(ui, s, "discard");
            Ok(serde_json::json!({
                "discarded": title,
                "closed": s.borrow().docs.len() < before || before == 1,
                "tabs": s.borrow().docs.len(),
                "now": document_now(ui, s),
            }))
        },
    );

    add(
        act(
            "find",
            "Search the active tab for this text, open the find bar, and select the first match. \
             It reads the document and changes nothing in it.",
        )
        .arg(arg(
            "text",
            "The text to look for, matched literally. Case is ignored unless the find bar's \
             match-case box is ticked; an empty query clears the matches.",
        )),
        |ui, s, args| {
            no_dialog(ui, "find")?;
            let query = needed(ui, args, "find", "text", "The text to look for.")?;
            ui.set_query(query.clone().into());
            ui.set_show_find(true);
            search(ui, s, true);
            Ok(serde_json::json!({
                "query": query,
                "matches": ui.get_match_count(),
                "at_match": ui.get_match_index(),
                "in": s.borrow().docs[s.borrow().active].title(),
            }))
        },
    );

    add(
        act(
            "find-next",
            "Move the selection to the next match of the current `find` query, wrapping round at \
             the end of the document. Nothing is changed.",
        ),
        |ui, s, _| step_match(ui, s, "find-next"),
    );

    add(
        act(
            "find-prev",
            "Move the selection to the previous match of the current `find` query, wrapping \
             round at the start of the document. Nothing is changed.",
        ),
        |ui, s, _| step_match(ui, s, "find-prev"),
    );

    add(
        act(
            "replace_text",
            "Set the text that `replace` and `replace-all` will put in place of a match. By \
             itself it changes nothing in the document.",
        )
        .arg(arg(
            "text",
            "What each match becomes. An empty string is allowed and deletes the match instead.",
        )),
        |ui, s, args| {
            let with = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| refuse(ui, "`replace_text` needs `text`: what each match becomes."))?;
            ui.set_replacement(with.into());
            Ok(serde_json::json!({
                "replacement": with,
                "query": ui.get_query().to_string(),
                "matches_waiting": ui.get_match_count(),
                "in": s.borrow().docs[s.borrow().active].title(),
            }))
        },
    );

    add(
        act(
            "replace",
            "Replace the one match the selection is on with the `replace_text` text. The tab is \
             changed in the window; nothing is written until `save`.",
        ),
        |ui, s, _| replace_matches(ui, s, "replace"),
    );

    add(
        act(
            "replace-all",
            "Replace every match of the current `find` query in the active tab at once. The tab \
             is changed in the window; nothing is written until `save`.",
        ),
        |ui, s, _| replace_matches(ui, s, "replace-all"),
    );

    add(
        // Sensitive: it overwrites the whole tab, unsaved paragraphs included, in one call.
        // What it replaces is on disk only if it had been saved.
        act(
            "set_content",
            "Replace everything in the active tab with this text. The file on disk is untouched \
             until `save`; whatever was in the tab and unsaved is gone. To start a document with \
             text, `new` with `text` does it without replacing anything.",
        )
        .risk("sensitive")
        .arg(arg(
            "text",
            "The tab's entire new text, up to 1 MiB and 20,000 lines of UTF-8 with no control \
             characters. An empty string empties the tab.",
        )),
        |ui, s, args| {
            no_dialog(ui, "set_content")?;
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| refuse(ui, "`set_content` needs `text`: the tab's entire new text."))?
                .to_string();
            document::validate(&text).map_err(|e| refuse(ui, e))?;
            ui.set_content(text.clone().into());
            edit(ui, s, text.clone());
            let answer = document_now(ui, s);
            if s.borrow().docs[s.borrow().active].text != text {
                return Err(refuse(ui, "The tab was not changed; the text was rejected."));
            }
            Ok(answer)
        },
    );

    add(
        // Standard, unlike `set_content`: it adds to the tab and takes nothing away, and `undo`
        // takes it back off. With `new` it is how a mind under a `standard` ceiling writes a
        // draft here — the Writer role's way in since the shell's own editor, and its
        // `editor_append`, were removed (#253).
        act(
            "append",
            "Add this text to the end of the active tab; on a new, empty tab it is the whole \
             text. Nothing already in the tab is changed, and the file on disk is untouched \
             until `save`.",
        )
        .arg(arg(
            "text",
            "The text to add, exactly as given: start it with a newline to begin a new line. The \
             whole tab has to stay within 1 MiB and 20,000 lines.",
        )),
        |ui, s, args| {
            no_dialog(ui, "append")?;
            let add = args
                .get("text")
                .and_then(|v| v.as_str())
                .filter(|t| !t.is_empty())
                .ok_or_else(|| refuse(ui, "`append` needs `text`: what to add to the end of the tab."))?;
            let text = format!("{}{add}", s.borrow().docs[s.borrow().active].text);
            document::validate(&text).map_err(|e| refuse(ui, e))?;
            ui.set_content(text.clone().into());
            edit(ui, s, text.clone());
            if s.borrow().docs[s.borrow().active].text != text {
                return Err(refuse(ui, "The tab was not changed; the text was rejected."));
            }
            Ok(document_now(ui, s))
        },
    );

    add(
        // Published because `set_content` and `replace-all` rewrite a whole tab in one call and
        // the person at the keyboard had Ctrl+Z while a mind had nothing: a caller that could
        // destroy a draft could not put it back.
        act(
            "undo",
            "Take back the last change to the active tab, including a whole-tab `set_content` or \
             `replace-all`. Up to 64 steps are kept per tab and none of it touches the file.",
        ),
        |ui, s, _| step_history(ui, s, "undo"),
    );

    add(
        act(
            "redo",
            "Put back the change `undo` took off the active tab. Editing the tab in any other \
             way discards what redo was holding.",
        ),
        |ui, s, _| step_history(ui, s, "redo"),
    );

    add(
        act(
            "select_tab",
            "Make one of the open tabs the active one, by its position in `describe.tabs`. \
             Nothing is saved, closed or changed.",
        )
        .arg(
            Param::number("index")
                .describe(
                    "Which tab, counting from 0 in the order `describe.tabs` lists them; \
                     `describe.active_tab` is the one in front now.",
                ),
        ),
        |ui, s, args| {
            no_dialog(ui, "select_tab")?;
            let index = args
                .get("index")
                .and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()))
                .ok_or_else(|| {
                    refuse(ui, "`select_tab` needs `index`: which tab, counting from 0.")
                })?;
            let open = s.borrow().docs.len();
            if index < 0 || index as usize >= open {
                return Err(refuse(
                    ui,
                    format!("There is no tab {index}; {open} are open, numbered 0 to {}.", open - 1),
                ));
            }
            ui.invoke_select_tab(index as i32);
            if s.borrow().active != index as usize {
                return Err(refuse(ui, format!("Tab {index} did not come forward.")));
            }
            Ok(document_now(ui, s))
        },
    );

    out
}

/// Step the active tab's history, and refuse rather than report a move that did not happen.
fn step_history(ui: &TextEditorApp, s: &State, id: &str) -> Result<serde_json::Value, String> {
    no_dialog(ui, id)?;
    let before = s.borrow().docs[s.borrow().active].text.clone();
    action(ui, s, id);
    if s.borrow().docs[s.borrow().active].text == before {
        return Err(refuse(
            ui,
            format!("There is nothing left to {id} in this tab; it is unchanged."),
        ));
    }
    Ok(document_now(ui, s))
}

/// Walk to the next or previous match, and say where that left the selection.
fn step_match(
    ui: &TextEditorApp,
    s: &State,
    id: &str,
) -> Result<serde_json::Value, String> {
    no_dialog(ui, id)?;
    if ui.get_match_count() == 0 {
        return Err(refuse(
            ui,
            format!(
                "Nothing matches {:?} in this tab, so there is no match to step to. `find` sets \
                 the query.",
                ui.get_query()
            ),
        ));
    }
    action(ui, s, id);
    Ok(serde_json::json!({
        "query": ui.get_query().to_string(),
        "at_match": ui.get_match_index(),
        "matches": ui.get_match_count(),
        "in": s.borrow().docs[s.borrow().active].title(),
    }))
}

/// Replace one match or all of them, and count what actually changed.
fn replace_matches(
    ui: &TextEditorApp,
    s: &State,
    id: &str,
) -> Result<serde_json::Value, String> {
    no_dialog(ui, id)?;
    let intended = if id == "replace-all" { ui.get_match_count() } else { 1.min(ui.get_match_count()) };
    if intended == 0 {
        return Err(refuse(
            ui,
            format!(
                "Nothing matches {:?} in this tab, so there is nothing to replace. `find` sets \
                 the query and `replace_text` sets what it becomes.",
                ui.get_query()
            ),
        ));
    }
    let before = s.borrow().docs[s.borrow().active].text.clone();
    action(ui, s, id);
    let after = s.borrow().docs[s.borrow().active].text.clone();
    if after == before {
        let why = ui.get_notice().to_string();
        return Err(refuse(
            ui,
            if why.is_empty() { "Nothing was replaced.".to_string() } else { why },
        ));
    }
    let mut answer = document_now(ui, s);
    answer["replaced"] = serde_json::json!(intended);
    answer["with"] = serde_json::json!(ui.get_replacement().to_string());
    answer["matches_left"] = serde_json::json!(ui.get_match_count());
    Ok(answer)
}

fn publish_control(ui: &TextEditorApp, s: State) {
    let weak = ui.as_weak();
    let state = s.clone();
    let mut app = App::new("editor").describe(move || match weak.upgrade() {
        Some(ui) => view(&ui, &state),
        None => View::new("Text Editor — the window is closed"),
    });
    for (spec, run) in surface(ui, &s) {
        app = app.action(spec, run);
    }
    app.serve();
}

#[cfg(test)]
mod tests;
