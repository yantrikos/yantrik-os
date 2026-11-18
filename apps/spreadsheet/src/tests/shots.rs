//! The real window drawn on Slint's software renderer with no display, written as screenshots
//! beside the test binary (`<target>/<profile>/ui-screenshots/`): the sheet, the question over it,
//! and the window drawn for a Mind. Each asserts what it shows as well, so a picture is never the
//! only evidence.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use slint::ComponentHandle as _;
use yantrik_app::app::WindowId;
use yantrik_app::for_mind::HIDDEN;
use yantrik_app::mind::fake::FakeMind;
use yantrik_testing::TestApp;

use super::{act, settle, started};
use crate::YSheets;

fn shot_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn screenshot(app: &TestApp<YSheets>, name: &str) {
    app.ui(|_| slint::platform::update_timers_and_animations());
    let (width, height, pixels) = app.ui(|ui| {
        let shot = ui.window().take_snapshot().expect("a snapshot of the software window");
        (shot.width(), shot.height(), shot.as_bytes().to_vec())
    });
    let mut png = png::Encoder::new(std::fs::File::create(shot_dir().join(name)).unwrap(), width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header().unwrap().write_image_data(&pixels).unwrap();
}

fn sized(app: &TestApp<YSheets>) {
    app.ui(|ui| ui.window().set_size(slint::PhysicalSize::new(1300, 800)));
}

/// A small budget: numbers, a total, a text that makes a formula go wrong.
fn budget(app: &TestApp<YSheets>) {
    for (cell, text) in [
        ("A1", "Item"),
        ("B1", "Cost"),
        ("A2", "Rent"),
        ("B2", "1200"),
        ("A3", "Food"),
        ("B3", "430.5"),
        ("A4", "Total"),
        ("B4", "=SUM(B2:B3)"),
        ("C4", "=A2+1"),
    ] {
        act(app, "set_cell", json!({ "cell": cell, "text": text })).unwrap();
    }
    act(app, "select", json!({ "cell": "B4" })).unwrap();
    settle(app);
}

#[test]
fn the_sheet_is_drawn_with_numbers_text_and_an_error() {
    let app = started();
    sized(&app);
    budget(&app);
    assert_eq!(super::shown(&app, "B4"), "1630.5");
    assert_eq!(super::shown(&app, "C4"), "#VALUE!");
    screenshot(&app, "ysheets-sheet.png");
}

#[test]
fn the_unsaved_changes_question_is_drawn_over_the_sheet() {
    let app = started();
    sized(&app);
    budget(&app);
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_dialog()), crate::SheetDialog::Unsaved);
    screenshot(&app, "ysheets-question.png");
}

#[test]
fn a_window_drawn_for_a_mind_shows_the_mask_where_the_contents_were() {
    let paths = {
        let app = started();
        budget(&app);
        super::flush_drafts(&app);
        let paths = app.paths();
        let _ = app.shutdown();
        paths
    };
    let app = TestApp::<YSheets>::start_for_mind(paths, Vec::new(), Arc::new(FakeMind::new()));
    sized(&app);
    settle(&app);
    assert_eq!(super::shown(&app, "B2"), HIDDEN);
    assert_eq!(super::bar(&app), HIDDEN);
    assert_eq!(app.ui(|ui| ui.get_active_ref().to_string()), "B4");
    screenshot(&app, "ysheets-mind-view.png");
}
