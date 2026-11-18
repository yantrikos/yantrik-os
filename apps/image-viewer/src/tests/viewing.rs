//! Looking at a picture: turning and mirroring it (the pixels, never the file), fit and fill,
//! the info panel and the slideshow the screen keeps itself, and the Describe button's ask.

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::{Reach, OFFLINE_HINT};
use yantrik_app::watch::Seen;

use super::fixture::{a_pixels, drawn, open, showing, start_with, Folder};
use crate::{Command, Orientation, ViewState, NO_PICTURE};

#[test]
fn rotate_and_flip_turn_the_pixels_on_screen_and_never_the_file() {
    let folder = Folder::new("turn");
    let on_disk = std::fs::read(folder.path("a.png")).unwrap();
    let app = showing(&folder, "a.png");
    assert_eq!(drawn(&app), (3, 2, a_pixels()));

    let reply = app.act("rotate", json!({ "direction": "right" })).unwrap().settle().unwrap();
    assert_eq!(
        reply,
        json!({ "rotation": 90, "flipped_horizontally": false, "flipped_vertically": false, "file_unchanged": true })
    );
    assert_eq!(app.ui(|ui| ui.get_viewer_rotation()), 90, "the badge and the buttons say it at once");
    assert_eq!(drawn(&app).0, 3, "the turned pixels are made off the UI thread");
    app.run_jobs();
    let right = Orientation::default().rotate_right();
    let (want, w, h) = right.apply(&a_pixels(), 3, 2);
    assert_eq!(drawn(&app), (w as u32, h as u32, want), "stood on its side");
    assert_eq!(app.describe()["state"]["rotation"], json!(90));

    // The toolbar's buttons are the same path.
    app.ui(|ui| ui.invoke_viewer_flip_horizontal());
    assert!(app.ui(|ui| ui.get_viewer_flip_h()));
    app.run_jobs();
    let flipped = right.flip_horizontal();
    let (want, _, _) = flipped.apply(&a_pixels(), 3, 2);
    assert_eq!(drawn(&app).2, want);
    // Two presses before either lands: only the last is drawn.
    app.ui(|ui| ui.invoke_viewer_rotate_right());
    app.ui(|ui| ui.invoke_viewer_rotate_right());
    app.run_jobs();
    let twice = flipped.rotate_right().rotate_right();
    assert_eq!(app.model(|m| m.shown.orientation), twice);
    assert_eq!(drawn(&app).2, twice.apply(&a_pixels(), 3, 2).0);
    // Back to how it is stored: drawn at once, with no job.
    app.ui(|ui| ui.invoke_viewer_rotate_left());
    app.ui(|ui| ui.invoke_viewer_rotate_left());
    app.ui(|ui| ui.invoke_viewer_flip_horizontal());
    app.ui(|ui| ui.invoke_viewer_rotate_left());
    assert_eq!(app.model(|m| m.shown.orientation), Orientation::default());
    app.run_jobs();
    assert_eq!(drawn(&app), (3, 2, a_pixels()));

    let flip = app.act("flip", json!({ "direction": "Vertically" })).unwrap().settle().unwrap();
    assert_eq!(flip["flipped_vertically"], json!(true));
    let e = app.act("rotate", json!({ "direction": "up" })).unwrap_err();
    assert_eq!(e, "unknown direction `up`; use left or right");
    let e = app.act("flip", json!({ "direction": "sideways" })).unwrap_err();
    assert_eq!(e, "unknown direction `sideways`; use horizontal or vertical");
    assert!(app.act("rotate", json!({})).is_err(), "the direction is needed");
    app.run_jobs();
    assert_eq!(std::fs::read(folder.path("a.png")).unwrap(), on_disk, "the file is never touched");
}

#[test]
fn a_new_picture_is_shown_unturned_and_a_turn_made_for_the_last_one_is_dropped() {
    let folder = Folder::new("unturned");
    let app = showing(&folder, "a.png");
    app.ui(|ui| ui.invoke_viewer_rotate_right());
    app.run_jobs();
    let turned = app.model(|m| m.shown.turned.clone().unwrap());
    let screen = app.model(|m| m.screen);
    app.ui(|ui| ui.invoke_nav_next());
    app.run_jobs();
    assert_eq!(app.model(|m| m.shown.orientation), Orientation::default());
    assert_eq!((app.ui(|ui| ui.get_viewer_rotation()), app.ui(|ui| ui.get_viewer_flip_h())), (0, false));
    assert_eq!(drawn(&app).0, 2, "b.png, as stored");
    // The last picture's turn, landing late, is not drawn over this one.
    app.run(Command::Turned { screen, orientation: turned.0, pixels: Some(turned.1) });
    assert_eq!(app.model(|m| m.shown.orientation), Orientation::default());
    assert_eq!(drawn(&app).0, 2);
}

#[test]
fn fit_and_the_info_panel_answer_and_describe_reads_what_the_window_shows() {
    let folder = Folder::new("fit");
    let app = showing(&folder, "a.png");
    let fit = app.act("fit", json!({})).unwrap().settle().unwrap();
    assert_eq!(fit, json!({ "fit_to_window": false }));
    assert!(!app.ui(|ui| ui.get_fit_contain()), "the window fills");
    assert_eq!(app.act("fit", json!({})).unwrap().settle().unwrap(), json!({ "fit_to_window": true }));
    assert!(app.ui(|ui| ui.get_fit_contain()));

    // A zoom of the person's own, as the window tells it: `fit` goes back to the whole picture.
    // The two `fit`s above changed the window's fit flag, and its change handler tells the app
    // what the window shows on the next turn of the event loop. Every test shares one UI thread,
    // so any other test's turn can be that next turn, and it would land after the zoom told here
    // and put the window's own (unzoomed) state over it. Take that turn first, so none is owed.
    app.ui(|_| slint::platform::update_timers_and_animations());
    let zoomed = ViewState { zoomed: true, fit_contain: false, ..ViewState::default() };
    app.run(Command::ViewSeen(zoomed));
    assert_eq!(app.describe()["state"]["zoomed"], json!(true));
    assert_eq!(app.describe()["state"]["fit_to_window"], json!(false));
    assert_eq!(app.act("fit", json!({})).unwrap().settle().unwrap(), json!({ "fit_to_window": true }));
    assert_eq!(app.describe()["state"]["zoomed"], json!(false));

    let info = app.act("toggle_info", json!({})).unwrap().settle().unwrap();
    assert_eq!(info, json!({ "info_panel": true }));
    assert!(app.ui(|ui| ui.get_viewer_info_open()));
    assert_eq!(app.act("toggle_info", json!({})).unwrap().settle().unwrap(), json!({ "info_panel": false }));
    assert!(!app.ui(|ui| ui.get_viewer_info_open()));
}

/// The header's Info and Slideshow buttons flip the window's flag themselves and then call back.
/// The app used to flip the same flag again in that callback, undoing the press: the info panel
/// never opened from the header and the slideshow always started paused. Now the callback only
/// says what the window shows. Driven here as the markup drives it: the flag, then the callback.
#[test]
fn the_headers_info_and_slideshow_buttons_keep_their_press() {
    let folder = Folder::new("header");
    let app = showing(&folder, "a.png");
    app.ui(|ui| {
        ui.set_viewer_info_open(true);
        ui.invoke_viewer_toggle_info();
    });
    assert!(app.ui(|ui| ui.get_viewer_info_open()), "the panel stays open");
    assert_eq!(app.describe()["state"]["info_panel"], json!(true));

    app.ui(|ui| {
        ui.set_viewer_slideshow_active(true);
        ui.set_viewer_slideshow_paused(false);
        ui.invoke_viewer_slideshow_toggle();
    });
    assert!(!app.ui(|ui| ui.get_viewer_slideshow_paused()), "the slideshow runs");
    assert_eq!(app.describe()["state"]["slideshow"], json!(true));
    // The slideshow's own timer steps with the window's arrow command.
    app.ui(|ui| ui.invoke_nav_next());
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "b.png");
    app.ui(|ui| {
        ui.set_viewer_slideshow_active(false);
        ui.invoke_viewer_slideshow_stop();
    });
    assert_eq!(app.describe()["state"]["slideshow"], json!(false));

    // What changes with no callback at all (the panel's own close button, a zoom) is told by
    // the window's change handlers, which run each time the event loop turns. The first turn
    // reads where the window is; the next one sees what changed since.
    let turn = || app.ui(|_| slint::platform::update_timers_and_animations());
    turn();
    assert_eq!(app.describe()["state"]["info_panel"], json!(true));
    app.ui(|ui| ui.set_viewer_info_open(false));
    turn();
    assert_eq!(app.describe()["state"]["info_panel"], json!(false));
    app.ui(|ui| ui.set_fit_contain(false));
    turn();
    assert_eq!(app.describe()["state"]["fit_to_window"], json!(false));
    // The fit toggle's callback says the same.
    app.ui(|ui| {
        ui.set_fit_contain(true);
        ui.invoke_toggle_fit();
    });
    assert_eq!(app.describe()["state"]["fit_to_window"], json!(true));
}

#[test]
fn describe_asks_about_what_the_file_records_and_drops_an_answer_about_the_last_picture() {
    let folder = Folder::new("ask");
    let mind = FakeMind::new().answers([Ok("A small PNG, three by two.".to_string()), Ok("late".to_string())]);
    let (app, mind) = start_with(mind, Vec::new());

    // No picture: the panel says why.
    app.ui(|ui| ui.invoke_ai_describe_pressed());
    assert_eq!(app.ui(|ui| ui.get_ai_response()), NO_PICTURE);
    open(&app, &folder.path("a.png")).unwrap();
    // No companion to ask: the panel says that, and nothing is sent.
    app.ui(|ui| ui.invoke_ai_describe_pressed());
    assert_eq!(app.ui(|ui| ui.get_ai_response()), OFFLINE_HINT);
    assert_eq!(app.jobs_waiting(), 0);

    mind.set_reach(Reach::Ready);
    app.seen(Seen::Reach(Reach::Ready));
    app.ui(|ui| ui.invoke_ai_describe_pressed());
    assert!(app.ui(|ui| ui.get_ai_is_working()));
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "");
    app.run_jobs();
    let prompt = mind.prompts()[0].clone();
    assert!(prompt.contains("the picture a.png"), "{prompt}");
    assert!(prompt.contains("Dimensions: 3 \u{d7} 2") && prompt.contains("Format: PNG"), "{prompt}");
    assert!(!prompt.contains("GPS"), "{prompt}");
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "A small PNG, three by two.");
    app.ui(|ui| ui.invoke_ai_dismiss());
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "");

    // A step, then a press while the next picture loads: the ask is about a.png, still on
    // screen, and its answer lands after b.png is, so it is dropped.
    app.ui(|ui| ui.invoke_nav_next());
    app.ui(|ui| ui.invoke_ai_describe_pressed());
    app.run_jobs();
    assert_eq!(mind.prompts().len(), 2);
    assert_eq!(app.ui(|ui| ui.get_file_name()), "b.png");
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "", "no caption of the new picture from the old one");
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
}

/// An SVG is drawn as the vector it is (sharp at any zoom), measured for the facts its file
/// does not carry, and made into pixels only when the view turns it — as the window always did.
#[test]
fn an_svg_is_drawn_as_a_vector_and_turned_as_pixels() {
    let folder = Folder::new("svg");
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="#ff0000"/></svg>"##;
    std::fs::write(folder.path("c.svg"), svg).unwrap();
    let app = showing(&folder, "c.svg");
    assert!(matches!(app.model(|m| m.shown.picture.clone().unwrap().source), crate::picture::Source::Svg(_)));
    assert_eq!(app.describe()["state"]["dimensions"], json!("4 \u{d7} 2"));
    assert_eq!(app.describe()["state"]["format"], json!("SVG"));
    assert_eq!(drawn(&app).0, 4);
    app.act("rotate", json!({ "direction": "left" })).unwrap();
    app.run_jobs();
    let (w, h, pixels) = drawn(&app);
    assert_eq!((w, h), (2, 4), "turned");
    assert!(pixels.iter().all(|p| p[0] == 255 && p[1] == 0), "still the red the file draws");
}
