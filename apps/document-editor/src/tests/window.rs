//! The real window, its callbacks and drawing, on Slint's software renderer with no display.

use std::path::PathBuf;

use slint::ComponentHandle as _;
use slint::Model as _;
use yantrik_testing::TestApp;

use super::{file, modified, notice, scratch, settle, started, text, typed};
use crate::YDoc;

/// Beside the test binary, in `<target>/<profile>/ui-screenshots/`.
fn shot_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn screenshot(app: &TestApp<YDoc>, name: &str) {
    let (width, height, pixels) = app.ui(|ui| {
        let shot = ui.window().take_snapshot().expect("a snapshot of the software window");
        (shot.width(), shot.height(), shot.as_bytes().to_vec())
    });
    let mut png = png::Encoder::new(std::fs::File::create(shot_dir().join(name)).unwrap(), width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(&pixels).unwrap();
}

fn sized(app: &TestApp<YDoc>, width: u32, height: u32) {
    app.ui(move |ui| ui.window().set_size(slint::PhysicalSize::new(width, height)));
}

/// What the page shows.
fn shown(app: &TestApp<YDoc>) -> String {
    app.ui(|ui| ui.get_doc_content().to_string())
}

/// Lets Slint run its `changed` handlers, which nothing turns under the test harness.
fn turn(app: &TestApp<YDoc>) {
    app.ui(|_| slint::platform::update_timers_and_animations());
}

/// Typing in the page, as the screen reports it.
fn type_in_page(app: &TestApp<YDoc>, new_text: &str) {
    let new_text = new_text.to_string();
    app.ui(move |ui| {
        ui.set_doc_content(new_text.as_str().into());
        ui.invoke_doc_content_changed(new_text.as_str().into());
    });
    settle(app);
}

fn status(app: &TestApp<YDoc>) -> (String, bool) {
    app.ui(|ui| (ui.get_doc_save_status().to_string(), ui.get_doc_is_modified()))
}

#[test]
fn the_screen_shows_the_document_its_title_counts_outline_and_status() {
    let app = started();
    sized(&app, 1100, 760);
    assert_eq!(status(&app), ("New document".into(), false));
    type_in_page(&app, "# Plan\n\nOne two three.\n\n## Part\n");
    assert_eq!(text(&app), "# Plan\n\nOne two three.\n\n## Part\n");
    assert_eq!(status(&app), ("Unsaved".into(), true));
    let (title, words, chars, headings) = app.ui(|ui| {
        (ui.get_doc_title().to_string(), ui.get_doc_word_count(), ui.get_doc_char_count(), ui.get_doc_headings().row_count())
    });
    assert_eq!((title.as_str(), words, chars, headings), ("Plan", 7, 32, 2));
    let path = scratch("screen").join("plan.md");
    app.ui(|ui| ui.invoke_doc_save());
    settle(&app);
    // No file yet: Save asks for one. The dialog's field is given a suggestion, and confirming
    // it writes the file.
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 2);
    app.ui(move |ui| {
        ui.set_doc_dialog_path(path.display().to_string().into());
        ui.invoke_doc_dialog_confirm();
    });
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 0);
    assert_eq!(status(&app), ("Saved".into(), false));
    assert_eq!(file(&super::path_of(&app).unwrap()), "# Plan\n\nOne two three.\n\n## Part\n");
    turn(&app);
    screenshot(&app, "document-editor-saved.png");
}

#[test]
fn the_page_is_given_text_only_when_it_differs_so_typing_never_moves_the_caret() {
    let app = started();
    type_in_page(&app, "abc");
    // A render after typing finds the page already holding the text: it is not set again.
    app.ui(|ui| ui.set_doc_content("abc".into()));
    typed(&app, "abc");
    assert_eq!(shown(&app), "abc");
    // A change from somewhere else is put on the page.
    super::act(&app, "append", serde_json::json!({ "text": "def" })).unwrap();
    assert_eq!(shown(&app), "abc\ndef");
}

#[test]
fn the_format_buttons_wrap_the_selection_and_put_it_back() {
    let app = started();
    type_in_page(&app, "Hello world");
    app.ui(|ui| {
        ui.invoke_select_range(0, 5);
    });
    turn(&app);
    let (anchor, cursor) = app.ui(|ui| (ui.get_doc_anchor(), ui.get_doc_cursor()));
    assert_eq!((anchor, cursor), (0, 5), "the page reports its selection");
    app.ui(|ui| ui.invoke_doc_format_bold());
    settle(&app);
    assert_eq!(text(&app), "**Hello** world");
    assert_eq!(shown(&app), "**Hello** world");
    app.ui(|ui| ui.invoke_doc_format_heading(2));
    settle(&app);
    assert!(text(&app).starts_with("## "), "{}", text(&app));
    assert!(modified(&app));
}

#[test]
fn find_and_replace_work_from_the_bar() {
    let app = started();
    type_in_page(&app, "Kelvin kelvin KELVIN");
    app.ui(|ui| {
        ui.set_doc_find_query("kelvin".into());
        ui.invoke_doc_find_changed("kelvin".into());
        ui.set_doc_replace_text("K".into());
    });
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_doc_find_count()), 3);
    app.ui(|ui| ui.invoke_doc_find_next());
    app.ui(|ui| ui.invoke_doc_replace_one());
    settle(&app);
    assert_eq!(text(&app), "Kelvin K KELVIN");
    app.ui(|ui| ui.invoke_doc_replace_all());
    settle(&app);
    assert_eq!(text(&app), "K K K");
    app.ui(|ui| {
        ui.set_doc_find_query("zzz".into());
        ui.invoke_doc_find_changed("zzz".into());
        ui.invoke_doc_replace_all();
    });
    settle(&app);
    assert_eq!(notice(&app), "Nothing matched, so nothing was replaced.");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), "Nothing matched, so nothing was replaced.");
}

#[test]
fn undo_takes_the_pages_own_history_back_into_the_document() {
    let app = started();
    type_in_page(&app, "first");
    app.ui(|ui| ui.invoke_doc_undo());
    settle(&app);
    assert_eq!(text(&app), shown(&app), "the document is never one step behind the page");
}

#[test]
fn a_heading_in_the_outline_puts_the_caret_there() {
    let app = started();
    type_in_page(&app, "# One\n\n## Two\n");
    app.ui(|ui| ui.invoke_doc_heading_clicked(7));
    settle(&app);
    turn(&app);
    assert_eq!(app.ui(|ui| ui.get_doc_cursor()), 7);
}

#[test]
fn the_window_closing_with_unsaved_text_shows_the_prompt() {
    let app = started();
    sized(&app, 1100, 760);
    type_in_page(&app, "# Unsaved\n\nwords\n");
    app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 5);
    turn(&app);
    screenshot(&app, "document-editor-unsaved-prompt.png");
    app.ui(|ui| ui.invoke_doc_dialog_cancel());
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 0);
}
