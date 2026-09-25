//! Yantrik Document Editor (yDoc) — standalone app binary.
//!
//! It could rewrite your document and it could not save it.
//!
//! `on_doc_save` read `doc-file-path`, found it empty, logged "No file path set" and returned.
//! `doc-file-path` is an `in` property — settable only from Rust — and no line in this app ever
//! called `set_doc_file_path`. `on_doc_open` was a log line and there was no command line, so
//! there was no path anywhere in the process: the guard could never be false, Save always took
//! its early return in silence, and the `std::fs::write` beneath it, with its error handling, was
//! dead code that had never run once.
//!
//! What made that urgent rather than merely wrong is what did work. `on_doc_ai_submit` really
//! asks the companion, really shows a proposal saying "Replaces the document with N words,
//! unsaved", and `on_proposal_applied` really replaces the text. The app could take your
//! document away and give you a different one, and there was no way to keep the result.
//!
//! Now: the document is Markdown, it has a path, Save writes it through a temporary and refuses
//! to overwrite a file that changed underneath it, an unsaved draft survives a crash, and a mind
//! can open, write, search and save one over the control surface. The buttons that were not
//! backed by any of that are off the screen, each with the reason at the site it left.

mod document;

use document::{Document, Format, Saved};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};
use yantrik_app_runtime::prelude::*;
// The watch on the folder the document lives in, and the decisions behind following a move.
// Shared with the Text Editor (#86); it was this app's own `follow.rs` until then.
use yantrik_file_follow as follow;

slint::include_modules!();

/// The app's name on the bus, and the id the launcher routes `documents` to
/// (`crates/yantrik-ui/src/wire/dock.rs`: `Launch::Program { id: "documents", bin:
/// "yantrik-document-editor" }`). The window's own `AppIdentity.id` is the same word. The socket
/// is `app-documents.sock`.
const APP_ID: &str = "documents";

/// How much of the document `describe` carries.
///
/// A glance, not a transcript — docs/app-control.md. Notes settled on the same 4000 characters,
/// and the state says how much of the document that was, so a caller reading an excerpt knows it
/// is one.
const EXCERPT_CHARS: usize = 4000;

// What the file prompt is asking for. Mirrors the numbers in document_editor.slint.
const DIALOG_NONE: i32 = 0;
const DIALOG_OPEN: i32 = 1;
const DIALOG_SAVE_AS: i32 = 2;
const DIALOG_EXPORT_MD: i32 = 3;
const DIALOG_EXPORT_HTML: i32 = 4;
const DIALOG_UNSAVED: i32 = 5;

/// What the unsaved-changes prompt is standing in front of.
#[derive(Clone, PartialEq, Eq)]
enum Intent {
    None,
    Close,
    New,
    /// Open this file, once the draft on screen has been saved or given up on purpose. The path
    /// travels with the intent so that answering the prompt does not mean retyping it.
    Open(PathBuf),
}

/// Everything the app holds that the window does not.
struct Editor {
    doc: Document,
    matches: Vec<(usize, usize)>,
    match_index: usize,
    /// The rewritten document, between the companion answering and the person pressing Replace.
    ///
    /// Not a UI property because nothing draws it: the proposal card shows the text, and this is
    /// what gets written if the card is accepted. Behind an `Arc<Mutex<_>>` rather than in this
    /// struct's own storage because the worker thread that asks the companion is what fills it,
    /// and the rest of this struct never leaves the UI thread.
    pending: std::sync::Arc<std::sync::Mutex<String>>,
    intent: Intent,
    recovery: PathBuf,
    /// A draft from a previous session was found AND a file was named on the command line. The
    /// file wins the window, and the draft is left exactly where it is rather than being written
    /// over by the first keystroke — it is offered again on the next launch with no argument.
    hold_recovery: bool,
    /// What startup found, said once the window has finished opening whatever it was given.
    startup_notice: String,
    recovery_timer: slint::Timer,
    rail_timer: slint::Timer,
    /// The watch on the folder the document lives in, re-pointed by `paint` as the document
    /// moves. See `follow.rs` for why a document editor watches a folder at all.
    follow: follow::Watch,
    /// What that watch has seen, waiting for the UI thread to come and read it.
    moves: mpsc::Receiver<follow::Move>,
}

type State = Rc<RefCell<Editor>>;

fn expanded(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
    } else {
        PathBuf::from(value)
    }
}

fn main() {
    init_tracing("yantrik-document-editor");

    // A VM forcing software OpenGL has no GPU to accelerate femtovg; render on the CPU instead
    // and leave an explicit renderer choice alone. The same rule the editor and the shell follow.
    if std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref() == Ok("1")
        && matches!(std::env::var("SLINT_BACKEND").as_deref(), Ok("winit") | Err(_))
    {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }

    let argv = std::env::args_os().nth(1).map(PathBuf::from);

    // One window per app. A second launch hands its file to the running one rather than opening
    // a second editor over the same document — two windows on one file is how an edit is lost.
    let Some(_instance) = instance::claim("document-editor") else {
        let request = match &argv {
            Some(p) => serde_json::json!({"action": "open", "args": {"path": p}}),
            None => serde_json::json!({"action": "show", "args": {}}),
        };
        let client = SyncRpcClient::for_service(&control::service_id_for(APP_ID))
            .with_timeout(Duration::from_secs(3));
        for _ in 0..20 {
            if let Ok(reply) = client.call("app.act", request.clone()) {
                if reply["accepted"] != true {
                    eprintln!("yDoc declined the request: {reply}");
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("yDoc is already starting; try opening the document again.");
        return;
    };

    let app = DocumentEditorApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    let state = wire(&app, document::recovery_path(), argv.is_some());
    if let Some(path) = argv {
        let _ = report(&app, open_path(&app, &state, &path, false));
    }
    // Last, because a successful open clears the notice and a draft held aside is the one thing
    // a person opening a file still needs to be told.
    let startup = state.borrow().startup_notice.clone();
    if !startup.is_empty() {
        app.set_notice(startup.into());
    }

    publish_control(&app, state.clone());
    run_until_closed(&app, "yantrik-document-editor");

    // The last checkpoint, once the event loop has stopped and nothing else can be typed.
    let b = state.borrow();
    b.recovery_timer.stop();
    if !b.hold_recovery {
        if let Err(e) = document::checkpoint(&b.recovery, &b.doc) {
            tracing::warn!(error = %e, "final draft checkpoint failed");
        }
    }
}

// ── The agent rail ──────────────────────────────────────────────────────────

/// Fill the agent rail from the document on screen.
///
/// Its own headings are the context: they are what the document IS, the app already computes
/// them for the outline panel, and a heading list is the one thing here that is true without
/// asking anybody. Memory is added when the companion is reachable, filtered by the shared
/// relevance floor -- see companion::recall_relevant for why that floor exists.
fn refresh_agent_rail(ui: &DocumentEditorApp, text: &str, title: &str) {
    let mut context: Vec<AgentContextItem> = Vec::new();
    for h in document::outline(text).into_iter().take(6) {
        context.push(AgentContextItem {
            id: format!("heading:{}", h.offset).into(),
            label: h.title.into(),
            detail: format!("H{}", h.level).into(),
            source: "outline".into(),
        });
    }
    ui.set_agent_context(ModelRc::new(VecModel::from(context.clone())));

    let has_text = document::word_count(text) > 0;
    let reach = companion::reach();
    let mut next: Vec<AgentSuggestion> = Vec::new();
    if has_text && reach == companion::Reach::Ready {
        next.push(AgentSuggestion {
            id: "summarize".into(),
            label: "Summarise it".into(),
            detail: "five bullets, from what it says".into(),
            icon: "template".into(),
            running: ui.get_proposal_working(),
            proposes: true,
        });
        next.push(AgentSuggestion {
            id: "tighten".into(),
            label: "Make it shorter".into(),
            detail: "clearer, keeping every fact".into(),
            icon: "spark".into(),
            running: ui.get_proposal_working(),
            proposes: true,
        });
    }
    ui.set_agent_suggestions(ModelRc::new(VecModel::from(next)));

    ui.set_agent_unavailable(match reach.hint() {
        Some(hint) if has_text => hint.into(),
        _ => SharedString::new(),
    });

    // Memory is a search, not a generation: it answers from the shell even when no model does,
    // so only a missing shell takes it away.
    if reach != companion::Reach::NoShell && has_text {
        let query = title.to_string();
        if query.trim().is_empty() {
            return;
        }
        let back = ui.as_weak();
        std::thread::spawn(move || {
            let found = companion::recall_relevant(&query, companion::RELEVANCE_FLOOR, 3);
            if found.is_empty() {
                return;
            }
            let _ = back.upgrade_in_event_loop(move |ui| {
                let mut rows = context;
                for m in found {
                    let line = m.text.lines().next().unwrap_or("").trim().to_string();
                    rows.push(AgentContextItem {
                        id: format!("memory:{}", m.rid).into(),
                        label: line.into(),
                        detail: format!("{}% match", (m.score * 100.0).round() as i64).into(),
                        source: "memory".into(),
                    });
                }
                ui.set_agent_context(ModelRc::new(VecModel::from(rows)));
            });
        });
    }
}

// ── Painting ────────────────────────────────────────────────────────────────

/// Put the document on screen.
///
/// `content` is false while the person is typing: writing the text back into the `TextInput` it
/// just came out of would move the caret to the start on every keystroke. Everything else — the
/// title, the outline, the counts, the path, whether it is saved — is derived from the document
/// each time, so no two of them can disagree.
fn paint(ui: &DocumentEditorApp, state: &State, content: bool) {
    let b = state.borrow();
    let d = &b.doc;
    let text = d.text.clone();
    let title = d.title();
    if content {
        ui.set_doc_content(text.clone().into());
    }
    ui.set_doc_title(title.clone().into());
    ui.set_doc_file_path(
        d.path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    ui.set_doc_is_modified(d.dirty());
    ui.set_doc_word_count(document::word_count(&text) as i32);
    ui.set_doc_char_count(document::char_count(&text) as i32);
    ui.set_doc_headings(ModelRc::new(VecModel::from(
        document::outline(&text)
            .into_iter()
            .map(|h| DocHeadingEntry {
                title: h.title.into(),
                level: h.level,
                block_index: h.offset as i32,
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_doc_save_status(
        if d.recovered {
            "Recovered draft"
        } else if d.dirty() {
            "Unsaved"
        } else if d.path.is_some() {
            "Saved"
        } else {
            "New document"
        }
        .into(),
    );
    ui.set_doc_find_count(b.matches.len() as i32);
    drop(b);
    follow_file(state);
    refresh_rail_soon(ui, state);
}

/// Point the folder watch at wherever the document lives now.
///
/// Called from `paint` rather than from each of the places that change the path, because `paint`
/// is the one function every one of them already goes through — the same reason the title, the
/// outline and the counts are derived there. Re-pointing at the folder already being watched
/// costs a lock and a comparison.
fn follow_file(state: &State) {
    let mut b = state.borrow_mut();
    let path = b.doc.path.clone();
    b.follow.point_at(path.as_deref());
}

/// One thing the folder watch saw, or `None` when there is nothing waiting.
///
/// A function of its own so that the borrow it takes ends before the caller handles what it
/// returns; draining the channel inside a `while let` over `state.borrow()` would hold the
/// document borrowed while the handler tries to change it.
fn next_move(state: &State) -> Option<follow::Move> {
    state.borrow().moves.try_recv().ok()
}

/// The folder the open document lives in has reported a move. Follow it, if it was ours.
///
/// This is what closes the loop the issue describes: a document moved in Files goes on being the
/// same document, saved by Save, named correctly in the window and in `describe`. Only the path
/// changes — the text, the baseline and the stamp are all untouched, because a rename moves the
/// same bytes and the same inode and a save's conflict check is still asking about the right
/// file afterwards.
fn file_moved(ui: &DocumentEditorApp, state: &State) {
    // Everything waiting, read as one thing. A rename arrives as two events — "it left", then
    // "and here is where it went" — and acting on the first would send the app hunting through a
    // directory for a file the second event is about to name.
    let mut waiting = Vec::new();
    while let Some(moved) = next_move(state) {
        waiting.push(moved);
    }
    let Some(moved) = follow::latest(waiting) else { return };

    let open_file = {
        let b = state.borrow();
        b.doc.path.clone().map(|p| (p, b.doc.baseline.clone()))
    };
    let Some((current, baseline)) = open_file else { return };
    let now = match moved {
        follow::Move::To(to) => Some(to),
        // The event said only that the file left. Where it went is a look at the disk.
        follow::Move::Away => document::moved_to(&current, &baseline),
    };
    match now {
        Some(now) if now != current => {
            {
                let stamp = document::stamp_of(&now);
                let mut b = state.borrow_mut();
                b.doc.path = Some(now.clone());
                if stamp.is_some() {
                    b.doc.stamp = stamp;
                }
            }
            paint(ui, state, false);
            checkpoint(ui, state);
            ui.set_notice(format!("This document moved. It is {} now.", now.display()).into());
        }
        Some(_) => {}
        None => ui.set_notice(
            format!(
                "{} is not there any more, and nothing with the same name and the same contents \
                 was found near it. Your draft is untouched — Save As to give it a file again.",
                current.display()
            )
            .into(),
        ),
    }
}

/// Refresh the agent rail a beat after the document stops changing.
///
/// `refresh_agent_rail` asks the companion whether it is reachable, and that is a round trip on
/// the bus with the ask timeout behind it. Calling it from `paint` meant one round trip per
/// keystroke, and — once the control surface existed — one inside an action, which has three
/// seconds in total (`UI_ROUNDTRIP` in control.rs) and has no business spending them on somebody
/// else's socket. So the rail lags the document by 400 ms and nothing waits on it.
fn refresh_rail_soon(ui: &DocumentEditorApp, state: &State) {
    let b = state.borrow();
    let text = b.doc.text.clone();
    let title = b.doc.title();
    let back = ui.as_weak();
    b.rail_timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(400),
        move || {
            if let Some(ui) = back.upgrade() {
                refresh_agent_rail(&ui, &text, &title);
            }
        },
    );
}

/// Say what went wrong, twice: on screen for the person, and in `describe.notice` for a mind,
/// which reads the same property. Contract point 4.
///
/// Every mutation below goes through here on its way to either caller, and the `Result` carries
/// on to the control surface so an action refuses with the same sentence the window shows.
fn report<T>(ui: &DocumentEditorApp, outcome: Result<T, String>) -> Result<T, String> {
    match &outcome {
        Ok(_) => ui.set_notice(SharedString::new()),
        Err(e) => {
            tracing::warn!(error = %e, "refused");
            ui.set_notice(e.clone().into());
        }
    }
    outcome
}

/// Keep the unsaved draft, 750 ms after the typing stops.
fn checkpoint(ui: &DocumentEditorApp, state: &State) {
    let b = state.borrow();
    if b.hold_recovery {
        return;
    }
    let path = b.recovery.clone();
    let doc = b.doc.clone();
    let back = ui.as_weak();
    b.recovery_timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(750),
        move || {
            if let Err(e) = document::checkpoint(&path, &doc) {
                if let Some(ui) = back.upgrade() {
                    ui.set_notice(format!("Draft recovery failed: {e}").into());
                }
            }
        },
    );
}

// ── The mutations. One path each, used by the button and by the action ──────

/// Take the text the surface holds and make it the document.
fn edit(ui: &DocumentEditorApp, state: &State, text: String) -> Result<(), String> {
    if let Err(e) = document::validate(&text) {
        // Put the document back on screen: the surface is holding something this app will not
        // keep, and leaving it there would let a person go on typing into a buffer that can
        // never be saved.
        paint(ui, state, true);
        return Err(e);
    }
    state.borrow_mut().doc.text = text;
    search(ui, state, false);
    paint(ui, state, false);
    checkpoint(ui, state);
    Ok(())
}

/// Replace the document's text from somewhere other than the keyboard, and put the caret where
/// the change left it.
fn commit(
    ui: &DocumentEditorApp,
    state: &State,
    text: String,
    selection: Option<(usize, usize)>,
) -> Result<(), String> {
    document::validate(&text)?;
    state.borrow_mut().doc.text = text;
    search(ui, state, false);
    paint(ui, state, true);
    if let Some((a, z)) = selection {
        ui.invoke_select_range(a as i32, z as i32);
        ui.invoke_focus_body();
    }
    checkpoint(ui, state);
    Ok(())
}

/// Open a file, and say what opening it would cost before it costs it.
///
/// `discard` is the caller agreeing to lose the draft on screen. Without it an open onto a dirty
/// document is refused: it used to replace the buffer in silence, so the one action a person
/// reached for when their document had been moved out from under them was also the action that
/// threw the edit away. The window turns the refusal into the Save / Discard / Cancel prompt; a
/// mind gets the refusal itself, with the size of what it is about to lose in it.
fn open_path(
    ui: &DocumentEditorApp,
    state: &State,
    path: &Path,
    discard: bool,
) -> Result<serde_json::Value, String> {
    let target = expanded(&path.to_string_lossy());
    if !discard {
        if let Some(lost) = state.borrow().doc.unsaved() {
            return Err(format!(
                "Opening {} would throw away {}. Save this document first, or open again with \
                 discard=true.",
                target.display(),
                lost
            ));
        }
    }
    let doc = Document::open(&target)?;
    let opened = doc.path.clone().unwrap_or_default();
    let words = document::word_count(&doc.text);
    let bytes = doc.text.len();
    let headings = document::outline(&doc.text).len();
    {
        let mut b = state.borrow_mut();
        b.doc = doc;
        b.matches.clear();
        b.match_index = 0;
        b.intent = Intent::None;
    }
    ui.set_doc_dialog(DIALOG_NONE);
    search(ui, state, false);
    paint(ui, state, true);
    ui.invoke_select_range(0, 0);
    checkpoint(ui, state);
    Ok(serde_json::json!({
        "opened": opened, "bytes": bytes, "words": words, "headings": headings,
    }))
}

/// Start a new, empty document.
///
/// Refuses while there are unsaved changes unless `force`. The window's New button turns that
/// refusal into the Save / Discard / Cancel prompt; an action gets the refusal itself, because a
/// mind asking for a blank document should be told it is about to throw one away rather than
/// have it happen.
fn new_document(
    ui: &DocumentEditorApp,
    state: &State,
    force: bool,
) -> Result<serde_json::Value, String> {
    if !force && state.borrow().doc.dirty() {
        return Err("This document has unsaved changes. Save it first, or discard them in the \
                    window."
            .into());
    }
    {
        let mut b = state.borrow_mut();
        b.doc = Document::blank();
        b.matches.clear();
        b.match_index = 0;
    }
    ui.set_doc_dialog(DIALOG_NONE);
    paint(ui, state, true);
    checkpoint(ui, state);
    Ok(serde_json::json!({"new": true, "path": serde_json::Value::Null}))
}

/// Write the document, and report what is on disk afterwards.
///
/// `path` is `None` for a plain Save, which then needs the document to already have a home. This
/// is THE bug: with no path it used to return in silence, so a person could press Save all day
/// and lose everything. It now says so, and the window turns that into the Save As prompt.
///
/// `overwrite` is the caller having been told that something is already at `path` and answering.
/// Nothing in the window passes it: the Save As prompt still refuses to write over a file it was
/// not expecting, and the one case where that refusal was wrong — the target IS this document's
/// own file, moved — is recognised inside `Document::save` and needs no permission.
fn save_to(
    ui: &DocumentEditorApp,
    state: &State,
    path: Option<PathBuf>,
    overwrite: bool,
) -> Result<serde_json::Value, String> {
    let doc = state.borrow().doc.clone();
    let target = doc.save_target(path)?;
    let Saved { document: saved, stamp } = if overwrite {
        doc.save_over(&target)?
    } else {
        doc.save(&target)?
    };
    let written = saved.path.clone().unwrap_or_default();
    // The write is synchronous on this thread, so nothing can have been typed in between: the
    // document the save agreed with is still the document on screen.
    state.borrow_mut().doc = saved;
    ui.set_doc_dialog(DIALOG_NONE);
    paint(ui, state, false);
    checkpoint(ui, state);
    finish_intent(ui, state);
    Ok(serde_json::json!({
        "saved": written,
        "bytes": stamp.bytes,
        "modified_unix": stamp.modified_unix,
    }))
}

/// Write a copy somewhere else, and leave the document where it lives.
///
/// Not the same thing as Save As, which moves the document's home. A copy is the reason `export`
/// exists at all once the native format is already Markdown.
fn export(
    ui: &DocumentEditorApp,
    state: &State,
    path: &Path,
    html: bool,
) -> Result<serde_json::Value, String> {
    let (text, title) = {
        let b = state.borrow();
        (b.doc.text.clone(), b.doc.title())
    };
    let copy = Document {
        path: None,
        text: if html { document::to_html(&text, &title) } else { text },
        ..Default::default()
    };
    let Saved { document: written, stamp } = copy.save(&expanded(&path.to_string_lossy()))?;
    ui.set_doc_dialog(DIALOG_NONE);
    Ok(serde_json::json!({
        "exported": written.path,
        "format": if html { "html" } else { "markdown" },
        "bytes": stamp.bytes,
        "modified_unix": stamp.modified_unix,
    }))
}

fn set_content(
    ui: &DocumentEditorApp,
    state: &State,
    text: String,
) -> Result<serde_json::Value, String> {
    let words = document::word_count(&text);
    let chars = document::char_count(&text);
    commit(ui, state, text, Some((0, 0)))?;
    Ok(serde_json::json!({"words": words, "chars": chars, "dirty": true}))
}

fn append(
    ui: &DocumentEditorApp,
    state: &State,
    addition: &str,
) -> Result<serde_json::Value, String> {
    let mut text = state.borrow().doc.text.clone();
    // A document is lines. Appending to a document that does not end in one would glue the new
    // paragraph onto the end of the last, which for Markdown changes what it means.
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(addition);
    let end = text.len();
    let words = document::word_count(&text);
    let chars = document::char_count(&text);
    commit(ui, state, text, Some((end, end)))?;
    Ok(serde_json::json!({
        "appended_chars": document::char_count(addition), "words": words, "chars": chars,
    }))
}

/// Recount the matches for whatever is in the find box, and optionally go to the first one.
fn search(ui: &DocumentEditorApp, state: &State, select: bool) {
    let query = ui.get_doc_find_query().to_string();
    let mut b = state.borrow_mut();
    b.matches = document::matches(&b.doc.text, &query);
    b.match_index = 0;
    ui.set_doc_find_count(b.matches.len() as i32);
    let first = b.matches.first().copied();
    drop(b);
    if select {
        if let Some((a, z)) = first {
            ui.invoke_select_range(a as i32, z as i32);
            ui.invoke_focus_body();
        }
    }
}

fn step_match(ui: &DocumentEditorApp, state: &State, forward: bool) {
    let mut b = state.borrow_mut();
    let len = b.matches.len();
    if len == 0 {
        return;
    }
    b.match_index = (b.match_index + if forward { 1 } else { len - 1 }) % len;
    let (a, z) = b.matches[b.match_index];
    drop(b);
    ui.invoke_select_range(a as i32, z as i32);
    ui.invoke_focus_body();
}

fn replace_matches(
    ui: &DocumentEditorApp,
    state: &State,
    all: bool,
) -> Result<serde_json::Value, String> {
    let with = ui.get_doc_replace_text().to_string();
    let (text, ranges) = {
        let b = state.borrow();
        let ranges: Vec<(usize, usize)> = if all {
            b.matches.clone()
        } else {
            b.matches.get(b.match_index).copied().into_iter().collect()
        };
        (b.doc.text.clone(), ranges)
    };
    let count = ranges.len();
    if count == 0 {
        return Err("Nothing matched, so nothing was replaced.".into());
    }
    let out = document::replace(&text, &ranges, &with)?;
    // `commit` recounts the matches against the new text, so a second search here would only
    // repeat it.
    commit(ui, state, out, None)?;
    Ok(serde_json::json!({"replaced": count}))
}

/// A formatting button: read the live selection, do the string work, put the selection back.
fn apply_format(ui: &DocumentEditorApp, state: &State, what: Format) -> Result<(), String> {
    let text = state.borrow().doc.text.clone();
    let anchor = ui.get_doc_anchor().max(0) as usize;
    let cursor = ui.get_doc_cursor().max(0) as usize;
    let edit = document::format(&text, anchor, cursor, what)?;
    commit(ui, state, edit.text, Some((edit.start, edit.end)))
}

// ── The file prompt ─────────────────────────────────────────────────────────

fn prompt_path(ui: &DocumentEditorApp, state: &State, which: i32) {
    let suggestion = {
        let b = state.borrow();
        match which {
            DIALOG_OPEN => "~/Documents/".to_string(),
            DIALOG_EXPORT_HTML => {
                let mut p = b.doc.home();
                p.set_extension("html");
                p.display().to_string()
            }
            _ => b.doc.home().display().to_string(),
        }
    };
    ui.set_doc_dialog_path(suggestion.into());
    ui.set_doc_dialog_error(SharedString::new());
    ui.set_doc_dialog(which);
}

fn prompt_unsaved(ui: &DocumentEditorApp, state: &State, intent: Intent) {
    state.borrow_mut().intent = intent;
    ui.set_doc_dialog_error(SharedString::new());
    ui.set_doc_dialog(DIALOG_UNSAVED);
}

/// Whatever the unsaved prompt was standing in front of, now that the draft is dealt with.
fn finish_intent(ui: &DocumentEditorApp, state: &State) {
    let intent = std::mem::replace(&mut state.borrow_mut().intent, Intent::None);
    match intent {
        Intent::None => {}
        Intent::New => {
            let _ = report(ui, new_document(ui, state, true));
        }
        Intent::Close => {
            // The draft is saved or discarded, so the final checkpoint in `main` will clear the
            // recovery file rather than offer a draft nobody is missing.
            let _ = ui.hide();
            let _ = slint::quit_event_loop();
        }
        Intent::Open(path) => {
            // Whichever way the prompt was answered — saved, or given up on purpose — the draft
            // has been dealt with, so this open is no longer discarding anything unasked.
            let _ = report(ui, open_path(ui, state, &path, true));
        }
    }
}

// ── Wiring ──────────────────────────────────────────────────────────────────

fn wire(ui: &DocumentEditorApp, recovery: PathBuf, have_argv: bool) -> State {
    let mut hold_recovery = false;
    // Recorded here and said at the end of startup: a file named on the command line is opened
    // after this, and a successful open clears the notice. `main` puts this back afterwards.
    let mut startup_notice = String::new();
    let doc = match document::recover(&recovery) {
        Ok(Some(_)) if have_argv => {
            // A file was named on the command line and there is also an unsaved draft. The file
            // gets the window; the draft is left on disk untouched and offered next time, which
            // is the only outcome here that does not throw work away. `hold_recovery` is what
            // stops this session's own checkpoints from writing over it.
            hold_recovery = true;
            startup_notice = format!(
                "An unsaved draft from a previous session is waiting in {}. It was left alone; \
                 open yDoc with no file to pick it up.",
                recovery.display()
            );
            Document::blank()
        }
        Ok(Some(draft)) => {
            startup_notice =
                "A draft from a previous session was recovered. Review it, then Save or Save As."
                    .into();
            draft
        }
        Ok(None) => Document::blank(),
        Err(e) => {
            // An unreadable recovery file is preserved rather than overwritten: it may still be
            // the only copy of something.
            hold_recovery = true;
            startup_notice = e;
            Document::blank()
        }
    };

    // The folder watch and its mailbox. The watcher's own thread cannot touch the document, so
    // it puts what it saw on the channel and pokes `doc-file-event`, which is handled below on
    // the UI thread where the document lives.
    let (moves, inbox) = mpsc::channel();
    let wake = ui.as_weak();
    let follow = follow::Watch::new(moves, move || {
        let _ = wake.upgrade_in_event_loop(|ui| ui.invoke_doc_file_event());
    });

    let state: State = Rc::new(RefCell::new(Editor {
        doc,
        matches: vec![],
        match_index: 0,
        pending: Default::default(),
        intent: Intent::None,
        recovery,
        hold_recovery,
        startup_notice,
        recovery_timer: slint::Timer::default(),
        rail_timer: slint::Timer::default(),
        follow,
        moves: inbox,
    }));

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_file_event(move || {
            let Some(ui) = weak.upgrade() else { return };
            file_moved(&ui, &st);
        });
    }

    // ── The document ──
    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_new(move || {
            let Some(ui) = weak.upgrade() else { return };
            // New goes through the same guard an action does. The window turns the refusal into
            // the Save / Discard / Cancel prompt rather than dropping the draft on the floor.
            if new_document(&ui, &st, false).is_err() {
                prompt_unsaved(&ui, &st, Intent::New);
            } else {
                ui.set_notice(SharedString::new());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_open(move || {
            let Some(ui) = weak.upgrade() else { return };
            prompt_path(&ui, &st, DIALOG_OPEN);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_save(move || {
            let Some(ui) = weak.upgrade() else { return };
            if st.borrow().doc.path.is_none() {
                // The document has no home yet, so Save's first job is to ask for one. This is
                // the branch that used to be `tracing::info!("No file path set"); return;`.
                prompt_path(&ui, &st, DIALOG_SAVE_AS);
                return;
            }
            let _ = report(&ui, save_to(&ui, &st, None, false));
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_save_as(move || {
            let Some(ui) = weak.upgrade() else { return };
            prompt_path(&ui, &st, DIALOG_SAVE_AS);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_export_md(move || {
            let Some(ui) = weak.upgrade() else { return };
            prompt_path(&ui, &st, DIALOG_EXPORT_MD);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_export_html(move || {
            let Some(ui) = weak.upgrade() else { return };
            prompt_path(&ui, &st, DIALOG_EXPORT_HTML);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_content_changed(move |text| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = report(&ui, edit(&ui, &st, text.to_string()));
        });
    }

    // ── The file prompt ──
    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_dialog_confirm(move || {
            let Some(ui) = weak.upgrade() else { return };
            let which = ui.get_doc_dialog();
            let path = expanded(&ui.get_doc_dialog_path());
            let outcome: Result<(), String> = match which {
                DIALOG_OPEN => {
                    if st.borrow().doc.dirty() {
                        // There is work on screen that is in no file. Ask about that before the
                        // file being opened takes the window; the path rides on the intent, so
                        // answering the prompt does not mean typing it again. This is the third
                        // of the three dead ends in the issue: Open used to replace the buffer
                        // in silence and the unsaved edit was simply gone.
                        prompt_unsaved(&ui, &st, Intent::Open(path));
                        return;
                    }
                    open_path(&ui, &st, &path, false).map(|_| ())
                }
                DIALOG_SAVE_AS => save_to(&ui, &st, Some(path), false).map(|_| ()),
                DIALOG_EXPORT_MD => export(&ui, &st, &path, false).map(|_| ()),
                DIALOG_EXPORT_HTML => export(&ui, &st, &path, true).map(|_| ()),
                DIALOG_UNSAVED => {
                    if st.borrow().doc.path.is_none() {
                        // Keeping the changes means choosing a file first; the intent is already
                        // recorded, so the save that follows will carry it out.
                        prompt_path(&ui, &st, DIALOG_SAVE_AS);
                        return;
                    }
                    save_to(&ui, &st, None, false).map(|_| ())
                }
                _ => Ok(()),
            };
            match report(&ui, outcome) {
                Ok(()) => ui.set_doc_dialog_error(SharedString::new()),
                // The prompt stays open with the reason in it, so the path can be corrected
                // rather than retyped from the beginning.
                Err(e) => ui.set_doc_dialog_error(e.into()),
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_dialog_cancel(move || {
            let Some(ui) = weak.upgrade() else { return };
            st.borrow_mut().intent = Intent::None;
            ui.set_doc_dialog(DIALOG_NONE);
            ui.set_doc_dialog_error(SharedString::new());
            ui.invoke_focus_body();
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_dialog_discard(move || {
            let Some(ui) = weak.upgrade() else { return };
            // The draft is given up on purpose, so the recovery file has to go with it —
            // offering it again on the next start would resurrect what was just thrown away.
            // Marking the text as agreed-with is what makes `checkpoint` clear the file instead
            // of writing the draft out again.
            let (recovery, doc) = {
                let mut b = st.borrow_mut();
                b.recovery_timer.stop();
                b.doc.baseline = b.doc.text.clone();
                b.doc.recovered = false;
                (b.recovery.clone(), b.doc.clone())
            };
            let _ = document::checkpoint(&recovery, &doc);
            ui.set_doc_dialog(DIALOG_NONE);
            finish_intent(&ui, &st);
        });
    }

    // ── Undo and redo ──
    //
    // The TextInput keeps its own edit history under the caret. These press it and then take the
    // text back out, because Slint's undo does not raise `edited` and the document would
    // otherwise be one step behind what is on screen.
    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_undo(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.invoke_undo_body();
            let text = ui.get_doc_content().to_string();
            let _ = report(&ui, edit(&ui, &st, text));
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_redo(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.invoke_redo_body();
            let text = ui.get_doc_content().to_string();
            let _ = report(&ui, edit(&ui, &st, text));
        });
    }

    // ── Formatting ──
    format_button(ui, &state, |u, f| u.on_doc_format_bold(f), Format::Bold);
    format_button(ui, &state, |u, f| u.on_doc_format_italic(f), Format::Italic);
    format_button(ui, &state, |u, f| u.on_doc_format_strikethrough(f), Format::Strikethrough);
    format_button(ui, &state, |u, f| u.on_doc_format_bullet(f), Format::Bullet);
    format_button(ui, &state, |u, f| u.on_doc_format_checklist(f), Format::Checklist);
    format_button(ui, &state, |u, f| u.on_doc_format_quote(f), Format::Quote);
    format_button(ui, &state, |u, f| u.on_doc_format_code(f), Format::CodeBlock);
    format_button(ui, &state, |u, f| u.on_doc_format_inline_code(f), Format::InlineCode);
    format_button(ui, &state, |u, f| u.on_doc_format_divider(f), Format::Divider);

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_format_heading(move |level| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = report(&ui, apply_format(&ui, &st, Format::Heading(level.clamp(1, 6) as u8)));
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_format_link(move |url| {
            let Some(ui) = weak.upgrade() else { return };
            let text = st.borrow().doc.text.clone();
            let anchor = ui.get_doc_anchor().max(0) as usize;
            let cursor = ui.get_doc_cursor().max(0) as usize;
            let outcome = document::format_link(&text, anchor, cursor, &url)
                .and_then(|e| commit(&ui, &st, e.text, Some((e.start, e.end))));
            let _ = report(&ui, outcome);
        });
    }

    // ── Find and replace ──
    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_find_changed(move |_| {
            let Some(ui) = weak.upgrade() else { return };
            search(&ui, &st, false);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_find_next(move || {
            let Some(ui) = weak.upgrade() else { return };
            step_match(&ui, &st, true);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_find_prev(move || {
            let Some(ui) = weak.upgrade() else { return };
            step_match(&ui, &st, false);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_replace_one(move || {
            let Some(ui) = weak.upgrade() else { return };
            let _ = report(&ui, replace_matches(&ui, &st, false));
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_replace_all(move || {
            let Some(ui) = weak.upgrade() else { return };
            let _ = report(&ui, replace_matches(&ui, &st, true));
        });
    }

    // ── The outline ──
    {
        let weak = ui.as_weak();
        ui.on_doc_heading_clicked(move |offset| {
            let Some(ui) = weak.upgrade() else { return };
            // block-index carries the heading's byte offset, which is what the editing surface
            // takes. The outline used to be decorative: this handler was a log line.
            ui.invoke_select_range(offset, offset);
            ui.invoke_focus_body();
        });
    }

    // ── The agent layer ──
    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_ai_submit(move |prompt| {
            let Some(ui) = weak.upgrade() else { return };
            ask_companion(&ui, &st, &prompt, AskKind::Rewrite);
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_ai_summarize(move || {
            let Some(ui) = weak.upgrade() else { return };
            ask_companion(
                &ui,
                &st,
                "Summarise this document in at most five bullet points.",
                AskKind::Summary,
            );
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_doc_ai_improve(move || {
            let Some(ui) = weak.upgrade() else { return };
            ask_companion(
                &ui,
                &st,
                "Rewrite this document to be shorter and clearer, keeping every fact.",
                AskKind::Rewrite,
            );
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_proposal_applied(move || {
            let Some(ui) = weak.upgrade() else { return };
            let text = st
                .borrow()
                .pending
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default();
            if text.is_empty() {
                return;
            }
            // Left unsaved on purpose, the same as Notes: generated text is looked at before it
            // is kept. The difference from before is that it CAN now be kept.
            let _ = report(&ui, commit(&ui, &st, text, Some((0, 0))));
            if let Ok(mut p) = st.borrow().pending.lock() {
                p.clear();
            }
            ui.set_proposal(AgentProposal::default());
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_proposal_dismissed(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut p) = st.borrow().pending.lock() {
                    p.clear();
                }
                ui.set_proposal(AgentProposal::default());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.on_agent_suggestion_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            match id.as_str() {
                "summarize" => ask_companion(
                    &ui,
                    &st,
                    "Summarise this document in at most five bullet points.",
                    AskKind::Summary,
                ),
                "tighten" => ask_companion(
                    &ui,
                    &st,
                    "Rewrite this document to be shorter and clearer, keeping every fact.",
                    AskKind::Rewrite,
                ),
                other => tracing::warn!(id = other, "unknown rail suggestion"),
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_agent_context_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            // The outline rows in the rail carry the same byte offset the sidebar does, so
            // pressing one goes to that heading. A memory row has nowhere in this document to go.
            if let Some(offset) = id.strip_prefix("heading:").and_then(|o| o.parse::<i32>().ok()) {
                ui.invoke_select_range(offset, offset);
                ui.invoke_focus_body();
            }
        });
    }

    // ── Closing ──
    {
        let weak = ui.as_weak();
        let st = state.clone();
        ui.window().on_close_requested(move || {
            let Some(ui) = weak.upgrade() else {
                return slint::CloseRequestResponse::HideWindow;
            };
            if !st.borrow().doc.dirty() {
                return slint::CloseRequestResponse::HideWindow;
            }
            // Dirty: the window does not go away without asking, whatever else is on screen —
            // a prompt about some other file is replaced by the one that matters. The recovery
            // file is the second net under this, for the closes nobody gets to ask about.
            if ui.get_doc_dialog() != DIALOG_UNSAVED {
                prompt_unsaved(&ui, &st, Intent::Close);
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }

    search(ui, &state, false);
    paint(ui, &state, true);
    ui.invoke_focus_body();
    state
}

/// Register one formatting button. They differ only in which callback and which mark, and
/// writing eleven near-identical blocks is how one of them ends up wired to the wrong thing.
fn format_button(
    ui: &DocumentEditorApp,
    state: &State,
    register: impl Fn(&DocumentEditorApp, Box<dyn Fn()>),
    what: Format,
) {
    let weak = ui.as_weak();
    let st = state.clone();
    register(
        ui,
        Box::new(move || {
            let Some(ui) = weak.upgrade() else { return };
            let _ = report(&ui, apply_format(&ui, &st, what));
        }),
    );
}

/// What kind of answer an ask is for, which decides both the prompt's closing line and what the
/// proposal offers to do with the answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AskKind {
    /// The answer is a new version of the document, and applying it replaces what is on screen.
    Rewrite,
    /// The answer is about the document — a summary. It is shown beside the document and there
    /// is nothing to apply: swapping a five-bullet summary in for the thing it summarises is
    /// losing the document, whatever the card calls itself.
    Summary,
}

/// The ask, built. Split out because the closing line is the one that used to be wrong: every
/// ask, summaries included, ended with "Reply with the rewritten document only", so the model
/// answered a summary request with a rewrite and the card offered to swap it in.
fn ask_prompt(prompt: &str, body: &str, kind: AskKind) -> String {
    let closing = match kind {
        AskKind::Rewrite => "Reply with the rewritten document only.",
        AskKind::Summary => "Reply with the summary only.",
    };
    format!(
        "{prompt}\n\nHere is the document. Use only what it says; invent nothing. \
         {closing}\n\n{body}"
    )
}

/// The card an answer turns into, and what applying it would write — `None` when there is
/// nothing to apply. Pure so a test can hold the contract without a window: a summary proposes
/// no replacement, and a failure proposes nothing at all.
fn proposal_for(
    outcome: Result<String, companion::AskError>,
    kind: AskKind,
) -> (AgentProposal, Option<String>) {
    match outcome {
        Ok(text) if kind == AskKind::Rewrite => {
            let words = document::word_count(&text);
            (
                AgentProposal {
                    title: "Rewritten document".into(),
                    body: text.clone().into(),
                    source: "from this document".into(),
                    // This one really does replace what is on screen, so it says so and says
                    // how big the replacement is.
                    impact: format!("Replaces the document with {words} words, unsaved").into(),
                    destructive: false,
                    verb: "Replace".into(),
                },
                Some(text),
            )
        }
        Ok(text) => (
            AgentProposal {
                title: "Summary".into(),
                body: text.into(),
                source: "from this document".into(),
                impact: "The document itself is left alone".into(),
                destructive: false,
                verb: "Close".into(),
            },
            None,
        ),
        Err(e) => (
            AgentProposal {
                title: "The companion did not answer".into(),
                body: e.to_string().into(),
                verb: "Close".into(),
                ..Default::default()
            },
            None,
        ),
    }
}

/// Ask the companion about this document, off the UI thread.
///
/// The answer arrives as a proposal that says what applying it would do — which for a rewrite is
/// "replace the document", so it says that before the button is pressed, and for a summary is
/// nothing: it is read, not applied. Nothing on the control surface waits on this: an action has
/// three seconds (`UI_ROUNDTRIP` in control.rs) and a language model does not.
fn ask_companion(ui: &DocumentEditorApp, state: &State, prompt: &str, kind: AskKind) {
    let (body, pending) = {
        let b = state.borrow();
        (b.doc.text.clone(), b.pending.clone())
    };
    if body.trim().is_empty() {
        ui.set_proposal(AgentProposal {
            title: "Nothing to work on".into(),
            body: "This document is empty.".into(),
            verb: "Close".into(),
            ..Default::default()
        });
        return;
    }

    ui.set_proposal_working(true);
    ui.set_proposal(AgentProposal {
        title: match kind {
            AskKind::Rewrite => "Rewriting".into(),
            AskKind::Summary => "Summarising".into(),
        },
        source: "from this document".into(),
        ..Default::default()
    });

    let ask = ask_prompt(prompt, &body, kind);
    let back = ui.as_weak();
    std::thread::spawn(move || {
        let outcome = companion::ask(&ask);
        let _ = back.upgrade_in_event_loop(move |ui| {
            ui.set_proposal_working(false);
            if let Err(e) = &outcome {
                tracing::warn!(error = %e, "Companion call failed");
            }
            let (proposal, write) = proposal_for(outcome, kind);
            ui.set_proposal(proposal);
            if let Some(text) = write {
                if let Ok(mut p) = pending.lock() {
                    *p = text;
                }
            }
        });
    });
}

// ── The control surface ─────────────────────────────────────────────────────

fn publish_control(ui: &DocumentEditorApp, state: State) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let mut app = App::new(APP_ID).describe({
        let weak = ui.as_weak();
        let st = state.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("yDoc — closing");
            };
            let b = st.borrow();
            let d = &b.doc;
            let text = &d.text;
            let words = document::word_count(text);
            let total = document::char_count(text);
            let excerpt: String = text.chars().take(EXCERPT_CHARS).collect();
            let where_it_lives = d
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "no file yet".into());
            View::new(format!(
                "yDoc — \u{201c}{}\u{201d}, {words} words, {} \u{b7} {where_it_lives}",
                d.title(),
                if d.dirty() { "unsaved" } else { "saved" },
            ))
            .with("format", "markdown")
            .with("path", serde_json::json!(d.path))
            .with("title", d.title())
            .with("dirty", d.dirty())
            .with("recovered_draft", d.recovered)
            .with("words", words)
            .with("chars", total)
            .with(
                "outline",
                document::outline(text)
                    .into_iter()
                    .map(|h| {
                        serde_json::json!({"title": h.title, "level": h.level, "line": h.line,
                                           "offset": h.offset})
                    })
                    .collect::<Vec<_>>(),
            )
            .with("content", excerpt.clone())
            .with("content_chars_shown", document::char_count(&excerpt))
            .with("content_chars_total", total)
            .with("find_query", ui.get_doc_find_query().to_string())
            .with("find_matches", b.matches.len())
            // A prompt is open in the window, so a person is part-way through something and an
            // action that changed the document under them would be a surprise.
            .with("prompt_open", ui.get_doc_dialog() != DIALOG_NONE)
            .with("notice", ui.get_notice().to_string())
        }
    });

    /// Nothing may be driven while the window is asking the person a question.
    fn ready(ui: &DocumentEditorApp) -> Result<(), String> {
        if ui.get_doc_dialog() != DIALOG_NONE {
            return Err("yDoc is asking the person about a file. Answer that prompt first.".into());
        }
        Ok(())
    }

    macro_rules! action {
        ($spec:expr, $run:expr) => {{
            let weak = ui.as_weak();
            let st = state.clone();
            let run = $run;
            app = app.action($spec, move |args| {
                let ui = weak.upgrade().ok_or("yDoc is closing")?;
                ready(&ui)?;
                report(&ui, run(&ui, &st, args))
            });
        }};
    }

    action!(
        Action::new("open", "Open a Markdown document by path")
            .arg(Param::text("path"))
            .arg(
                Param::flag("discard")
                    .optional()
                    .describe("Open even though this document has unsaved changes, losing them"),
            ),
        |ui: &DocumentEditorApp, st: &State, args: &serde_json::Value| -> Result<serde_json::Value, String> {
            let path = args["path"].as_str().ok_or("`open` needs a path")?;
            let discard = args["discard"].as_bool().unwrap_or(false);
            open_path(ui, st, Path::new(path), discard)
        }
    );

    action!(
        Action::new("new", "Start an empty document"),
        |ui: &DocumentEditorApp, st: &State, _a: &serde_json::Value| -> Result<serde_json::Value, String> { new_document(ui, st, false) }
    );

    action!(
        Action::new("save", "Write the document to the file it came from"),
        |ui: &DocumentEditorApp, st: &State, _a: &serde_json::Value| -> Result<serde_json::Value, String> { save_to(ui, st, None, false) }
    );

    action!(
        Action::new("save_as", "Write the document to a path and keep it there")
            .arg(Param::text("path"))
            .arg(
                Param::flag("overwrite")
                    .optional()
                    .describe("Replace a file that is already at that path"),
            ),
        |ui: &DocumentEditorApp, st: &State, args: &serde_json::Value| -> Result<serde_json::Value, String> {
            let path = args["path"].as_str().ok_or("`save_as` needs a path")?;
            let overwrite = args["overwrite"].as_bool().unwrap_or(false);
            save_to(ui, st, Some(expanded(path)), overwrite)
        }
    );

    action!(
        Action::new("set_content", "Replace the document with this Markdown")
            .arg(Param::text("text")),
        |ui: &DocumentEditorApp, st: &State, args: &serde_json::Value| -> Result<serde_json::Value, String> {
            let text = args["text"].as_str().ok_or("`set_content` needs text")?;
            set_content(ui, st, text.to_string())
        }
    );

    action!(
        Action::new("append", "Add Markdown to the end of the document").arg(Param::text("text")),
        |ui: &DocumentEditorApp, st: &State, args: &serde_json::Value| -> Result<serde_json::Value, String> {
            let text = args["text"].as_str().ok_or("`append` needs text")?;
            append(ui, st, text)
        }
    );

    action!(
        Action::new("find", "Search the document and show the matches")
            .arg(Param::text("query"))
            .risk("safe"),
        |ui: &DocumentEditorApp, st: &State, args: &serde_json::Value| -> Result<serde_json::Value, String> {
            let query = args["query"].as_str().ok_or("`find` needs a query")?;
            ui.set_doc_find_query(query.into());
            ui.set_doc_show_find(true);
            search(ui, st, true);
            Ok(serde_json::json!({"query": query, "matches": st.borrow().matches.len()}))
        }
    );

    action!(
        Action::new("replace_all", "Replace every match in the document")
            .arg(Param::text("find"))
            .arg(Param::text("with")),
        |ui: &DocumentEditorApp, st: &State, args: &serde_json::Value| -> Result<serde_json::Value, String> {
            let query = args["find"].as_str().ok_or("`replace_all` needs `find`")?;
            let with = args["with"].as_str().ok_or("`replace_all` needs `with`")?;
            let text = st.borrow().doc.text.clone();
            let (out, count) = document::replace_all(&text, query, with)?;
            ui.set_doc_find_query(query.into());
            ui.set_doc_replace_text(with.into());
            commit(ui, st, out, None)?;
            search(ui, st, false);
            Ok(serde_json::json!({"replaced": count}))
        }
    );

    action!(
        Action::new("export_markdown", "Write a Markdown copy to a path, leaving the document in its own file")
            .arg(Param::text("path")),
        |ui: &DocumentEditorApp, st: &State, args: &serde_json::Value| -> Result<serde_json::Value, String> {
            let path = args["path"].as_str().ok_or("`export_markdown` needs a path")?;
            export(ui, st, Path::new(path), false)
        }
    );

    {
        // `show` exists for the handover in `main`: a second launch with no file asks the running
        // window to come forward instead of opening another one.
        let weak = ui.as_weak();
        app = app.action(
            Action::new("show", "Bring the yDoc window forward").risk("safe"),
            move |_| {
                let ui = weak.upgrade().ok_or("yDoc is closing")?;
                let _ = ui.show();
                Ok(serde_json::json!({"shown": true}))
            },
        );
    }

    app.serve();
}

#[cfg(test)]
mod ask_tests {
    use super::{ask_prompt, proposal_for, AskKind};
    use yantrik_app_runtime::companion::{AskError, NO_MODEL_HINT};

    const DOC: &str = "First line of the document. Second line of it.";

    /// The defect: Summarize asked for a rewritten document and offered to replace the document
    /// with the answer — applying a five-bullet summary destroyed the prose it summarised.
    #[test]
    fn a_summary_is_shown_beside_the_document_and_never_swaps_into_it() {
        let (proposal, write) = proposal_for(Ok("- one\n- two".to_string()), AskKind::Summary);
        assert_eq!(write, None, "a summary must leave nothing to apply");
        assert_eq!(proposal.verb, "Close");
        assert_eq!(proposal.body, "- one\n- two");
        assert!(!proposal.impact.to_string().contains("Replaces"));
    }

    #[test]
    fn a_rewrite_still_offers_to_replace_the_document() {
        let (proposal, write) = proposal_for(Ok("Shorter.".to_string()), AskKind::Rewrite);
        assert_eq!(write.as_deref(), Some("Shorter."));
        assert_eq!(proposal.verb, "Replace");
    }

    /// A missing model is a refusal, not an answer: the card says the one sentence that points
    /// at Settings → AI, and nothing is staged for replacement.
    #[test]
    fn no_model_leaves_the_document_alone() {
        let (proposal, write) = proposal_for(Err(AskError::NoModel), AskKind::Rewrite);
        assert_eq!(write, None);
        assert_eq!(proposal.body, NO_MODEL_HINT);
        assert_eq!(proposal.verb, "Close");
    }

    #[test]
    fn the_prompt_asks_for_what_the_kind_can_use() {
        let summary = ask_prompt("Summarise it.", DOC, AskKind::Summary);
        assert!(summary.contains("Reply with the summary only."));
        assert!(!summary.contains("rewritten document"));
        let rewrite = ask_prompt("Tighten it.", DOC, AskKind::Rewrite);
        assert!(rewrite.contains("Reply with the rewritten document only."));
        assert!(rewrite.ends_with(DOC));
    }
}
