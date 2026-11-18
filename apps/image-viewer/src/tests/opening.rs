//! Opening a picture and moving through its folder: what is drawn, what `describe` says, and
//! what each action answers, in every case the app has always had (nothing open, a picture, a
//! file that is not one, a file that is not there), plus the launch, the stale result, the
//! panicking worker and the shutdown mid-load.

use serde_json::json;
use slint::Model as _;
use yantrik_app::launch::Launch;

use super::fixture::{self, drawn, open, showing, start, start_with, Folder};
use crate::model::{NOTHING_OPEN, PART_NOTHING};
use crate::{Command, NO_PICTURE};

#[test]
fn nothing_open_is_said_as_it_always_was_and_the_view_controls_refuse() {
    let (app, _) = start();
    let d = app.describe();
    assert_eq!(d["summary"], json!("Images — nothing open"));
    let s = &d["state"];
    assert_eq!(s["file"], json!(""));
    assert_eq!(s["path"], json!(null));
    assert_eq!((s["position"].clone(), s["of"].clone()), (json!(1), json!(0)));
    assert_eq!(s["notice"], json!(NOTHING_OPEN));
    assert_eq!(s["view_controls_off"], json!(NO_PICTURE));
    assert_eq!(s["folder"], json!([]));
    assert_eq!(s["fit_to_window"], json!(true));
    for key in ["zoomed", "info_panel", "slideshow", "flipped_horizontally", "flipped_vertically"] {
        assert_eq!(s[key], json!(false), "{key}");
    }
    assert_eq!(s["rotation"], json!(0));
    for key in ["dimensions", "file_size", "format", "taken", "camera"] {
        assert_eq!(s[key], json!(""), "{key}");
    }
    assert_eq!(app.ui(|ui| ui.get_notice()), NOTHING_OPEN);
    assert_eq!(app.ui(|ui| ui.get_file_name()), "");
    assert_eq!(drawn(&app).0, 0, "no picture is drawn");

    // Every view control refuses with the window's own words, and nothing changes.
    for (action, args) in [
        ("rotate", json!({ "direction": "left" })),
        ("flip", json!({ "direction": "horizontal" })),
        ("fit", json!({})),
        ("toggle_info", json!({})),
    ] {
        assert_eq!(app.act(action, args).unwrap_err(), NO_PICTURE, "{action}");
    }
    for action in ["next", "previous"] {
        assert_eq!(app.act(action, json!({})).unwrap_err(), "no pictures are open", "{action}");
    }
    // A bad direction with nothing open is refused for the missing picture first, as before.
    assert_eq!(app.act("rotate", json!({ "direction": "up" })).unwrap_err(), NO_PICTURE);
    assert_eq!(app.describe()["state"]["info_panel"], json!(false));
    // The window's arrows do nothing either: there is nothing to step through.
    app.ui(|ui| ui.invoke_nav_next());
    assert_eq!(app.jobs_waiting(), 0);
}

#[test]
fn open_shows_the_picture_and_the_folder_it_is_in_once_it_has_loaded() {
    let folder = Folder::new("open");
    let (app, _) = start();
    let pending = app.act("open", json!({ "path": folder.text("b.png") })).unwrap();
    assert!(pending.is_owed(), "answered when the picture has loaded");
    assert_eq!(app.ui(|ui| ui.get_file_name()), "", "the screen changes when the load lands, at once");
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "showing": "b.png", "of": 3 }));

    let d = app.describe();
    assert_eq!(d["summary"], json!("Images — b.png, 2 of 3"));
    let s = &d["state"];
    assert_eq!(s["file"], json!("b.png"));
    assert_eq!(s["path"], json!(folder.text("b.png")));
    assert_eq!((s["position"].clone(), s["of"].clone()), (json!(2), json!(3)));
    assert_eq!(s["dimensions"], json!("2 \u{d7} 2"), "read off the decoded picture when EXIF is silent");
    assert_eq!(s["format"], json!("PNG"));
    assert!(s["file_size"].as_str().unwrap().ends_with(" B"), "{}", s["file_size"]);
    assert_eq!(s["notice"], json!(""));
    assert_eq!(s["view_controls_off"], json!(null));
    assert_eq!(s["folder"], json!(["a.png", "b.png", "notes.png"]), "pictures only, in name order");

    assert_eq!(app.ui(|ui| ui.get_file_name()), "b.png");
    assert_eq!(app.ui(|ui| ui.get_counter_text()), "2 / 3");
    assert_eq!(app.ui(|ui| ui.get_viewer_exif_format()), "PNG");
    assert_eq!(drawn(&app).0, 2);
    assert_eq!(app.model(|m| m.raises), 1, "an asked-for open brings the window forward");

    // The rail follows, after its debounce: the picture by name and size, and nothing to ask.
    app.advance(std::time::Duration::from_millis(300));
    assert_eq!(app.ui(|ui| ui.get_agent_context().row_count()), 1);
    let row = app.ui(|ui| ui.get_agent_context().row_data(0).unwrap());
    assert_eq!((row.label.as_str(), row.detail.as_str(), row.source.as_str()), ("b.png", "2 \u{d7} 2", "file"));
    assert_eq!(app.ui(|ui| ui.get_agent_unavailable()), "");

    // Opening one already in the folder selects it; `show` answers with it.
    assert_eq!(open(&app, &folder.path("a.png")).unwrap(), json!({ "showing": "a.png", "of": 3 }));
    let shown = app.act("show", json!({})).unwrap().settle().unwrap();
    assert_eq!(shown, json!({ "showing": "a.png" }));
    assert_eq!(app.model(|m| m.raises), 3);
}

#[test]
fn a_file_that_is_not_a_picture_is_named_and_one_that_is_not_there_is_refused() {
    let folder = Folder::new("bad");
    let app = showing(&folder, "notes.png");
    let d = app.describe();
    assert_eq!(d["state"]["file"], json!("notes.png"));
    let said = format!("Could not open {}", folder.text("notes.png"));
    assert_eq!(d["state"]["notice"], json!(said));
    assert_eq!(d["state"]["view_controls_off"], json!(NO_PICTURE));
    assert_eq!(app.ui(|ui| ui.get_notice()), said.as_str(), "beside the name, on screen");
    assert_eq!(app.ui(|ui| ui.get_file_name()), "notes.png");
    assert_eq!(drawn(&app).0, 0);
    assert_eq!(app.act("rotate", json!({ "direction": "left" })).unwrap_err(), NO_PICTURE);

    let missing = folder.path("gone.png");
    let e = open(&app, &missing).unwrap_err();
    assert_eq!(e, format!("no file at {}", missing.display()));
    assert_eq!(app.describe()["state"]["file"], json!("notes.png"), "the window is left as it was");
    assert_eq!(app.act("open", json!({ "path": "  " })).unwrap_err(), "`path` is empty");
    assert!(app.act("open", json!({})).is_err(), "the path is needed");
}

#[test]
fn next_and_previous_step_through_the_folder_wrapping_at_its_ends() {
    let folder = Folder::new("step");
    let app = showing(&folder, "a.png");
    let pending = app.act("next", json!({})).unwrap();
    assert!(pending.is_owed());
    assert_eq!(app.ui(|ui| ui.get_file_name()), "a.png", "until the next one lands");
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "showing": "b.png", "position": 2 }));
    assert_eq!(app.ui(|ui| ui.get_counter_text()), "2 / 3");

    for _ in 0..2 {
        let p = app.act("previous", json!({})).unwrap();
        app.run_jobs();
        p.settle().unwrap();
    }
    assert_eq!(app.describe()["summary"], json!("Images — notes.png, 3 of 3"), "wrapped from the first to the last");

    // The window's own arrows are the same commands.
    app.ui(|ui| ui.invoke_nav_next());
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "a.png");
    app.ui(|ui| ui.invoke_nav_prev());
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "notes.png");
}

#[test]
fn a_slow_picture_never_lands_over_a_newer_one() {
    let folder = Folder::new("stale");
    let app = showing(&folder, "a.png");
    // Two steps before either picture has loaded: the first one's result is not shown, however
    // late it comes and in whatever order.
    app.run(Command::Next);
    let first = app.model(|m| m.load);
    app.run(Command::Next);
    let late = crate::open::load(&folder.path("b.png"));
    app.run(Command::Loaded { load: first, loaded: late.clone() });
    assert_eq!(app.ui(|ui| ui.get_file_name()), "a.png", "a stale result is dropped");
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "notes.png");
    app.run(Command::Loaded { load: first, loaded: late });
    assert_eq!(app.ui(|ui| ui.get_file_name()), "notes.png", "and stays dropped after the newer one");
    assert_eq!(app.describe()["state"]["position"], json!(3));

    // An open that fails after a step it replaced: the step is taken up again, so the folder's
    // current picture is the one on screen.
    app.run(Command::Previous);
    let e = open(&app, &folder.path("gone.png")).unwrap_err();
    assert!(e.contains("no file at"), "{e}");
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "b.png");

    // A dropped result redraws nothing.
    let mut fx = yantrik_app::Effects::<crate::ImageViewer>::recording();
    let mut m = crate::Model::new(std::sync::Arc::new(yantrik_app::mind::fake::FakeMind::new()));
    let stale = Command::Loaded { load: 99, loaded: crate::open::load(&folder.path("a.png")) };
    crate::update::update(&mut m, stale, &mut fx).unwrap();
    assert_eq!(fx.changed_parts(), &yantrik_app::Changed::Parts(vec![PART_NOTHING]));
    assert!(fx.requests().is_empty());
}

#[test]
fn the_first_launch_opens_its_file_and_a_second_launch_opens_in_the_same_window() {
    let folder = Folder::new("launch");
    let (app, _) = start_with(Default::default(), vec![folder.path("b.png").into()]);
    app.run_jobs();
    assert_eq!(app.describe()["summary"], json!("Images — b.png, 2 of 3"));
    assert_eq!(app.model(|m| m.raises), 0, "a window just made is not raised again");

    // A second launch hands its file over; a retry of the same hand-over opens nothing twice.
    let handed = Launch::of_files("default", std::path::Path::new("/"), [folder.path("a.png")]);
    app.act("launch", json!({ "launch": handed })).unwrap();
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "a.png");
    app.act("launch", json!({ "launch": handed })).unwrap();
    assert_eq!(app.jobs_waiting(), 0, "the retry is answered, not applied");
    // One that names a file that is not there leaves the window as it was.
    let gone = Launch::of_files("default", std::path::Path::new("/"), [folder.path("gone.png")]);
    app.act("launch", json!({ "launch": gone })).unwrap();
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "a.png");
    // One that names nothing only brings the window forward (the framework's `present`).
    let nothing = Launch::of_files("default", std::path::Path::new("/"), Vec::new());
    app.act("launch", json!({ "launch": nothing })).unwrap();
    assert_eq!(app.jobs_waiting(), 0);

    // The first launch's missing file is still named on screen, with the folder beside it.
    let (app, _) = start_with(Default::default(), vec![folder.path("gone.png").into()]);
    app.run_jobs();
    let s = app.describe()["state"].clone();
    assert_eq!(s["file"], json!("gone.png"));
    assert_eq!(s["notice"], json!(format!("Could not open {}", folder.text("gone.png"))));
    assert_eq!(s["of"], json!(4), "the missing file, then the folder's three");
}

#[test]
fn a_decoder_that_panics_and_a_shutdown_mid_load_are_each_held() {
    let folder = Folder::new("panic");
    std::fs::write(folder.path(crate::open::PANICS), fixture::png(1, 1, fixture::pixel)).unwrap();
    let (app, _) = start();
    // An open whose worker panics: the call is told, the window is left as it was.
    let pending = app.act("open", json!({ "path": folder.text(crate::open::PANICS) })).unwrap();
    app.run_jobs();
    let e = pending.settle().unwrap_err();
    assert!(e.contains("the work failed"), "{e}");
    assert_eq!(app.describe()["summary"], json!("Images — nothing open"));
    // A step onto it: named on screen as a file that cannot be shown, and the app keeps going.
    // (The folder: a.png, b.png, decoder-panics.png, notes.png.)
    open(&app, &folder.path("notes.png")).unwrap();
    app.run(Command::Previous);
    app.run_jobs();
    let s = app.describe()["state"].clone();
    assert_eq!(s["file"], json!(crate::open::PANICS));
    assert_eq!(s["notice"], json!(format!("Could not open {}", folder.text(crate::open::PANICS))));
    app.run(Command::Previous);
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_file_name()), "b.png");

    // A shutdown while an open is in flight: the owed answer is refused as a closing, and the
    // end comes within the grace period.
    let owed = app.act("open", json!({ "path": folder.text("b.png") })).unwrap();
    assert!(owed.is_owed());
    let began = std::time::Instant::now();
    let _ = app.shutdown();
    assert!(began.elapsed() < std::time::Duration::from_secs(2), "shutdown must not hang on the job");
    assert_eq!(owed.settle().unwrap_err(), "the app is closing");
}
