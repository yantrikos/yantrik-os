//! Yantrik Snippet Manager — standalone app binary.
//!
//! It could not keep a snippet. `on_snip_save` was `tracing::info!` over `let _ = (code, tags)`,
//! so the four values the editor sent were dropped on the floor; the model was never updated,
//! and `on_snip_select` then repainted the editor out of that stale model — so typing, clicking
//! another snippet and clicking back silently reverted the typing. Copy, which is the whole
//! point of a snippet manager, was a log line. There was no store anywhere in the crate, so
//! after a restart the window could only ever be empty. Search, the tag chips, favourites, the
//! collections, export, import, the language menu and the version panel were all log lines too.
//!
//! What it does now: keeps snippets in `~/.local/share/yantrik/snippets/snippets.json`, puts a
//! snippet on the clipboard with `wl-copy`, searches and filters what it has, and answers on a
//! control surface so the launcher, a file on the command line and a mind all reach the same
//! window. Every mutation goes through one function in `store.rs` that writes the file and puts
//! memory back if the write fails, so the screen and the disk cannot tell different stories.
//!
//! The controls that had nothing behind them and could not be given anything small and correct
//! were removed rather than left to be pressed: see the comments in `snippet_manager.slint`.

use std::cell::RefCell;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;

mod store;
use store::{Imported, Patch, Snippet, Store, ALL, FAVORITES};

slint::include_modules!();

/// How many snippets `describe` carries. The state is a glance, not a transcript — the true
/// count travels beside it, so a caller is never misled about how many there are.
const LISTING_CAP: usize = 20;

/// How long "Copied!" stays on the button.
const COPY_FEEDBACK: Duration = Duration::from_millis(1400);

/// The id this app answers to.
///
/// `snippets`, not `snippet-manager`, because that is the id the launcher's route carries —
/// `(&["snippets", "snippet_manager"], Launch::Program { id: "snippets", … })` in
/// `crates/yantrik-ui/src/wire/dock.rs` — and an app whose surface is called one thing while the
/// launcher calls it another is an app a mind can open and then not find. The single-instance
/// claim uses the same word for the same reason.
const APP_ID: &str = "snippets";

/// Everything this window holds, outside Slint.
///
/// The store is the truth about snippets; the three filters and the selection are the truth
/// about what is being looked at, and they live here rather than only in Slint properties
/// because `describe` has to be able to say what the person is looking at.
struct Session {
    store: Store,
    query: String,
    tag: String,
    collection: i32,
    selected: i32,
    /// The last thing that went wrong, in words. Shown on screen and published in `describe`.
    notice: String,
}

type State = Rc<RefCell<Session>>;

fn main() {
    init_tracing("yantrik-snippet-manager");

    // A VM forcing software OpenGL has no GPU to accelerate femtovg; render on the CPU instead
    // and leave an explicit renderer choice alone. Same rule the editor, Images and the shell
    // follow.
    if std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref() == Ok("1")
        && matches!(std::env::var("SLINT_BACKEND").as_deref(), Ok("winit") | Err(_))
    {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }

    let path = std::env::args_os().nth(1).map(PathBuf::from);

    // One window per app. A second launch hands its file to the running one and focuses it,
    // rather than opening a second window the person did not ask for — contract point 5, the
    // same handover Images does.
    let Some(_instance) = instance::claim(APP_ID) else {
        let request = match &path {
            Some(p) => serde_json::json!({"action": "import", "args": {"path": p}}),
            None => serde_json::json!({"action": "show", "args": {}}),
        };
        let client = SyncRpcClient::for_service(&control::service_id_for(APP_ID))
            .with_timeout(Duration::from_secs(3));
        for _ in 0..20 {
            if let Ok(reply) = client.call("app.act", request.clone()) {
                if reply["accepted"] != true {
                    eprintln!("Snippets declined the request: {reply}");
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("Snippets is already starting; try again in a moment.");
        return;
    };

    let app = SnippetManagerApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    let store = Store::open();
    let state: State = Rc::new(RefCell::new(Session {
        notice: store.notice().to_string(),
        store,
        query: String::new(),
        tag: String::new(),
        collection: ALL,
        selected: -1,
    }));

    // A file on the command line is kept before the window is shown, so the snippet it made is
    // the one selected when the window appears.
    if let Some(path) = path {
        match import_path(&state, &path) {
            Ok(imported) => {
                if let Some(id) = imported.ids.first() {
                    state.borrow_mut().selected = *id;
                }
            }
            Err(e) => state.borrow_mut().notice = e,
        }
    }

    wire(&app, &state);
    show_all(&app, &state);
    publish_control(&app, state.clone());

    run_until_closed(&app, "yantrik-snippet-manager");
}

// ── What the window shows ────────────────────────────────────────────

/// The list, the chips, the sidebar and the counts. Never the editor.
///
/// Deliberately separate from [`show_detail`]: typing in the search box repaints the list on
/// every keystroke, and a repaint that also rewrote the editor would throw away whatever was
/// being typed into it — which is a new way of committing the bug this app was fixed for.
fn show_list(ui: &SnippetManagerApp, state: &State) {
    let session = state.borrow();
    let rows = session
        .store
        .matching(&session.query, &session.tag, session.collection);
    let now = store::now();

    let items: Vec<SnippetData> = rows
        .iter()
        .map(|s| SnippetData {
            id: s.id,
            title: s.title.clone().into(),
            language: s.language.clone().into(),
            code: s.code.clone().into(),
            tags: s.tags.clone().into(),
            preview: s.preview().into(),
            date_text: store::relative(s.updated, now).into(),
            is_selected: s.id == session.selected,
            is_favorite: s.favorite,
            created_date: store::date(s.created).into(),
            last_used_date: store::relative(s.used, now).into(),
            use_count: s.use_count,
            collection_id: s.collection,
        })
        .collect();
    ui.set_snippets(ModelRc::new(VecModel::from(items)));

    let tags: Vec<TagChip> = session
        .store
        .tags()
        .into_iter()
        .map(|name| TagChip {
            is_active: name.to_lowercase() == session.tag.to_lowercase(),
            name: name.into(),
        })
        .collect();
    ui.set_tag_chips(ModelRc::new(VecModel::from(tags)));

    let total = session.store.snippets().len();
    let mut collections = vec![
        CollectionData {
            id: ALL,
            name: "All Snippets".into(),
            snippet_count: total as i32,
            is_builtin: true,
            is_selected: session.collection == ALL,
            icon: "\u{1F4CB}".into(),
        },
        CollectionData {
            id: FAVORITES,
            name: "Favorites".into(),
            snippet_count: session.store.favorites() as i32,
            is_builtin: true,
            is_selected: session.collection == FAVORITES,
            icon: "\u{2B50}".into(),
        },
    ];
    for collection in session.store.collections() {
        collections.push(CollectionData {
            id: collection.id,
            name: collection.name.clone().into(),
            snippet_count: session
                .store
                .snippets()
                .iter()
                .filter(|s| s.collection == collection.id)
                .count() as i32,
            is_builtin: false,
            is_selected: session.collection == collection.id,
            icon: "\u{1F4C1}".into(),
        });
    }
    ui.set_collections(ModelRc::new(VecModel::from(collections)));

    ui.set_snippet_count(total as i32);
    // -1 is the screen's "no search is narrowing this", which is not the same as "nothing
    // matched" — and drawing 0 for both was how the old blank window looked deliberate.
    ui.set_match_count(if session.query.trim().is_empty() && session.tag.is_empty() {
        -1
    } else {
        rows.len() as i32
    });
    ui.set_search_query(session.query.clone().into());
    ui.set_notice(session.notice.clone().into());
}

/// The editor pane, painted from the store — never from the list model, which is what the old
/// `on_snip_select` did.
fn show_detail(ui: &SnippetManagerApp, state: &State) {
    {
        let session = state.borrow();
        match session.store.get(session.selected) {
            Some(snippet) => {
                let now = store::now();
                ui.set_detail_id(snippet.id);
                ui.set_detail_title(snippet.title.clone().into());
                ui.set_detail_language(snippet.language.clone().into());
                ui.set_detail_code(snippet.code.clone().into());
                ui.set_detail_tags(snippet.tags.clone().into());
                ui.set_detail_is_favorite(snippet.favorite);
                ui.set_detail_created_date(store::date(snippet.created).into());
                ui.set_detail_last_used_date(store::relative(snippet.used, now).into());
                ui.set_detail_use_count(snippet.use_count);
            }
            None => {
                ui.set_detail_id(-1);
                ui.set_detail_title(SharedString::new());
                ui.set_detail_language(SharedString::new());
                ui.set_detail_code(SharedString::new());
                ui.set_detail_tags(SharedString::new());
                ui.set_detail_is_favorite(false);
                ui.set_detail_created_date(SharedString::new());
                ui.set_detail_last_used_date(SharedString::new());
                ui.set_detail_use_count(0);
            }
        }
    }
    refresh_agent_rail(ui);
}

fn show_all(ui: &SnippetManagerApp, state: &State) {
    show_list(ui, state);
    show_detail(ui, state);
}

/// What Yantrik knows about what is on screen.
///
/// The open snippet and one thing worth asking about it, read off the editor's own properties —
/// which are painted from the store — so this can also be called from the thread hop that brings
/// the companion's answer back, where the store is not reachable.
///
/// There is no timer behind it and no `reach()` check on the way in: the companion's status
/// call blocks on the UI thread, and this app has one refresh path — every mutation ends in
/// [`show_detail`] — so a poll would buy nothing and could freeze the window. `agent-unavailable`
/// is deliberately not touched here: it belongs to [`ask_companion`], which is the only thing
/// that knows whether the companion answered, and a repaint must neither clear a real failure
/// nor invent one.
fn refresh_agent_rail(ui: &SnippetManagerApp) {
    let mut context: Vec<AgentContextItem> = Vec::new();
    let mut suggestions: Vec<AgentSuggestion> = Vec::new();
    if ui.get_detail_id() >= 0 {
        let language = ui.get_detail_language().to_string();
        let lines = ui.get_detail_code().lines().count();
        context.push(AgentContextItem {
            id: ui.get_detail_id().to_string().into(),
            label: ui.get_detail_title(),
            detail: if language.is_empty() {
                format!("{lines} lines").into()
            } else {
                format!("{language}, {lines} lines").into()
            },
            source: "file".into(),
        });
        suggestions.push(AgentSuggestion {
            id: "explain".into(),
            label: "What does this do?".into(),
            detail: "reads the open snippet".into(),
            icon: "spark".into(),
            running: ui.get_proposal_working(),
            proposes: false,
        });
    }
    ui.set_agent_context(ModelRc::new(VecModel::from(context)));
    ui.set_agent_suggestions(ModelRc::new(VecModel::from(suggestions)));
}

/// Put a failure where both halves of contract point 4 can see it, and hand it back to whoever
/// asked — the button's handler ignores it, the control surface returns it as a refusal.
///
/// A notice stays until the next thing that works: a repaint does not wipe it, and a successful
/// action does, because by then it is describing a world that has moved on.
fn settle<T>(ui: &SnippetManagerApp, state: &State, outcome: Result<T, String>) -> Result<T, String> {
    match outcome {
        Ok(value) => {
            set_notice(ui, state, String::new());
            Ok(value)
        }
        Err(e) => {
            tracing::warn!(error = %e, "snippet action failed");
            set_notice(ui, state, e.clone());
            Err(e)
        }
    }
}

fn set_notice(ui: &SnippetManagerApp, state: &State, text: String) {
    state.borrow_mut().notice = text.clone();
    ui.set_notice(text.into());
}

// ── The mutations, one path each ─────────────────────────────────────
//
// Every function below is the only way this app makes the change it names. The Slint callback
// and the control-surface action both call it, so an action cannot drift away from what the
// button does, and neither can report an outcome the store did not give it.
//
// None of them is slow: the control surface runs an action on the UI thread and gives it three
// seconds before telling the caller the app did not answer, and the longest thing here is one
// pretty-printed write of a few hundred kilobytes. The one genuinely slow path — asking the
// companion — is not an action at all; it is a button, and it runs on a worker thread.

/// Write whatever is in the editor back to the snippet it belongs to.
///
/// This is the fix for the fault that started all of this. The editor's four fields are `in-out`
/// and the person types straight into them; before anything moves the selection they have to be
/// committed, or the repaint underneath takes the typing with it.
fn commit_editor(ui: &SnippetManagerApp, state: &State) -> Result<(), String> {
    let id = ui.get_detail_id();
    if id < 0 {
        return Ok(());
    }
    let patch = Patch::edits(
        &ui.get_detail_title(),
        &ui.get_detail_language(),
        &ui.get_detail_code(),
        &ui.get_detail_tags(),
    );
    let mut session = state.borrow_mut();
    if session.store.get(id).is_none() {
        // It was deleted from under the editor; there is nothing to commit it to.
        return Ok(());
    }
    session.store.save(id, patch).map(|_| ())
}

/// Save the open editor and answer with the snippet as the store now holds it.
fn save_snippet(state: &State, id: i32, patch: Patch) -> Result<Snippet, String> {
    state.borrow_mut().store.save(id, patch)
}

/// Keep a new snippet and make it the one on screen.
fn new_snippet(
    ui: &SnippetManagerApp,
    state: &State,
    title: &str,
    language: &str,
    code: &str,
    tags: &str,
) -> Result<Snippet, String> {
    commit_editor(ui, state)?;
    let id = {
        let mut session = state.borrow_mut();
        // A snippet made while a collection is selected belongs to it. This is the only way a
        // custom collection gains a member: there is no move-between-collections control on this
        // screen, and the `save` action's `collection` argument is the other way in. Favourites
        // is a filter, so creating there files under no collection and starring is what puts a
        // snippet in it.
        let collection = session.collection;
        session
            .store
            .create(title, language, code, tags, collection)?
    };
    {
        // The new snippet must be visible, or "New" looks like it did nothing. Whatever was
        // narrowing the list is cleared, and the sidebar follows the snippet.
        let mut session = state.borrow_mut();
        session.selected = id;
        session.query.clear();
        session.tag.clear();
        if session.collection == FAVORITES {
            session.collection = ALL;
        }
    }
    let snippet = state
        .borrow()
        .store
        .get(id)
        .cloned()
        .ok_or_else(|| format!("snippet {id} was not stored"))?;
    Ok(snippet)
}

fn delete_snippet(state: &State, id: i32) -> Result<Snippet, String> {
    let removed = state.borrow_mut().store.delete(id)?;
    let mut session = state.borrow_mut();
    if session.selected == id {
        session.selected = -1;
    }
    Ok(removed)
}

fn toggle_favorite(state: &State, id: i32) -> Result<bool, String> {
    state.borrow_mut().store.toggle_favorite(id)
}

fn set_language(state: &State, id: i32, language: &str) -> Result<Snippet, String> {
    state.borrow_mut().store.save(
        id,
        Patch {
            language: Some(language.to_string()),
            ..Patch::default()
        },
    )
}

/// Put a snippet's code on the clipboard, and say whether the use count reached the disk.
///
/// The copy itself is the thing being asked for and it either happened or it did not; recording
/// that it happened is bookkeeping, and a bookkeeping failure must not be reported as a failed
/// copy — the code really is on the clipboard by then.
fn copy_snippet(state: &State, id: i32) -> Result<(Snippet, bool), String> {
    let snippet = state
        .borrow()
        .store
        .get(id)
        .cloned()
        .ok_or_else(|| format!("no snippet with id {id}"))?;
    if snippet.code.is_empty() {
        return Err(format!("`{}` has no code to copy", snippet.title));
    }
    copy_to_clipboard(&snippet.code)?;
    match state.borrow_mut().store.mark_used(id) {
        Ok(updated) => Ok((updated, true)),
        Err(_) => Ok((snippet, false)),
    }
}

/// Show "Copied!" on the button for a moment, from wherever the copy came from.
///
/// The timer is thread-local and lives for the life of the process, because a dropped Slint
/// timer stops and one created inside a handler is dropped the moment the handler returns. Both
/// the button and the control surface flash this same one: a window that reads `Copied!` for
/// ever because a mind copied without pressing anything is a small lie of exactly the kind this
/// app was full of.
fn flash_copied(ui: &SnippetManagerApp) {
    thread_local! {
        static TIMER: slint::Timer = slint::Timer::default();
    }
    ui.set_copy_feedback_visible(true);
    let back = ui.as_weak();
    TIMER.with(|timer| {
        timer.start(slint::TimerMode::SingleShot, COPY_FEEDBACK, move || {
            if let Some(ui) = back.upgrade() {
                ui.set_copy_feedback_visible(false);
            }
        })
    });
}

/// The clipboard, the way the rest of this OS reaches it: `wl-copy` over a pipe, the same call
/// the companion's `clipboard_write` tool and the screenshot path make.
///
/// Slint does not publish a way to set the selection from Rust — the apps that copy do it inside
/// a text widget — and this is one process on a Wayland session, so the answer is the session's
/// own clipboard tool. If it is missing or refuses, that is reported: a snippet manager that
/// says "copied" without copying is the log line this replaced.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("wl-copy could not be started ({e}); nothing was copied"))?;
    let written = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(text.as_bytes()).map_err(|e| e.to_string()),
        None => Err("wl-copy accepted no input".to_string()),
    };
    let status = child
        .wait()
        .map_err(|e| format!("wl-copy did not finish: {e}"))?;
    written.map_err(|e| format!("could not write to wl-copy: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy exited with {status}; nothing was copied"))
    }
}

/// Write every snippet to one file, and answer with where it went.
fn export_all(state: &State) -> Result<(PathBuf, usize), String> {
    let session = state.borrow();
    let dir = session
        .store
        .path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| store::dir());
    // Into the app's own directory, named by the moment: this screen has no file dialog, so the
    // only honest alternative would be no export at all. The notice names the full path.
    let path = dir.join(format!("export-{}.json", store::now()));
    let count = session.store.export_all(&path)?;
    Ok((path, count))
}

/// Take a file in as snippets. Used by the command line, the handover and the `import` action.
fn import_path(state: &State, path: &Path) -> Result<Imported, String> {
    let mut session = state.borrow_mut();
    let collection = session.collection;
    session.store.import_file(path, collection)
}

/// A name no collection is using yet.
///
/// The "+" button has no text field behind it, so it always asks for "New Collection"; the store
/// refuses a duplicate name, and a button that fails the second time it is pressed is not a
/// working button. A caller that names a collection itself still gets the refusal.
fn free_collection_name(state: &State, base: &str) -> String {
    let session = state.borrow();
    let taken = |name: &str| {
        session
            .store
            .collections()
            .iter()
            .any(|c| c.name.to_lowercase() == name.to_lowercase())
    };
    if !taken(base) {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base} {n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{base} {}", store::now())
}

// ── Wiring ───────────────────────────────────────────────────────────

fn default_languages() -> Vec<LanguageOption> {
    [
        "Rust", "Python", "JavaScript", "TypeScript", "Go", "C", "C++", "Shell", "SQL", "HTML",
        "CSS", "TOML", "YAML", "JSON", "Other",
    ]
    .iter()
    .map(|name| LanguageOption { name: (*name).into() })
    .collect()
}

fn wire(app: &SnippetManagerApp, state: &State) {
    app.set_language_options(ModelRc::new(VecModel::from(default_languages())));

    // ── Search ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_search(move |query| {
            let Some(ui) = weak.upgrade() else { return };
            st.borrow_mut().query = query.to_string();
            show_list(&ui, &st);
        });
    }

    // ── Tag chips ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_tag_filter(move |tag| {
            let Some(ui) = weak.upgrade() else { return };
            {
                let mut session = st.borrow_mut();
                // Pressing the active chip again clears it: a filter with no way off is a trap.
                if session.tag.to_lowercase() == tag.to_lowercase() {
                    session.tag.clear();
                } else {
                    session.tag = tag.to_string();
                }
            }
            show_list(&ui, &st);
        });
    }

    // ── Selection ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_select(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            // Commit first, and do not move if the commit failed: the typing is still on screen
            // where it can be retried or copied out, which is the one thing the old app took away.
            if settle(&ui, &st, commit_editor(&ui, &st)).is_err() {
                show_list(&ui, &st);
                return;
            }
            st.borrow_mut().selected = id;
            show_all(&ui, &st);
        });
    }

    // ── New ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_new(move || {
            let Some(ui) = weak.upgrade() else { return };
            let made = new_snippet(&ui, &st, "Untitled Snippet", "Rust", "", "");
            let _ = settle(&ui, &st, made);
            show_all(&ui, &st);
        });
    }

    // ── Save ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_save(move |id, title, language, code, tags| {
            let Some(ui) = weak.upgrade() else { return };
            let saved = save_snippet(&st, id, Patch::edits(&title, &language, &code, &tags));
            let _ = settle(&ui, &st, saved);
            show_all(&ui, &st);
        });
    }

    // ── Delete ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_delete(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = settle(&ui, &st, delete_snippet(&st, id));
            show_all(&ui, &st);
        });
    }

    // ── Copy ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_copy(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            // Copy takes the code from the store, so the editor goes in first: a person who
            // typed and then pressed Copy means the words in front of them, and handing back
            // the version before the typing is the same betrayal this app was fixed for. A
            // caller on the control surface gets the stored snippet untouched, because it asked
            // for the stored snippet and is not looking at the editor.
            if settle(&ui, &st, commit_editor(&ui, &st)).is_err() {
                ui.set_copy_feedback_visible(false);
                return;
            }
            match settle(&ui, &st, copy_snippet(&st, id)) {
                Ok((_, counted)) => {
                    if !counted {
                        set_notice(
                            &ui,
                            &st,
                            "The code is on the clipboard, but the use count could not be saved."
                                .to_string(),
                        );
                    }
                    flash_copied(&ui);
                }
                // Nothing was copied, so the button must not say it was.
                Err(_) => ui.set_copy_feedback_visible(false),
            }
            show_all(&ui, &st);
        });
    }

    // ── Favourite ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_toggle_favorite(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            // Starring saves one field and the repaint after it comes out of the store, so
            // whatever is in the editor goes in first or the star costs the person their typing.
            if settle(&ui, &st, commit_editor(&ui, &st)).is_err() {
                return;
            }
            let _ = settle(&ui, &st, toggle_favorite(&st, id));
            show_all(&ui, &st);
        });
    }

    // ── Language menu ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_set_language(move |language| {
            let Some(ui) = weak.upgrade() else { return };
            let id = ui.get_detail_id();
            if id < 0 {
                set_notice(&ui, &st, "Open a snippet before choosing its language.".into());
                return;
            }
            // As the favourite button: one field saved, then a repaint out of the store.
            if settle(&ui, &st, commit_editor(&ui, &st)).is_err() {
                return;
            }
            let _ = settle(&ui, &st, set_language(&st, id, &language));
            show_all(&ui, &st);
        });
    }

    // ── Collections ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_collection_select(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            st.borrow_mut().collection = id;
            show_list(&ui, &st);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_collection_create(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let name = free_collection_name(&st, name.trim());
            let made = st.borrow_mut().store.collection_create(&name);
            if let Ok(id) = settle(&ui, &st, made) {
                st.borrow_mut().collection = id;
            }
            show_list(&ui, &st);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_collection_rename(move |id, name| {
            let Some(ui) = weak.upgrade() else { return };
            let renamed = st.borrow_mut().store.collection_rename(id, &name);
            let _ = settle(&ui, &st, renamed);
            show_list(&ui, &st);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_collection_delete(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let deleted = st.borrow_mut().store.collection_delete(id);
            if let Ok(moved) = settle(&ui, &st, deleted) {
                {
                    // The sidebar cannot stay pointed at a collection that has gone.
                    let mut session = st.borrow_mut();
                    if session.collection == id {
                        session.collection = ALL;
                    }
                }
                if moved > 0 {
                    // Said out loud: a person who deletes a folder expecting its contents to go
                    // with it should not have to work out where they went.
                    set_notice(
                        &ui,
                        &st,
                        format!("{moved} snippet(s) moved back to All Snippets."),
                    );
                }
            }
            // The list and the sidebar, not the editor: nothing the editor shows belongs to a
            // collection, and repainting it here would throw away uncommitted typing.
            show_list(&ui, &st);
        });
    }

    // ── Export ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_snip_export_all(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok((path, count)) = settle(&ui, &st, export_all(&st)) {
                set_notice(
                    &ui,
                    &st,
                    format!("Exported {count} snippet(s) to {}", path.display()),
                );
            }
        });
    }

    // ── The agent layer ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_ai_explain_pressed(move || {
            let Some(ui) = weak.upgrade() else { return };
            ask_companion(&ui, &st);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_agent_suggestion_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if id == "explain" {
                // The same path the AI Explain button takes, so the two cannot come to differ.
                ask_companion(&ui, &st);
            }
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_agent_context_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            // The one context row is the open snippet; activating it selects it, which is what
            // makes the row a way back to the thing rather than a label.
            if let Ok(snippet_id) = id.parse::<i32>() {
                if settle(&ui, &st, commit_editor(&ui, &st)).is_err() {
                    return;
                }
                st.borrow_mut().selected = snippet_id;
                show_all(&ui, &st);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_proposal_applied(move || {
            // The card's verb is "Close": the companion's answer is an answer, not an edit, so
            // there is nothing to apply and both buttons put the card away.
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_proposal_dismissed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
}

/// Ask the companion about the snippet on screen.
///
/// On a worker thread, because the model can take a minute and this would otherwise be a minute
/// with a frozen window. The answer lands in the shared proposal card.
fn ask_companion(ui: &SnippetManagerApp, state: &State) {
    let snippet = {
        let session = state.borrow();
        session.store.get(session.selected).cloned()
    };
    let Some(snippet) = snippet else {
        set_notice(ui, state, "Open a snippet before asking about it.".into());
        return;
    };
    if ui.get_proposal_working() {
        return;
    }

    let prompt = format!(
        "Explain what this {} snippet does, in at most four short lines. Use only the code.\n\n{}",
        if snippet.language.is_empty() { "code" } else { &snippet.language },
        snippet.code.chars().take(4000).collect::<String>()
    );
    let title = snippet.title.clone();
    ui.set_proposal_working(true);
    ui.set_proposal(AgentProposal {
        title: format!("About “{title}”").into(),
        source: "from the open snippet".into(),
        verb: "Close".into(),
        ..Default::default()
    });

    let back = ui.as_weak();
    std::thread::spawn(move || {
        let outcome = companion::ask(&prompt);
        let _ = back.upgrade_in_event_loop(move |ui| {
            ui.set_proposal_working(false);
            match outcome {
                Ok(text) => {
                    ui.set_agent_unavailable(SharedString::new());
                    ui.set_proposal(AgentProposal {
                        title: format!("About \u{201c}{title}\u{201d}").into(),
                        body: text.into(),
                        source: "from the open snippet".into(),
                        verb: "Close".into(),
                        ..Default::default()
                    });
                }
                Err(e) => {
                    // Said where the rail says it, and not pretended into an answer. Which
                    // sentence it is depends on the failure: a shell with no model behind it is
                    // running, and "start the Yantrik shell" would send the person to start the
                    // thing already in front of them.
                    ui.set_agent_unavailable(e.hint().into());
                    ui.set_proposal(AgentProposal {
                        title: "The companion did not answer".into(),
                        body: e.to_string().into(),
                        verb: "Close".into(),
                        ..Default::default()
                    });
                }
            }
            refresh_agent_rail(&ui);
        });
    });
}

// ── The control surface ──────────────────────────────────────────────

/// Publish the window on the bus.
///
/// The id is [`APP_ID`], which is the launcher's word for this app, so `yos act snippets …` and
/// the dock's route reach the same process.
fn publish_control(app: &SnippetManagerApp, state: State) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Snippets window is gone".to_string());

    let describe_ui = ui_for.clone();
    let describe_state = state.clone();
    let describe = move || {
        let Ok(ui) = describe_ui() else { return View::new("Snippets — closed") };
        let session = describe_state.borrow();
        let total = session.store.snippets().len();
        let rows = session
            .store
            .matching(&session.query, &session.tag, session.collection);
        let open = session.store.get(session.selected).cloned();

        let summary = match (&open, session.query.trim().is_empty()) {
            (Some(s), true) => format!("Snippets — “{}”, {total} kept", s.title),
            (Some(s), false) => format!(
                "Snippets — “{}”, {} of {total} match “{}”",
                s.title,
                rows.len(),
                session.query.trim()
            ),
            (None, true) if total == 0 => "Snippets — nothing kept yet".to_string(),
            (None, true) => format!("Snippets — {total} kept, none open"),
            (None, false) => format!(
                "Snippets — {} of {total} match “{}”, none open",
                rows.len(),
                session.query.trim()
            ),
        };

        let listed: Vec<serde_json::Value> = rows
            .iter()
            .take(LISTING_CAP)
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "title": s.title,
                    "language": s.language,
                    "tags": s.tag_list(),
                    "favorite": s.favorite,
                    "collection": session.store.collection_name(s.collection),
                    "lines": s.code.lines().count(),
                    "preview": s.preview(),
                })
            })
            .collect();

        View::new(summary)
            .with("total", total as i64)
            .with("matching", rows.len() as i64)
            .with("shown", listed.len() as i64)
            .with("snippets", listed)
            .with("favorites", session.store.favorites() as i64)
            .with("query", session.query.clone())
            .with("tag", session.tag.clone())
            .with("collection", session.store.collection_name(session.collection))
            .with(
                "collections",
                session
                    .store
                    .collections()
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>(),
            )
            .with("open", open.as_ref().map(|s| s.title.clone()))
            .with("open_id", open.as_ref().map(|s| s.id as i64))
            .with("open_language", open.as_ref().map(|s| s.language.clone()))
            .with("open_favorite", open.as_ref().map(|s| s.favorite))
            .with("open_lines", open.as_ref().map(|s| s.code.lines().count() as i64))
            .with("store", session.store.path().display().to_string())
            .with("copied_recently", ui.get_copy_feedback_visible())
            .with("notice", session.notice.clone())
    };

    let new_ui = ui_for.clone();
    let new_state = state.clone();
    let save_ui = ui_for.clone();
    let save_state = state.clone();
    let open_ui = ui_for.clone();
    let open_state = state.clone();
    let search_ui = ui_for.clone();
    let search_state = state.clone();
    let copy_ui = ui_for.clone();
    let copy_state = state.clone();
    let delete_ui = ui_for.clone();
    let delete_state = state.clone();
    let favorite_ui = ui_for.clone();
    let favorite_state = state.clone();
    let import_ui = ui_for.clone();
    let import_state = state.clone();
    let show_ui = ui_for;

    App::new(APP_ID)
        .describe(describe)
        .action(
            // Standard: it writes this app's own store and nothing else on the machine changes.
            Action::new("new", "Keep a new snippet")
                .arg(Param::text("title"))
                .arg(Param::text("language").describe("Rust, Python, Shell, …"))
                .arg(Param::text("code").describe("The snippet itself"))
                .arg(Param::text("tags").describe("Comma-separated").optional())
                .risk("standard"),
            move |args| {
                let ui = new_ui()?;
                let title = arg(args, "title");
                let code = arg(args, "code");
                if title.trim().is_empty() {
                    return Err("`title` is empty".into());
                }
                if code.is_empty() {
                    return Err("`code` is empty; there is nothing to keep".into());
                }
                let language = {
                    let given = arg(args, "language");
                    if given.trim().is_empty() { "Other".to_string() } else { given }
                };
                let made = new_snippet(&ui, &new_state, &title, &language, &code, &arg(args, "tags"));
                let snippet = settle(&ui, &new_state, made)?;
                show_all(&ui, &new_state);
                Ok(stored_answer(&new_state, &snippet))
            },
        )
        .action(
            Action::new("save", "Change a snippet that is already kept")
                .arg(Param::text("id").describe("Its id, or its title"))
                .arg(Param::text("title").optional())
                .arg(Param::text("language").optional())
                .arg(Param::text("code").optional())
                .arg(Param::text("tags").optional())
                .arg(Param::flag("favorite").optional())
                .arg(
                    Param::text("collection")
                        .describe("A collection by name or id; empty files it under none")
                        .optional(),
                )
                .risk("standard"),
            move |args| {
                let ui = save_ui()?;
                let id = named(&save_state, &arg(args, "id"))?;
                // Whatever the person has typed is committed before the caller's change lands on
                // top of it: this action ends in a repaint out of the store, and a mind saving a
                // snippet must not silently take somebody's unfinished edit with it.
                settle(&ui, &save_state, commit_editor(&ui, &save_state))?;
                let mut patch = Patch {
                    title: optional(args, "title"),
                    language: optional(args, "language"),
                    code: optional(args, "code"),
                    tags: optional(args, "tags"),
                    favorite: args["favorite"].as_bool(),
                    collection: None,
                };
                if let Some(wanted) = optional(args, "collection") {
                    patch.collection = Some(collection_named(&save_state, &wanted)?);
                }
                if patch.is_empty() {
                    return Err("nothing to change: give at least one of title, language, code, \
                                tags, favorite or collection"
                        .into());
                }
                let saved = settle(&ui, &save_state, save_snippet(&save_state, id, patch))?;
                show_all(&ui, &save_state);
                Ok(stored_answer(&save_state, &saved))
            },
        )
        .action(
            // Safe: it changes what is on screen and nothing that is stored.
            Action::new("open", "Show a snippet in the editor")
                .arg(Param::text("id").describe("Its id, or its title"))
                .risk("safe"),
            move |args| {
                let ui = open_ui()?;
                let id = named(&open_state, &arg(args, "id"))?;
                settle(&ui, &open_state, commit_editor(&ui, &open_state))?;
                open_state.borrow_mut().selected = id;
                show_all(&ui, &open_state);
                ui.window().set_minimized(false);
                let snippet = open_state
                    .borrow()
                    .store
                    .get(id)
                    .cloned()
                    .ok_or_else(|| format!("no snippet with id {id}"))?;
                Ok(stored_answer(&open_state, &snippet))
            },
        )
        .action(
            Action::new("search", "Narrow the list to what matches")
                .arg(Param::text("query").describe("Matched against title, code, tags and language"))
                .risk("safe"),
            move |args| {
                let ui = search_ui()?;
                let query = arg(args, "query");
                search_state.borrow_mut().query = query.clone();
                show_list(&ui, &search_state);
                let session = search_state.borrow();
                let rows = session
                    .store
                    .matching(&session.query, &session.tag, session.collection);
                Ok(serde_json::json!({
                    "query": query,
                    "matching": rows.len(),
                    "shown": rows.len().min(LISTING_CAP),
                    "of": session.store.snippets().len(),
                    "titles": rows.iter().take(LISTING_CAP).map(|s| s.title.clone()).collect::<Vec<_>>(),
                }))
            },
        )
        .action(
            // Standard: the clipboard is readable by everything else on the session, so this is
            // data leaving the app — reversible, but not nothing.
            Action::new("copy", "Put a snippet's code on the clipboard")
                .arg(Param::text("id").describe("Its id, or its title"))
                .risk("standard"),
            move |args| {
                let ui = copy_ui()?;
                let id = named(&copy_state, &arg(args, "id"))?;
                let (snippet, counted) = settle(&ui, &copy_state, copy_snippet(&copy_state, id))?;
                flash_copied(&ui);
                show_all(&ui, &copy_state);
                Ok(serde_json::json!({
                    "copied": snippet.title,
                    "id": snippet.id,
                    "bytes": snippet.code.len(),
                    "with": "wl-copy",
                    "use_count": snippet.use_count,
                    "use_count_saved": counted,
                }))
            },
        )
        .action(
            // Sensitive, not standard: this destroys something the person wrote and there is no
            // trash to take it out of. Container Manager grades `remove` the same way for the
            // same reason — the recoverable and the unrecoverable are not one grade.
            Action::new("delete", "Throw a snippet away")
                .arg(Param::text("id").describe("Its id, or its title"))
                .risk("sensitive"),
            move |args| {
                let ui = delete_ui()?;
                let id = named(&delete_state, &arg(args, "id"))?;
                let removed = settle(&ui, &delete_state, delete_snippet(&delete_state, id))?;
                show_all(&ui, &delete_state);
                let session = delete_state.borrow();
                Ok(serde_json::json!({
                    "deleted": removed.title,
                    "id": removed.id,
                    // Read back rather than assumed: the answer is what the store says now.
                    "still_present": session.store.get(id).is_some(),
                    "remaining": session.store.snippets().len(),
                }))
            },
        )
        .action(
            Action::new("toggle_favorite", "Star or unstar a snippet")
                .arg(Param::text("id").describe("Its id, or its title"))
                .risk("standard"),
            move |args| {
                let ui = favorite_ui()?;
                let id = named(&favorite_state, &arg(args, "id"))?;
                settle(&ui, &favorite_state, commit_editor(&ui, &favorite_state))?;
                settle(&ui, &favorite_state, toggle_favorite(&favorite_state, id))?;
                show_all(&ui, &favorite_state);
                let session = favorite_state.borrow();
                let snippet = session
                    .store
                    .get(id)
                    .ok_or_else(|| format!("no snippet with id {id}"))?;
                Ok(serde_json::json!({
                    "id": snippet.id,
                    "title": snippet.title,
                    "favorite": snippet.favorite,
                    "favorites": session.store.favorites(),
                }))
            },
        )
        .action(
            Action::new("import", "Keep a file as a snippet, or read back an export")
                .arg(Param::text("path").describe("A source file, or a file `export` wrote"))
                .risk("standard"),
            move |args| {
                let ui = import_ui()?;
                let raw = arg(args, "path");
                if raw.trim().is_empty() {
                    return Err("`path` is empty".into());
                }
                let path = expanded(raw.trim());
                if !path.is_file() {
                    return Err(format!("no file at {}", path.display()));
                }
                settle(&ui, &import_state, commit_editor(&ui, &import_state))?;
                let imported = settle(&ui, &import_state, import_path(&import_state, &path))?;
                if let Some(id) = imported.ids.first() {
                    import_state.borrow_mut().selected = *id;
                }
                show_all(&ui, &import_state);
                ui.window().set_minimized(false);
                let session = import_state.borrow();
                Ok(serde_json::json!({
                    "imported": imported.ids.len(),
                    "as": imported.kind,
                    "ids": imported.ids,
                    "titles": imported
                        .ids
                        .iter()
                        .filter_map(|id| session.store.get(*id).map(|s| s.title.clone()))
                        .collect::<Vec<_>>(),
                    "total": session.store.snippets().len(),
                }))
            },
        )
        .action(
            Action::new("show", "Bring the window forward").risk("safe"),
            move |_| {
                let ui = show_ui()?;
                ui.window().set_minimized(false);
                Ok(serde_json::json!({ "showing": ui.get_detail_title().to_string() }))
            },
        )
        .serve();
}

/// One string argument, trimmed of nothing: code keeps its whitespace.
fn arg(args: &serde_json::Value, key: &str) -> String {
    args[key].as_str().unwrap_or_default().to_string()
}

/// An argument that was actually given, as against one left out.
fn optional(args: &serde_json::Value, key: &str) -> Option<String> {
    args[key].as_str().map(|s| s.to_string())
}

/// The id of the snippet a caller named, or a refusal saying why not.
fn named(state: &State, needle: &str) -> Result<i32, String> {
    state.borrow().store.find(needle).map(|s| s.id)
}

/// The id of the collection a caller named, by id or by name.
fn collection_named(state: &State, wanted: &str) -> Result<i32, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Ok(ALL);
    }
    let session = state.borrow();
    if let Ok(id) = wanted.parse::<i32>() {
        if id == ALL || session.store.collections().iter().any(|c| c.id == id) {
            return Ok(id);
        }
    }
    session
        .store
        .collections()
        .iter()
        .find(|c| c.name.to_lowercase() == wanted.to_lowercase())
        .map(|c| c.id)
        .ok_or_else(|| {
            let known: Vec<String> = session
                .store
                .collections()
                .iter()
                .map(|c| c.name.clone())
                .collect();
            if known.is_empty() {
                format!("no collection called `{wanted}`; this store has none yet")
            } else {
                format!("no collection called `{wanted}`; there is {}", known.join(", "))
            }
        })
}

/// What an action answers with: the snippet as the store holds it, read back after the write.
fn stored_answer(state: &State, snippet: &Snippet) -> serde_json::Value {
    let session = state.borrow();
    let stored = session.store.get(snippet.id);
    serde_json::json!({
        "id": snippet.id,
        "title": stored.map(|s| s.title.clone()),
        "language": stored.map(|s| s.language.clone()),
        "tags": stored.map(|s| s.tag_list()),
        "favorite": stored.map(|s| s.favorite),
        "collection": stored.map(|s| session.store.collection_name(s.collection)),
        "bytes": stored.map(|s| s.code.len()),
        "stored_in": session.store.path().display().to_string(),
        "total": session.store.snippets().len(),
    })
}

/// `~/x` means what it does in a shell; anything else is taken as given.
fn expanded(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
    } else {
        PathBuf::from(value)
    }
}
