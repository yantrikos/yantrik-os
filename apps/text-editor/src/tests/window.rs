//! The real window, its keys, callbacks and drawing, on Slint's software renderer with no display.

use std::path::PathBuf;

use serde_json::json;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle as _, SharedString};
use yantrik_testing::TestApp;

use super::{act, file, notice, press, scratch, settle, started, tabs, text};
use crate::command::WindowAction;
use crate::{TextEditor, TextEditorApp, ThemeMode};

/// Beside the test binary, in `<target>/<profile>/ui-screenshots/`.
fn shot_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn screenshot(app: &TestApp<TextEditor>, name: &str) {
    let (width, height, pixels) = app.ui(|ui| {
        let shot = ui.window().take_snapshot().expect("a snapshot of the software window");
        (shot.width(), shot.height(), shot.as_bytes().to_vec())
    });
    let mut png = png::Encoder::new(std::fs::File::create(shot_dir().join(name)).unwrap(), width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(&pixels).unwrap();
}

fn tap(ui: &TextEditorApp, text: &str) {
    let text: SharedString = text.into();
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window().dispatch_event(WindowEvent::KeyReleased { text });
}

/// Types into the editor field, as the person would.
fn keys(app: &TestApp<TextEditor>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        ui.invoke_focus_editor();
        tap(ui, &text);
    });
}

/// A chord with Control held, in one step on the UI thread.
fn control(app: &TestApp<TextEditor>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        ui.invoke_focus_editor();
        ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
        tap(ui, &text);
        ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    });
}

/// What the editor field shows.
fn shown(app: &TestApp<TextEditor>) -> String {
    app.ui(|ui| ui.get_content().to_string())
}

/// Lets Slint run its `changed` handlers, which nothing turns under the test harness.
fn turn(app: &TestApp<TextEditor>) {
    app.ui(|_| slint::platform::update_timers_and_animations());
}

fn sized(app: &TestApp<TextEditor>, width: u32, height: u32) {
    app.ui(move |ui| ui.window().set_size(slint::PhysicalSize::new(width, height)));
}

#[test]
fn the_keyboard_types_undoes_switches_tabs_finds_and_replaces() {
    let app = started();
    sized(&app, 1100, 760);
    keys(&app, "Hello 🦀\nKelvin K kelvin\n");
    assert_eq!(text(&app), "Hello 🦀\nKelvin K kelvin\n", "{}", notice(&app));
    assert_eq!(app.ui(|ui| ui.get_line_count()), 3);
    control(&app, "n");
    keys(&app, "second draft");
    assert_eq!((tabs(&app), text(&app)), (2, "second draft".into()));
    control(&app, "z");
    assert_eq!(shown(&app), "");
    press(&app, WindowAction::Redo);
    assert_eq!(shown(&app), "second draft");
    app.ui(|ui| ui.invoke_select_tab(0));
    assert!(shown(&app).starts_with("Hello"));
    app.ui(|ui| {
        ui.set_query("k".into());
        ui.invoke_search();
    });
    assert_eq!(app.ui(|ui| ui.get_match_count()), 3);
    app.ui(|ui| {
        ui.set_replacement("X".into());
        ui.invoke_action("replace-all".into());
    });
    assert!(shown(&app).contains("Xelvin X Xelvin"), "{}", shown(&app));
    app.ui(|ui| ui.invoke_close_tab(0));
    assert_eq!(app.ui(|ui| ui.get_dialog()), 3);
    app.ui(|ui| ui.invoke_action("cancel".into()));
    assert_eq!(tabs(&app), 2);
    // Ctrl+S on a tab with no file asks where; the dialog's path is the model's.
    control(&app, "s");
    assert_eq!(app.ui(|ui| (ui.get_dialog(), ui.get_dialog_path().to_string())), (2, "~/untitled.txt".into()));
    let path = scratch("keys").join("saved.txt");
    app.ui({
        let path = path.display().to_string();
        move |ui| {
            ui.set_dialog_path(path.into());
            ui.invoke_action("confirm".into());
        }
    });
    settle(&app);
    assert_eq!(file(&path), "Hello 🦀\nXelvin X Xelvin\n");
    assert_eq!(app.ui(|ui| (ui.get_dialog(), ui.get_modified())), (0, false));
}

#[test]
fn many_tabs_open_and_the_strip_scrolls_to_the_one_in_front() {
    let app = started();
    sized(&app, 1100, 760);
    for _ in 0..23 {
        press(&app, WindowAction::New);
    }
    assert_eq!(tabs(&app), 24, "{}", notice(&app));
    assert_eq!(app.ui(|ui| ui.get_active_tab()), 23);
    screenshot(&app, "editor-many-tabs.png");
    for _ in 0..16 {
        app.ui(|ui| ui.invoke_close_tab(2));
    }
    assert_eq!(tabs(&app), 8);
}

#[test]
fn the_editor_draws_in_both_themes_and_the_prompt_over_it() {
    let app = started();
    sized(&app, 1100, 760);
    let demo = scratch("drawn").join("hello.rs");
    super::opened(
        &app,
        &demo,
        "// A small idea, ready to grow.\n\nfn main() {\n    let message = \"Hello, Yantrik\";\n    println!(\"{}\", message);\n}\n",
    );
    assert!(app.ui(|ui| ui.get_has_highlights()), "a Rust file is coloured");
    screenshot(&app, "editor-dark.png");
    app.ui(|ui| ui.global::<ThemeMode>().set_dark(false));
    sized(&app, 800, 600);
    screenshot(&app, "editor-light.png");
    keys(&app, "// unsaved\n");
    press(&app, WindowAction::Close);
    screenshot(&app, "editor-prompt.png");
    assert_eq!(app.ui(|ui| ui.get_close_label().to_string()), "hello.rs has unsaved changes.");
}

/// #328: Slint's software renderer keeps every physical coordinate in an `i16`, so a document of
/// ~2000 lines or a ~3900-character line aborted the window mid-draw. The editor draws a read-only
/// leading window of such a document and keeps every byte; every snapshot below rendered it.
#[test]
fn a_document_too_big_to_draw_is_windowed_read_only_and_kept_whole() {
    let app = started();
    sized(&app, 1100, 760);
    let long = "x".repeat(5_000);
    act(&app, "new", json!({ "text": long })).unwrap();
    screenshot(&app, "editor-wide.png");
    assert!(app.ui(|ui| ui.get_view_limited()), "the wide document is windowed");
    assert!(shown(&app).len() < long.len());
    assert_eq!(text(&app), long, "the document keeps every character");
    assert!(!app.ui(|ui| ui.get_view_status().to_string()).is_empty());
    keys(&app, "y");
    assert_eq!(text(&app), long, "the read-only view takes no keystroke");
    let path = scratch("wide").join("whole.txt");
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(file(&path), long, "the file on disk is the whole document");

    let tall = format!("{}MARKER\n", "let value = 1;\n".repeat(19_000));
    act(&app, "new", json!({ "text": tall })).unwrap();
    screenshot(&app, "editor-tall.png");
    assert!(shown(&app).lines().count() < 1_000, "{} lines", shown(&app).lines().count());
    let found = act(&app, "find", json!({ "text": "MARKER" })).unwrap();
    assert_eq!(found["matches"], 1, "the match is in the document: {found}");
    // A bigger font fits fewer lines inside the limit; the window follows without a restart.
    turn(&app);
    app.ui(|ui| ui.set_font_pixels(22));
    turn(&app);
    let big = shown(&app).lines().count();
    assert!(big > 0 && big < 400, "22px window is {big} lines");
    app.ui(|ui| ui.set_font_pixels(14));
    turn(&app);
    assert!(shown(&app).lines().count() > big);
    let summary = super::summary(&app);
    assert!(summary.contains("19002 lines") && summary.contains("Showing the first"), "{summary}");
}
