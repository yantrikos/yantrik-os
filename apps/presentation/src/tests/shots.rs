//! The real window drawn on Slint's software renderer with no display, written as screenshots
//! beside the test binary (`<target>/<profile>/ui-screenshots/`): the editor, the presenter view,
//! and the window drawn for a Mind. Each asserts what it shows as well, so a picture is never the
//! only evidence.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use slint::{ComponentHandle as _, Model as _};
use yantrik_app::for_mind::HIDDEN;
use yantrik_app::mind::fake::FakeMind;
use yantrik_testing::TestApp;

use super::{act, press, settle, started};
use crate::command::WindowAction;
use crate::YPresent;

fn shot_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("ui-screenshots")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn screenshot(app: &TestApp<YPresent>, name: &str) {
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

fn sized(app: &TestApp<YPresent>) {
    app.ui(|ui| ui.window().set_size(slint::PhysicalSize::new(1300, 800)));
}

/// A talk of three slides on the second, with notes, in the Paper theme.
fn talk(app: &TestApp<YPresent>) {
    act(app, "new_deck", json!({ "title": "Quarterly review" })).unwrap();
    act(app, "set_slide", json!({ "index": 0, "title": "Quarterly review", "body": "Where we are, and where next" })).unwrap();
    act(app, "add_slide", json!({ "title": "Costs fell by half", "body": "Unit cost, year on year", "notes": "Open with the curve." })).unwrap();
    act(app, "add_slide", json!({ "title": "Next steps", "body": "Hire two people" })).unwrap();
    act(app, "go_to", json!({ "index": 1 })).unwrap();
    app.ui(|ui| ui.invoke_set_theme(1));
    settle(app);
}

#[test]
fn the_editor_is_drawn_with_the_rail_the_canvas_and_the_status() {
    let app = started();
    sized(&app);
    talk(&app);
    assert_eq!(app.ui(|ui| ui.get_slides().row_count()), 3);
    assert_eq!(app.ui(|ui| ui.get_current_title().to_string()), "Costs fell by half");
    screenshot(&app, "presentation-editor.png");
}

#[test]
fn the_presenter_view_is_drawn_with_the_next_card_and_the_clock() {
    let app = started();
    sized(&app);
    talk(&app);
    press(&app, WindowAction::TogglePresent);
    press(&app, WindowAction::ToggleTimer);
    app.advance(std::time::Duration::from_secs(75));
    assert_eq!(app.ui(|ui| (ui.get_is_presenting(), ui.get_timer_text().to_string())), (true, "01:15".to_string()));
    screenshot(&app, "presentation-presenter.png");
}

#[test]
fn a_window_drawn_for_a_mind_shows_the_mask_where_the_text_was() {
    let paths = {
        let app = started();
        talk(&app);
        app.run_writes();
        let paths = app.paths();
        let _ = app.shutdown();
        paths
    };
    let app = TestApp::<YPresent>::start_for_mind(paths, Vec::new(), Arc::new(FakeMind::new()));
    sized(&app);
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_current_title().to_string()), HIDDEN);
    assert_eq!(app.ui(|ui| ui.get_presentation_title().to_string()), "Quarterly review");
    screenshot(&app, "presentation-mind-view.png");
}
