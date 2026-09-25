//! Yantrik Presentation — standalone app binary.
//!
//! What was wrong with it. The slide model was the best-built thing in the office trio — add,
//! delete, duplicate, move and select all committed the canvas before they moved and reloaded
//! it after — and the two buttons a person presses most did neither. `on_next_slide` and
//! `on_prev_slide` changed `current-slide-index` and stopped there, so the index said slide 2
//! while the canvas still held slide 1's text, and the next thing that committed wrote slide 1
//! over slide 2. Typing, pressing Next and clicking a thumbnail destroyed a slide.
//!
//! The fix is not two more calls to `commit_current()`. There is one [`go_to`], it commits and
//! then moves and then draws, and every navigation path in the app — the buttons, the
//! thumbnails, the keyboard, the presenter view's click zones and every action on the control
//! surface — goes through it. A navigation path that forgets is no longer something that can
//! be written.
//!
//! Save and Load were log lines, so a deck existed only while the window was open. A deck is a
//! versioned `.ydeck` file now, it can be handed in on the command line or over the control
//! surface, and unsaved work is written to a recovery file that the next start offers back.
//! The arithmetic of all of that is in `deck.rs`, with no Slint in it, so it can be tested.

mod deck;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use deck::{Deck, Edits, Slide, THEMES};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

/// How long after the last keystroke the draft is written.
///
/// Long enough that typing a sentence is one write rather than forty, short enough that the
/// most a crash can cost is the word being typed.
const RECOVERY_DEBOUNCE: Duration = Duration::from_millis(750);

/// How many slide titles `describe` will list before it starts counting instead.
const OUTLINE_CAP: usize = 100;

// ── State ────────────────────────────────────────────────────────────

/// The presenter's clock.
///
/// Held apart from [`App`] because the timer that drives it must not own the state the rest of
/// the window borrows: the timer lives in `main` (a dropped Slint timer stops, which is how
/// System Monitor came to show one reading for the life of its window) and its closure holds
/// only this and a weak window handle.
#[derive(Default)]
struct Clock {
    running: Cell<bool>,
    seconds: Cell<i64>,
}

struct App {
    deck: Deck,
    /// The rows the sidebar draws. Updated in place by [`render`] when the shape has not
    /// changed, so moving the selection does not scroll the rail back to the top.
    rows: Rc<VecModel<SlideData>>,
    /// The slides the current search matched, and which of them is selected.
    hits: Vec<usize>,
    hit_at: usize,
    clock: Rc<Clock>,
    /// Where the draft goes, or `None` when this session must not write one — a recovery file
    /// that could not be read is left exactly as it is rather than overwritten with this
    /// window's draft, which is the same choice Text Editor makes.
    recovery: Option<PathBuf>,
}

type Shared = Rc<RefCell<App>>;

// ── Saying what happened ─────────────────────────────────────────────

/// One path behind every button and every action: the outcome, or the reason there is none.
///
/// `Err` goes on the strip under the header and into `describe.notice`, so a failure is said
/// twice — once to whoever is looking at the window and once to whoever is driving it. `Ok`
/// clears it, so a notice on screen is always about the last thing that happened rather than
/// about something three actions ago.
fn settle<T>(ui: &PresentationApp, outcome: Result<T, String>) -> Option<T> {
    match outcome {
        Ok(value) => {
            ui.set_notice(SharedString::new());
            Some(value)
        }
        Err(message) => {
            tracing::warn!(error = %message, "yPresent could not do that");
            ui.set_notice(message.into());
            None
        }
    }
}

/// Something to say that is not a failure: the recovery notice, "nothing to undo".
fn say(ui: &PresentationApp, message: &str) {
    ui.set_notice(message.into());
}

// ── The canvas, and the one way anything reaches the screen ──────────

/// What is on the canvas right now.
///
/// `presentation.slint` binds `text <=> current-title` (and body, and notes) two-way, so these
/// four properties *are* the live text and the `*-edited` callbacks are only the notification
/// that they changed. This is the value every commit is made from.
fn canvas(ui: &PresentationApp) -> Edits {
    Edits {
        title: ui.get_current_title().to_string(),
        body: ui.get_current_body().to_string(),
        notes: ui.get_current_notes().to_string(),
        layout: ui.get_current_layout(),
    }
}

fn row_of(slide: &Slide) -> SlideData {
    SlideData {
        title: slide.title.clone().into(),
        body: slide.body.clone().into(),
        notes: slide.notes.clone().into(),
        layout: slide.layout,
    }
}

fn brush(rgb: (u8, u8, u8)) -> slint::Brush {
    slint::Brush::SolidColor(slint::Color::from_rgb_u8(rgb.0, rgb.1, rgb.2))
}

/// One of `deck::THEMES` in the shape the screen draws it.
fn present_theme(index: usize, active: bool) -> PresentTheme {
    let spec = deck::theme_spec(index);
    PresentTheme {
        name: spec.name.into(),
        bg_color: brush(spec.bg),
        text_color: brush(spec.text),
        accent_color: brush(spec.accent),
        is_active: active,
    }
}

fn clock_text(seconds: i64) -> String {
    let s = seconds.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// The one render path. Every callback and every action ends here.
///
/// `canvas` says whether the slide's text is reloaded onto the editor as well. It is false on
/// the typing path and only there: reloading a TextInput the person is typing into would put
/// their cursor back at the start of every word. Everything else — every navigation, every
/// structural change, every load — passes true, and that is the half Next and Previous used to
/// skip.
fn render(ui: &PresentationApp, state: &Shared, canvas: bool) {
    let s = state.borrow();
    let d = &s.deck;
    let count = d.len();
    let at = d.current();

    if s.rows.row_count() == count {
        for (i, slide) in d.slides().iter().enumerate() {
            let row = row_of(slide);
            let same = s.rows.row_data(i).is_some_and(|old| {
                old.title == row.title
                    && old.body == row.body
                    && old.notes == row.notes
                    && old.layout == row.layout
            });
            if !same {
                s.rows.set_row_data(i, row);
            }
        }
    } else {
        s.rows.set_vec(d.slides().iter().map(row_of).collect::<Vec<_>>());
    }

    ui.set_slide_count(count as i32);
    ui.set_current_slide_index(at as i32);
    ui.set_slide_progress(format!("Slide {} of {}", at + 1, count).into());
    ui.set_presentation_title(d.title().into());
    ui.set_file_path(
        d.path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    ui.set_is_modified(d.dirty());
    ui.set_save_status(
        match (d.path.is_some(), d.dirty()) {
            (false, _) => "Not saved yet",
            (true, true) => "Unsaved changes",
            (true, false) => "Saved",
        }
        .into(),
    );

    if canvas {
        let slide = d.current_slide();
        ui.set_current_title(slide.title.clone().into());
        ui.set_current_body(slide.body.clone().into());
        ui.set_current_notes(slide.notes.clone().into());
        ui.set_current_layout(slide.layout);
    }

    // The presenter view's "Next Slide" card. It said nothing at all before: both properties
    // were declared, drawn, and never written.
    let next = d.slide(at + 1);
    ui.set_next_title(
        next.map(|s| s.title.clone())
            .unwrap_or_else(|| "End of deck".to_string())
            .into(),
    );
    ui.set_next_body(next.map(|s| s.body.clone()).unwrap_or_default().into());

    // The screen reads `themes[0]` everywhere it draws a slide, so it is given the one that is
    // chosen; the gallery is given all of them, each in its own colours.
    let chosen = d.theme();
    ui.set_current_theme(chosen as i32);
    ui.set_themes(ModelRc::new(VecModel::from(vec![present_theme(chosen, true)])));
    ui.set_theme_swatches(ModelRc::new(VecModel::from(
        (0..THEMES.len()).map(|i| present_theme(i, i == chosen)).collect::<Vec<_>>(),
    )));

    ui.set_search_count(s.hits.len() as i32);
    ui.set_timer_text(clock_text(s.clock.seconds.get()).into());
}

/// The one way the selection moves: commit what is on the canvas, move, draw the slide moved to.
fn go_to(ui: &PresentationApp, state: &Shared, index: i64) -> usize {
    let at = state.borrow_mut().deck.go_to(&canvas(ui), index);
    render(ui, state, true);
    at
}

// ── Keeping it ───────────────────────────────────────────────────────

/// Write the draft now, or clear it if there is nothing unsaved.
fn checkpoint(state: &Shared) -> Result<(), String> {
    let (path, draft) = {
        let s = state.borrow();
        let Some(path) = s.recovery.clone() else { return Ok(()) };
        (path, s.deck.draft())
    };
    deck::write_recovery(&path, draft.as_ref())
}

/// Restart the debounce. The closure holds a snapshot and a path and nothing else, so it cannot
/// be waiting on a borrow of the window when it fires.
fn arm_recovery(state: &Shared, timer: &Rc<slint::Timer>) {
    let (path, draft) = {
        let s = state.borrow();
        let Some(path) = s.recovery.clone() else { return };
        (path, s.deck.draft())
    };
    timer.start(slint::TimerMode::SingleShot, RECOVERY_DEBOUNCE, move || {
        if let Err(e) = deck::write_recovery(&path, draft.as_ref()) {
            // Not on the notice strip: a failing draft write is not a failure of the thing the
            // person just did, and overwriting what they are looking at with it would bury the
            // real message. `describe.recovery` carries it for a mind.
            tracing::warn!(error = %e, "could not write the recovery draft");
        }
    });
}

/// Write the deck, to `to` or to the home it already has.
///
/// The canvas is committed first, through the same path everything else uses: Save has to write
/// what is on screen, not what was on screen when the selection last moved.
fn save(ui: &PresentationApp, state: &Shared, to: Option<PathBuf>) -> Result<PathBuf, String> {
    state.borrow_mut().deck.commit(&canvas(ui));
    let path = match to {
        Some(p) => p,
        None => state.borrow().deck.destination()?,
    };
    state.borrow_mut().deck.save_to(&path)?;
    // The deck is on disk, so the draft is no longer the newest copy of anything.
    let _ = checkpoint(state);
    render(ui, state, false);
    Ok(path)
}

/// Make sure nothing unsaved is about to be dropped on the floor.
///
/// Open, New and a path on the command line all replace the deck in the window. A deck with
/// unsaved changes is written to its home first — the one it has, or a fresh collision-safe one
/// under `~/Documents/Presentations` — and the caller is told where it went. If that write
/// fails the replacement does not happen at all: losing the work is the one outcome that is not
/// allowed.
fn secure_current(ui: &PresentationApp, state: &Shared) -> Result<Option<PathBuf>, String> {
    if !state.borrow().deck.dirty() {
        return Ok(None);
    }
    save(ui, state, None).map(Some)
}

/// Read a deck off disk and put it in the window.
///
/// The file is loaded *before* anything is displaced, so a path that is not a deck costs
/// nothing: the deck on screen is untouched and the reason is named. That is what "never
/// half-load" means from this side.
fn open_path(ui: &PresentationApp, state: &Shared, path: &Path) -> Result<Option<PathBuf>, String> {
    let opened = Deck::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let rescued = secure_current(ui, state)?;
    {
        let mut s = state.borrow_mut();
        s.deck = opened;
        s.hits.clear();
        s.hit_at = 0;
    }
    let _ = checkpoint(state);
    render(ui, state, true);
    Ok(rescued)
}

fn expanded(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
    } else {
        PathBuf::from(value)
    }
}

/// The most recently changed deck in the default folder.
///
/// This is what the ribbon's Open button opens. The screen has no file dialog and this app is
/// not the place to grow one; a mind or a file browser hands a path in through `open`, and a
/// person at the keyboard almost always means "the one I was just working on".
fn most_recent_deck() -> Result<PathBuf, String> {
    let dir = deck::default_deck_dir();
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some(deck::EXTENSION) {
            continue;
        }
        let Ok(when) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if best.as_ref().is_none_or(|(best_when, _)| when > *best_when) {
            best = Some((when, path));
        }
    }
    best.map(|(_, p)| p)
        .ok_or_else(|| format!("there are no decks in {} yet", dir.display()))
}

// ── Start ────────────────────────────────────────────────────────────

/// What the window opens with, and what to say about how it got there.
///
/// Three things can be true at once: a path on the command line, a draft left by a window that
/// closed with unsaved work, and neither. The rule is that unsaved work is never lost and never
/// silently overwritten — a draft for some *other* deck is saved to a file of its own before the
/// requested one is opened, and if that cannot be done this session writes no draft at all
/// rather than overwriting the one already there.
fn start(path: Option<PathBuf>, recovery: &Path) -> (Deck, Option<PathBuf>, String) {
    let autosave = Some(recovery.to_path_buf());

    let draft = match deck::recover(recovery) {
        Ok(d) => d,
        Err(e) => {
            let deck = match &path {
                Some(p) => Deck::open(p).unwrap_or_else(|_| Deck::blank("Untitled")),
                None => Deck::blank("Untitled"),
            };
            return (
                deck,
                // Left exactly as it is: a draft that cannot be read is the case where a person
                // most needs it kept, and this session overwriting it would be the loss.
                None,
                format!(
                    "The recovery file at {} could not be read ({e}). It has been left alone, \
                     and this window will not write over it.",
                    recovery.display()
                ),
            );
        }
    };

    match (path, draft) {
        (None, None) => (Deck::blank("Untitled"), autosave, String::new()),

        (None, Some(d)) => {
            let where_from = d
                .path
                .as_ref()
                .map(|p| format!(" to {}", p.display()))
                .unwrap_or_default();
            (
                Deck::from_recovery(d),
                autosave,
                format!("Recovered unsaved changes{where_from}. Save keeps them."),
            )
        }

        (Some(p), None) => match Deck::open(&p) {
            Ok(deck) => (deck, autosave, String::new()),
            Err(e) => (
                Deck::blank("Untitled"),
                autosave,
                format!("{}: {e}", p.display()),
            ),
        },

        (Some(p), Some(d)) if d.path.as_deref() == Some(p.as_path()) => {
            // The draft is a newer version of the very deck that was asked for.
            (
                Deck::from_recovery(d),
                autosave,
                format!("Recovered unsaved changes to {}. Save keeps them.", p.display()),
            )
        }

        (Some(p), Some(d)) => {
            // The draft is somebody else's work. Give it a home of its own before this window
            // takes the recovery slot over.
            let mut rescue = Deck::from_recovery(d);
            let rescued = rescue.destination().and_then(|home| {
                rescue.save_to(&home)?;
                Ok(home)
            });
            let (autosave, mut notice) = match rescued {
                Ok(home) => {
                    let _ = deck::write_recovery(recovery, None);
                    (
                        autosave,
                        format!("An unsaved deck from a previous session was saved to {}. ", home.display()),
                    )
                }
                Err(e) => (
                    None,
                    format!(
                        "An unsaved deck from a previous session is still in {} and could not be \
                         saved ({e}), so this window will not write a recovery draft. ",
                        recovery.display()
                    ),
                ),
            };
            match Deck::open(&p) {
                Ok(deck) => (deck, autosave, notice.trim_end().to_string()),
                Err(e) => {
                    notice.push_str(&format!("{}: {e}", p.display()));
                    (Deck::blank("Untitled"), autosave, notice)
                }
            }
        }
    }
}

fn main() {
    init_tracing("yantrik-presentation");

    let path = std::env::args_os().nth(1).map(PathBuf::from);

    // One window per app. A second launch hands its file to the running one rather than opening
    // a second window over the first — the deck a person is editing is in the first one.
    let Some(_instance) = instance::claim("presentation") else {
        let request = match &path {
            Some(p) => serde_json::json!({"action": "open", "args": {"path": p}}),
            None => serde_json::json!({"action": "show", "args": {}}),
        };
        let client =
            SyncRpcClient::for_service("app-presentation").with_timeout(Duration::from_secs(3));
        for _ in 0..20 {
            if let Ok(reply) = client.call("app.act", request.clone()) {
                if reply["accepted"] != true {
                    eprintln!("yPresent declined the request: {reply}");
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("yPresent is already starting; try opening the deck again.");
        return;
    };

    let app = PresentationApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    let recovery_path = deck::recovery_path();
    let (deck, recovery, notice) = start(path, &recovery_path);

    let state: Shared = Rc::new(RefCell::new(App {
        deck,
        rows: Rc::new(VecModel::default()),
        hits: Vec::new(),
        hit_at: 0,
        clock: Rc::new(Clock::default()),
        recovery,
    }));

    // Both timers are held here for the life of the window. A Slint timer bound inside `wire`
    // is dropped when `wire` returns and a dropped timer stops — that is verbatim the bug that
    // made System Monitor show one reading for as long as it was open.
    let recovery_timer = Rc::new(slint::Timer::default());
    let _presenter_timer = wire(&app, &state, &recovery_timer);

    publish_control(&app, state.clone());

    // Once, and only here: `render` updates the rows in place, and handing the screen a new
    // ModelRc on every render would scroll the slide rail back to the top each time the
    // selection moved.
    app.set_slides(ModelRc::from(state.borrow().rows.clone()));
    render(&app, &state, true);
    if !notice.is_empty() {
        say(&app, &notice);
    }

    run_until_closed(&app, "yantrik-presentation");

    // One last draft after the event loop has stopped, so work that was typed inside the
    // debounce window is still offered back on the next start.
    if let Err(e) = checkpoint(&state) {
        tracing::warn!(error = %e, "could not write the final recovery draft");
    }
}

fn wire(app: &PresentationApp, state: &Shared, recovery: &Rc<slint::Timer>) -> slint::Timer {
    // ── The deck ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_add_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            let next = st.borrow().deck.len() + 1;
            let outcome = st.borrow_mut().deck.add_slide(
                &canvas(&ui),
                None,
                Slide::new(format!("Slide {next}"), "", 1),
            );
            settle(&ui, outcome);
            render(&ui, &st, true);
            arm_recovery(&st, &timer);
        });
    }

    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_delete_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            // Commit first: deleting slide 3 must not also throw away the sentence being typed
            // into slide 2.
            st.borrow_mut().deck.commit(&canvas(&ui));
            let at = st.borrow().deck.current();
            let outcome = st.borrow_mut().deck.delete_slide(at);
            settle(&ui, outcome);
            render(&ui, &st, true);
            arm_recovery(&st, &timer);
        });
    }

    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_duplicate_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = st.borrow_mut().deck.duplicate(&canvas(&ui));
            settle(&ui, outcome);
            render(&ui, &st, true);
            arm_recovery(&st, &timer);
        });
    }

    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_move_slide_up(move || {
            let Some(ui) = weak.upgrade() else { return };
            st.borrow_mut().deck.commit(&canvas(&ui));
            let at = st.borrow().deck.current();
            if at > 0 {
                let outcome = st.borrow_mut().deck.move_slide(at, at - 1);
                settle(&ui, outcome);
            }
            render(&ui, &st, true);
            arm_recovery(&st, &timer);
        });
    }

    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_move_slide_down(move || {
            let Some(ui) = weak.upgrade() else { return };
            st.borrow_mut().deck.commit(&canvas(&ui));
            let (at, len) = {
                let s = st.borrow();
                (s.deck.current(), s.deck.len())
            };
            if at + 1 < len {
                let outcome = st.borrow_mut().deck.move_slide(at, at + 1);
                settle(&ui, outcome);
            }
            render(&ui, &st, true);
            arm_recovery(&st, &timer);
        });
    }

    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_select_slide(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            go_to(&ui, &st, idx as i64);
        });
    }

    // ── Navigation ──
    //
    // These two are the whole reason `go_to` exists. They used to move the index and leave the
    // canvas showing the previous slide's text, so the next commit — from any of the six
    // handlers above, all of which were correct — wrote it into the wrong slide.
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_next_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            let at = st.borrow().deck.current() as i64;
            go_to(&ui, &st, at + 1);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_prev_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            let at = st.borrow().deck.current() as i64;
            go_to(&ui, &st, at - 1);
        });
    }

    // ── Keyboard ──
    //
    // The focus scope in presentation.slint named the key; this decides what it means, and the
    // four navigation keys end in the same `go_to` every button does. The scope was 0x0 and
    // nothing ever focused it, so Present mode — the point of the app — could only be driven
    // with a mouse.
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_key_pressed(move |key| {
            let Some(ui) = weak.upgrade() else { return };
            let last = st.borrow().deck.len().saturating_sub(1) as i64;
            let at = st.borrow().deck.current() as i64;
            match key.as_str() {
                "next" => {
                    go_to(&ui, &st, at + 1);
                }
                "previous" => {
                    go_to(&ui, &st, at - 1);
                }
                "first" => {
                    go_to(&ui, &st, 0);
                }
                "last" => {
                    go_to(&ui, &st, last);
                }
                "escape" => {
                    ui.set_show_search(false);
                    ui.set_pres_template_gallery_open(false);
                    if ui.get_is_presenting() {
                        ui.set_is_presenting(false);
                        render(&ui, &st, true);
                    }
                }
                other => tracing::warn!(key = other, "unknown key name from the screen"),
            }
        });
    }

    // ── Editing ──
    //
    // The three of these were empty, and the lint was right to call them dead even though the
    // two-way binding means the text really does reach `current-*`. What was missing is
    // everything that has to happen *because* it did: the slide in the model catches up, the
    // window starts saying it has unsaved changes, and the draft is armed. The canvas is not
    // reloaded — that is the one path where it must not be, or the cursor would jump to the
    // start of the field on every keystroke.
    for_each_edit(app, state, recovery);

    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_set_layout(move |layout| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_current_layout(layout);
            st.borrow_mut().deck.commit(&canvas(&ui));
            render(&ui, &st, false);
            arm_recovery(&st, &timer);
        });
    }

    // ── Theme ──
    //
    // This was `tracing::info!("Set theme {idx}")`, which is why the Design tab was decorative
    // and why every slide drew in the shell's fallback colours: `themes` was an `in` property
    // nothing ever wrote. The chosen theme is part of the deck and is saved with it.
    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_set_theme(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            st.borrow_mut().deck.set_theme(idx.max(0) as usize);
            render(&ui, &st, false);
            arm_recovery(&st, &timer);
        });
    }
    {
        let weak = app.as_weak();
        app.on_pres_open_template_gallery(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_pres_template_gallery_open(true);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_pres_select_template(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            st.borrow_mut().deck.set_theme(idx.max(0) as usize);
            render(&ui, &st, false);
            arm_recovery(&st, &timer);
        });
    }

    // ── Present ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_toggle_present(move || {
            let Some(ui) = weak.upgrade() else { return };
            // Through `go_to` so entering and leaving Present both commit the canvas; the
            // presenter view reads the model, not the editor's properties.
            let at = st.borrow().deck.current() as i64;
            go_to(&ui, &st, at);
            ui.set_is_presenting(!ui.get_is_presenting());
        });
    }

    // ── Save / Open ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_save_presentation(move || {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = save(&ui, &st, None);
            settle(&ui, outcome);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_load_presentation(move || {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = most_recent_deck().and_then(|p| open_path(&ui, &st, &p));
            if let Some(rescued) = settle(&ui, outcome) {
                if let Some(home) = rescued {
                    say(&ui, &format!("The deck you were editing was saved to {}.", home.display()));
                }
            }
        });
    }

    // ── Export ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_export_markdown(move || {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = export(&ui, &st, "md", None);
            if let Some(path) = settle(&ui, outcome) {
                say(&ui, &format!("Exported to {}.", path.display()));
            }
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_export_outline(move || {
            let Some(ui) = weak.upgrade() else { return };
            let outcome = export(&ui, &st, "outline.md", None);
            if let Some(path) = settle(&ui, outcome) {
                say(&ui, &format!("Exported to {}.", path.display()));
            }
        });
    }

    // ── Search ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_search_slides(move |query| {
            let Some(ui) = weak.upgrade() else { return };
            search(&ui, &st, query.as_str());
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_search_next(move || {
            let Some(ui) = weak.upgrade() else { return };
            let next = {
                let mut s = st.borrow_mut();
                if s.hits.is_empty() {
                    None
                } else {
                    s.hit_at = (s.hit_at + 1) % s.hits.len();
                    Some(s.hits[s.hit_at])
                }
            };
            match next {
                Some(at) => {
                    go_to(&ui, &st, at as i64);
                    ui.set_notice(SharedString::new());
                }
                None => say(&ui, "Nothing to step through; search first."),
            }
        });
    }

    // ── Undo / Redo ──
    //
    // Over the deck's content, not over the keystrokes: a step back is the deck as it was
    // before the last thing that changed it, which is the unit a person means by "undo" in a
    // deck editor. The canvas is committed first, so the run of typing you just did is the
    // thing that comes back.
    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_undo(move || {
            let Some(ui) = weak.upgrade() else { return };
            step_history(&ui, &st, &timer, false);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        let timer = recovery.clone();
        app.on_redo(move || {
            let Some(ui) = weak.upgrade() else { return };
            step_history(&ui, &st, &timer, true);
        });
    }

    // ── Speaker notes, from the companion ──
    //
    // The one AI control that survives, and the reason it does: the answer has somewhere to
    // land that is not the deck itself, and it lands there unsaved so it is read before it is
    // kept — the same rule Notes and Document Editor follow. The other thirteen offered to
    // rewrite or replace slides through a proposal panel nothing produced; they are off the
    // screen, with the reason at the top of presentation.slint.
    {
        let weak = app.as_weak();
        app.on_generate_notes(move || {
            let Some(ui) = weak.upgrade() else { return };
            let slide = canvas(&ui);
            if slide.title.trim().is_empty() && slide.body.trim().is_empty() {
                say(&ui, "This slide has no text to write notes from.");
                return;
            }
            say(&ui, "Asking the companion for speaker notes…");

            let ask = format!(
                "Write speaker notes for one slide of a talk. Use only what the slide says; \
                 invent nothing. Three or four sentences of prose, no heading and no bullet \
                 markers.\n\nTitle: {}\n\nBody:\n{}",
                slide.title, slide.body
            );
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&ask);
                // Only the window can be touched from here: `upgrade_in_event_loop` needs a
                // Send closure and the app's state is an Rc. The answer goes onto the canvas
                // and then through `notes-edited`, which is the same path a person typing into
                // the notes field takes — so it commits, marks the deck modified and arms the
                // draft without this thread knowing any of that.
                let _ = back.upgrade_in_event_loop(move |ui| match outcome {
                    Ok(text) => {
                        let text = text.trim().to_string();
                        ui.set_current_notes(text.clone().into());
                        ui.invoke_notes_edited(text.into());
                        ui.set_notice(SharedString::new());
                    }
                    Err(e) => {
                        ui.set_notice(e.to_string().into());
                    }
                });
            });
        });
    }

    // ── The presenter's clock ──
    //
    // One repeating second that counts only while the timer is running, rather than a timer
    // started and stopped from two handlers: a Slint timer that is started inside a callback
    // and dropped at the end of it does not tick, and this is the shape that cannot make that
    // mistake.
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_toggle_timer(move || {
            let Some(ui) = weak.upgrade() else { return };
            let clock = st.borrow().clock.clone();
            clock.running.set(!clock.running.get());
            render(&ui, &st, false);
        });
    }
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_reset_timer(move || {
            let Some(ui) = weak.upgrade() else { return };
            let clock = st.borrow().clock.clone();
            clock.running.set(false);
            clock.seconds.set(0);
            render(&ui, &st, false);
        });
    }

    let presenter = slint::Timer::default();
    {
        let weak = app.as_weak();
        let clock = state.borrow().clock.clone();
        presenter.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            if !clock.running.get() {
                return;
            }
            clock.seconds.set(clock.seconds.get() + 1);
            if let Some(ui) = weak.upgrade() {
                ui.set_timer_text(clock_text(clock.seconds.get()).into());
            }
        });
    }

    // ── Closing ──
    //
    // Text Editor asks before it closes a dirty document, because it has a dialog layer to ask
    // in. This screen has none, so the protection is the draft: the canvas is committed and the
    // recovery file written synchronously before the window goes, and the next start offers it
    // back by name.
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.window().on_close_requested(move || {
            if let Some(ui) = weak.upgrade() {
                st.borrow_mut().deck.commit(&canvas(&ui));
            }
            if let Err(e) = checkpoint(&st) {
                tracing::warn!(error = %e, "could not write the recovery draft on close");
            }
            slint::CloseRequestResponse::HideWindow
        });
    }

    presenter
}

/// The three edit callbacks, which differ only in which field they are about.
fn for_each_edit(app: &PresentationApp, state: &Shared, recovery: &Rc<slint::Timer>) {
    let edited = {
        let st = state.clone();
        let timer = recovery.clone();
        move |ui: &PresentationApp| {
            st.borrow_mut().deck.commit(&canvas(ui));
            render(ui, &st, false);
            arm_recovery(&st, &timer);
        }
    };

    {
        let weak = app.as_weak();
        let edited = edited.clone();
        app.on_title_edited(move |_| {
            if let Some(ui) = weak.upgrade() {
                edited(&ui);
            }
        });
    }
    {
        let weak = app.as_weak();
        let edited = edited.clone();
        app.on_body_edited(move |_| {
            if let Some(ui) = weak.upgrade() {
                edited(&ui);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_notes_edited(move |_| {
            if let Some(ui) = weak.upgrade() {
                edited(&ui);
            }
        });
    }
}

fn step_history(
    ui: &PresentationApp,
    state: &Shared,
    recovery: &Rc<slint::Timer>,
    forward: bool,
) {
    state.borrow_mut().deck.commit(&canvas(ui));
    if state.borrow_mut().deck.undo(forward) {
        ui.set_notice(SharedString::new());
    } else {
        say(ui, if forward { "Nothing to redo." } else { "Nothing to undo." });
    }
    render(ui, state, true);
    arm_recovery(state, recovery);
}

/// Find, count, and go to the first match. The count is what the search bar draws.
fn search(ui: &PresentationApp, state: &Shared, query: &str) {
    state.borrow_mut().deck.commit(&canvas(ui));
    let first = {
        let mut s = state.borrow_mut();
        s.hits = s.deck.search(query);
        s.hit_at = 0;
        s.hits.first().copied()
    };
    ui.set_search_query(query.into());
    match first {
        Some(at) => {
            go_to(ui, state, at as i64);
            ui.set_notice(SharedString::new());
        }
        None => {
            render(ui, state, false);
            if query.trim().is_empty() {
                ui.set_notice(SharedString::new());
            } else {
                say(ui, &format!("No slide holds \u{201c}{}\u{201d}.", query.trim()));
            }
        }
    }
}

/// Write one of the two text exports and report where it went.
fn export(
    ui: &PresentationApp,
    state: &Shared,
    extension: &str,
    to: Option<PathBuf>,
) -> Result<PathBuf, String> {
    state.borrow_mut().deck.commit(&canvas(ui));
    let s = state.borrow();
    let path = match to {
        Some(p) => p,
        None => s.deck.export_path(extension)?,
    };
    let text = if extension == "md" {
        s.deck.to_markdown()
    } else {
        s.deck.to_outline()
    };
    deck::write_atomically(&path, &text)?;
    Ok(path)
}

// ── The control surface ──────────────────────────────────────────────

/// Run an action's body, and say what happened twice.
///
/// Contract point 4: a failure that reaches only the caller leaves the person at the window
/// looking at a deck that did not change with no idea why. Every action below goes through
/// here, so the strip under the header and `describe.notice` carry the same sentence the caller
/// was given, and a success clears it.
fn answering(
    ui: Result<PresentationApp, String>,
    body: impl FnOnce(&PresentationApp) -> Result<serde_json::Value, String>,
) -> Result<serde_json::Value, String> {
    let ui = ui?;
    let outcome = body(&ui);
    match &outcome {
        Ok(_) => ui.set_notice(SharedString::new()),
        Err(message) => {
            tracing::warn!(error = %message, "yPresent refused an action");
            ui.set_notice(message.clone().into());
        }
    }
    outcome
}

/// What an action says about where the deck now is.
///
/// One shape, so a caller never has to learn a different one per verb, and every field is read
/// back out of the deck after the change rather than composed from the arguments that went in.
fn position(state: &Shared) -> serde_json::Value {
    let s = state.borrow();
    serde_json::json!({
        "index": s.deck.current(),
        "title": s.deck.current_slide().title,
        "slides": s.deck.len(),
        "dirty": s.deck.dirty(),
    })
}

/// What is on disk at `path`, read back after writing it.
fn written(path: &Path) -> serde_json::Value {
    let bytes = std::fs::metadata(path).map(|m| m.len()).ok();
    let slides = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| deck::parse(&text).ok())
        .map(|c| c.slides.len());
    serde_json::json!({ "path": path.display().to_string(), "bytes": bytes, "slides": slides })
}

/// A slide index out of the arguments.
///
/// A model writing JSON by hand sends `2` and `2.0` about equally often, so both are read
/// rather than one of them being refused as "not a whole number" when it plainly is one.
fn index_arg(args: &serde_json::Value, name: &str) -> Result<usize, String> {
    let value = args.get(name);
    let raw = value
        .and_then(|v| v.as_i64())
        .or_else(|| {
            value
                .and_then(|v| v.as_f64())
                .filter(|f| f.fract() == 0.0)
                .map(|f| f as i64)
        })
        .ok_or_else(|| format!("`{name}` must be a whole number"))?;
    if raw < 0 {
        return Err(format!("`{name}` is {raw}; slides are numbered from 0"));
    }
    Ok(raw as usize)
}

fn text_arg<'v>(args: &'v serde_json::Value, name: &str) -> Option<&'v str> {
    args.get(name).and_then(|v| v.as_str())
}

fn path_arg(args: &serde_json::Value, name: &str) -> Result<PathBuf, String> {
    let raw = text_arg(args, name).unwrap_or_default().trim();
    if raw.is_empty() {
        return Err(format!("`{name}` is empty"));
    }
    Ok(expanded(raw))
}

/// Publish this window on the bus.
///
/// `presentation` is the id, which is what `crates/yantrik-ui/src/wire/dock.rs` routes both
/// "presentation" and "slides" to (`Launch::Program { id: "presentation", bin:
/// "yantrik-presentation" }`), so the socket is `app-presentation` and the handover at the top
/// of `main` reaches this same surface.
///
/// Every action here runs on the UI thread with three seconds to answer before the caller is
/// told the app did not reply while the work carries on (`UI_ROUNDTRIP` in control.rs). None of
/// them needs longer: the largest thing any of them does is write or parse a few kilobytes of
/// JSON, so none declares `defers`.
fn publish_control(app: &PresentationApp, state: Shared) {
    use yantrik_app_runtime::control::{Action, App as Surface, Param, View};

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "the yPresent window is gone".to_string());

    let describe_ui = ui_for.clone();
    let describe_state = state.clone();
    let describe = move || {
        let Ok(ui) = describe_ui() else { return View::new("yPresent — closed") };
        let s = describe_state.borrow();
        let d = &s.deck;
        let at = d.current();
        let titles = d.outline();
        View::new(format!(
            "yPresent — {} · slide {} of {}{}",
            d.title(),
            at + 1,
            d.len(),
            if d.dirty() { " · unsaved" } else { "" }
        ))
        .with("file", d.path.as_ref().map(|p| p.display().to_string()))
        .with("dirty", d.dirty())
        .with("recovered", d.recovered)
        .with("slides", d.len())
        .with("index", at)
        .with("deck_title", d.title().to_string())
        .with(
            "current",
            serde_json::json!({
                "title": d.current_slide().title,
                "body": d.current_slide().body,
                "notes": d.current_slide().notes,
                "layout": d.current_slide().layout,
            }),
        )
        // Capped, and the cap is said out loud: an outline that silently stops at a hundred
        // titles is a caller believing it has seen the whole deck.
        .with("outline", titles.iter().take(OUTLINE_CAP).cloned().collect::<Vec<_>>())
        .with("outline_shown", titles.len().min(OUTLINE_CAP))
        .with("outline_total", titles.len())
        .with("presenting", ui.get_is_presenting())
        .with("theme", deck::theme_spec(d.theme()).name)
        .with("timer", clock_text(s.clock.seconds.get()))
        .with("timer_running", s.clock.running.get())
        .with("search_matches", s.hits.len())
        // Where Save would put a deck that has never been given a home, so a caller can find
        // the file afterwards without having to know the rule.
        .with(
            "saves_to",
            d.path
                .clone()
                .or_else(|| d.destination().ok())
                .map(|p| p.display().to_string()),
        )
        .with("recovery", s.recovery.as_ref().map(|p| p.display().to_string()))
        .with("notice", ui.get_notice().to_string())
    };

    let mut surface = Surface::new("presentation").describe(describe);

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("open", "Open a deck file in this window")
                .arg(Param::text("path").describe("Path to a .ydeck file")),
            move |args| {
                answering(ui_for(), |ui| {
                    let path = path_arg(args, "path")?;
                    let rescued = open_path(ui, &st, &path)?;
                    ui.window().set_minimized(false);
                    let s = st.borrow();
                    Ok(serde_json::json!({
                        "opened": s.deck.path.as_ref().map(|p| p.display().to_string()),
                        "slides": s.deck.len(),
                        "deck_title": s.deck.title(),
                        "previous_deck_saved_to": rescued.map(|p| p.display().to_string()),
                    }))
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(Action::new("show", "Bring the window forward"), move |_| {
            answering(ui_for(), |ui| {
                ui.window().set_minimized(false);
                Ok(serde_json::json!({ "showing": st.borrow().deck.title() }))
            })
        });
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(Action::new("save", "Write the deck to its file"), move |_| {
            answering(ui_for(), |ui| {
                let path = save(ui, &st, None)?;
                let mut answer = written(&path);
                answer["dirty"] = serde_json::json!(st.borrow().deck.dirty());
                Ok(answer)
            })
        });
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("save_as", "Write the deck to a path of your choosing")
                .arg(Param::text("path").describe("Where to write the .ydeck file")),
            move |args| {
                answering(ui_for(), |ui| {
                    let path = save(ui, &st, Some(path_arg(args, "path")?))?;
                    let mut answer = written(&path);
                    answer["dirty"] = serde_json::json!(st.borrow().deck.dirty());
                    Ok(answer)
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("new_deck", "Start an empty deck")
                .arg(Param::text("title").describe("What to call it").optional()),
            move |args| {
                answering(ui_for(), |ui| {
                    let asked = text_arg(args, "title").unwrap_or("Untitled").trim().to_string();
                    let title = if asked.is_empty() { "Untitled".to_string() } else { asked };
                    // Whatever was open is written to a file of its own first, so New never
                    // silently ends a deck someone was in the middle of.
                    let rescued = secure_current(ui, &st)?;
                    {
                        let mut s = st.borrow_mut();
                        s.deck = Deck::blank(title);
                        s.hits.clear();
                        s.hit_at = 0;
                    }
                    let _ = checkpoint(&st);
                    render(ui, &st, true);
                    let s = st.borrow();
                    Ok(serde_json::json!({
                        "deck_title": s.deck.title(),
                        "slides": s.deck.len(),
                        "index": s.deck.current(),
                        "previous_deck_saved_to": rescued.map(|p| p.display().to_string()),
                    }))
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("add_slide", "Add a slide and select it")
                .arg(Param::text("title").optional())
                .arg(Param::text("body").optional())
                .arg(Param::text("notes").optional())
                .arg(Param::number("after").describe("Insert after this slide index").optional()),
            move |args| {
                answering(ui_for(), |ui| {
                    let after = match args.get("after") {
                        Some(_) => Some(index_arg(args, "after")?),
                        None => None,
                    };
                    let next = st.borrow().deck.len() + 1;
                    let title = match text_arg(args, "title") {
                        Some(t) => t.to_string(),
                        None => format!("Slide {next}"),
                    };
                    let mut slide = Slide::new(
                        title,
                        text_arg(args, "body").unwrap_or_default().to_string(),
                        1,
                    );
                    slide.notes = text_arg(args, "notes").unwrap_or_default().to_string();
                    let at = st.borrow_mut().deck.add_slide(&canvas(ui), after, slide)?;
                    render(ui, &st, true);
                    let _ = checkpoint(&st);
                    let mut answer = position(&st);
                    answer["added_at"] = serde_json::json!(at);
                    Ok(answer)
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("set_slide", "Change the text of one slide")
                .arg(Param::number("index"))
                .arg(Param::text("title").optional())
                .arg(Param::text("body").optional())
                .arg(Param::text("notes").optional()),
            move |args| {
                answering(ui_for(), |ui| {
                    let index = index_arg(args, "index")?;
                    if args.get("title").is_none()
                        && args.get("body").is_none()
                        && args.get("notes").is_none()
                    {
                        return Err("give at least one of `title`, `body` or `notes`".into());
                    }
                    // The canvas first, or editing slide 5 through the surface would be undone
                    // by the next commit of whatever slide the person is looking at.
                    st.borrow_mut().deck.commit(&canvas(ui));
                    st.borrow_mut().deck.set_slide(
                        index,
                        text_arg(args, "title"),
                        text_arg(args, "body"),
                        text_arg(args, "notes"),
                    )?;
                    render(ui, &st, true);
                    let _ = checkpoint(&st);
                    let s = st.borrow();
                    let slide = s.deck.slide(index).ok_or("the slide went away")?;
                    Ok(serde_json::json!({
                        "index": index,
                        "title": slide.title,
                        "body": slide.body,
                        "notes": slide.notes,
                        "dirty": s.deck.dirty(),
                    }))
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            // Graded sensitive because it destroys writing. Nothing else on this surface does:
            // `new_deck` and `open` save the deck they replace before replacing it,
            // `move_slide` reorders, and `set_slide` is a change whose text the caller supplied.
            // This is the one action whose whole effect is that a slide someone wrote is gone,
            // out of a deck that may never have been saved.
            Action::new("delete_slide", "Remove a slide from the deck")
                .risk("sensitive")
                .arg(Param::number("index")),
            move |args| {
                answering(ui_for(), |ui| {
                    let index = index_arg(args, "index")?;
                    st.borrow_mut().deck.commit(&canvas(ui));
                    let gone = st.borrow_mut().deck.delete_slide(index)?;
                    render(ui, &st, true);
                    let _ = checkpoint(&st);
                    let mut answer = position(&st);
                    answer["deleted"] = serde_json::json!(gone.title);
                    Ok(answer)
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("move_slide", "Move a slide to another position")
                .arg(Param::number("from"))
                .arg(Param::number("to")),
            move |args| {
                answering(ui_for(), |ui| {
                    let from = index_arg(args, "from")?;
                    let to = index_arg(args, "to")?;
                    st.borrow_mut().deck.commit(&canvas(ui));
                    let landed = st.borrow_mut().deck.move_slide(from, to)?;
                    render(ui, &st, true);
                    let _ = checkpoint(&st);
                    let mut answer = position(&st);
                    answer["moved_to"] = serde_json::json!(landed);
                    Ok(answer)
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("go_to", "Select a slide by index").arg(Param::number("index")),
            move |args| {
                answering(ui_for(), |ui| {
                    let index = index_arg(args, "index")?;
                    let len = st.borrow().deck.len();
                    if index >= len {
                        return Err(format!("there is no slide {} in a deck of {len}", index + 1));
                    }
                    go_to(ui, &st, index as i64);
                    Ok(position(&st))
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(Action::new("next", "The next slide"), move |_| {
            answering(ui_for(), |ui| {
                let at = st.borrow().deck.current() as i64;
                go_to(ui, &st, at + 1);
                Ok(position(&st))
            })
        });
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(Action::new("previous", "The previous slide"), move |_| {
            answering(ui_for(), |ui| {
                let at = st.borrow().deck.current() as i64;
                go_to(ui, &st, at - 1);
                Ok(position(&st))
            })
        });
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("present", "Enter or leave the full-screen presenter view")
                .arg(Param::text("mode").describe("on | off | toggle")),
            move |args| {
                answering(ui_for(), |ui| {
                    let asked = text_arg(args, "mode").unwrap_or_default().trim().to_lowercase();
                    let wanted = match asked.as_str() {
                        "on" | "true" | "start" => true,
                        "off" | "false" | "stop" => false,
                        "toggle" => !ui.get_is_presenting(),
                        other => {
                            return Err(format!("unknown mode `{other}`; use on, off or toggle"))
                        }
                    };
                    // Through `go_to` so entering and leaving both commit the canvas; the
                    // presenter view reads the model, not the editor's properties.
                    let at = st.borrow().deck.current() as i64;
                    go_to(ui, &st, at);
                    ui.set_is_presenting(wanted);
                    let mut answer = position(&st);
                    answer["presenting"] = serde_json::json!(ui.get_is_presenting());
                    Ok(answer)
                })
            },
        );
    }

    {
        let ui_for = ui_for.clone();
        let st = state.clone();
        surface = surface.action(
            Action::new("export_markdown", "Write the deck out as Markdown")
                .arg(Param::text("path").describe("Where to write it").optional()),
            move |args| {
                answering(ui_for(), |ui| {
                    let to = match args.get("path") {
                        Some(_) => Some(path_arg(args, "path")?),
                        None => None,
                    };
                    let path = export(ui, &st, "md", to)?;
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).ok();
                    Ok(serde_json::json!({
                        "path": path.display().to_string(),
                        "bytes": bytes,
                        "slides": st.borrow().deck.len(),
                    }))
                })
            },
        );
    }

    surface.serve();
}
