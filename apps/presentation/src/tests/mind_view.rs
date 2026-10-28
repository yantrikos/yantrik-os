//! yPresent drawn for a Mind (Mind View): `read_mind_view` reads the window as text, so the window
//! holds no slide text. The deck's name, its slide count and the position stay; the canvas takes
//! no typing; nothing that would change the deck, or read or write a file as the person, runs;
//! and looking at the deck, slide by slide, never writes the mask over a slide.

use std::sync::Arc;

use serde_json::json;
use slint::Model as _;
use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_app::mind::fake::FakeMind;
use yantrik_testing::TestApp;

use super::{act, listing, outline, settle, started};
use crate::command::{Command, WindowAction};
use crate::deck::Edits;
use crate::YPresent;

const SENTINEL: &str = "SENTINEL-ypresent-mind-view-3c9d";

/// A deck of two slides, each carrying the sentinel in its title, body and notes, kept as a draft,
/// and where it is kept.
fn kept() -> yantrik_app::Paths {
    let app = started();
    act(&app, "new_deck", json!({ "title": "Plain deck" })).unwrap();
    act(&app, "set_slide", json!({ "index": 0, "title": format!("one {SENTINEL}"), "body": format!("body {SENTINEL}"), "notes": format!("notes {SENTINEL}") })).unwrap();
    act(&app, "add_slide", json!({ "title": format!("two {SENTINEL}"), "body": format!("second {SENTINEL}") })).unwrap();
    act(&app, "go_to", json!({ "index": 0 })).unwrap();
    app.run_writes();
    let paths = app.paths();
    let _ = app.shutdown();
    paths
}

fn start_for_mind(paths: yantrik_app::Paths) -> TestApp<YPresent> {
    let app = TestApp::<YPresent>::start_for_mind(paths, Vec::new(), Arc::new(FakeMind::new()));
    super::deck_folder(&app);
    settle(&app);
    app
}

/// Every string the window holds that a screen reader or OCR could see.
fn window_text(app: &TestApp<YPresent>) -> String {
    app.ui(|ui| {
        let mut all = vec![
            ui.get_current_title().to_string(),
            ui.get_current_body().to_string(),
            ui.get_current_notes().to_string(),
            ui.get_next_title().to_string(),
            ui.get_next_body().to_string(),
            ui.get_presentation_title().to_string(),
            ui.get_slide_progress().to_string(),
            ui.get_notice().to_string(),
            ui.get_file_path().to_string(),
            ui.get_search_query().to_string(),
        ];
        let rows = ui.get_slides();
        for i in 0..rows.row_count() {
            let r = rows.row_data(i).unwrap();
            all.extend([r.title, r.body, r.notes].map(|s| s.to_string()));
        }
        all.join("\n")
    })
}

fn titles(app: &TestApp<YPresent>) -> Vec<String> {
    outline(app)
}

#[test]
fn a_mind_view_window_holds_no_slide_text_but_keeps_the_decks_name_and_place() {
    let app = start_for_mind(kept());
    let text = window_text(&app);
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(text.contains("Plain deck") && text.contains("Slide 1 of 2"), "{text}");
    let canvas = super::canvas(&app);
    assert_eq!((canvas.title.as_str(), canvas.body.as_str(), canvas.notes.as_str()), (HIDDEN, HIDDEN, HIDDEN));
    assert_eq!(app.ui(|ui| ui.get_next_title().to_string()), HIDDEN, "the next card is masked too");
    assert_eq!(app.ui(|ui| ui.get_slides().row_data(1).unwrap().title.to_string()), HIDDEN);
    assert_eq!(titles(&app), vec![format!("one {SENTINEL}"), format!("two {SENTINEL}")], "the deck itself is whole");
}

#[test]
fn a_normal_window_still_shows_the_deck() {
    let paths = kept();
    let app = TestApp::<YPresent>::start_at(paths, Vec::new());
    settle(&app);
    assert!(window_text(&app).contains(SENTINEL));
}

#[test]
fn typing_in_a_mind_view_window_changes_nothing() {
    let app = start_for_mind(kept());
    let before = titles(&app);
    app.run(Command::Canvas(Edits { title: HIDDEN.into(), body: HIDDEN.into(), notes: HIDDEN.into(), layout: 0 }));
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    assert_eq!(titles(&app), before);
}

#[test]
fn every_edit_and_file_button_in_a_mind_view_window_is_refused_before_anything_changes() {
    let dir_app = start_for_mind(kept());
    let folder = super::deck_folder(&dir_app);
    let before = (titles(&dir_app), dir_app.model(|m| m.deck().content().clone()));
    let actions = [
        WindowAction::AddSlide,
        WindowAction::DeleteSlide,
        WindowAction::DuplicateSlide,
        WindowAction::MoveUp,
        WindowAction::MoveDown,
        WindowAction::SetLayout(2),
        WindowAction::SetTheme(2),
        WindowAction::Save,
        WindowAction::OpenRecent,
        WindowAction::ExportMarkdown,
        WindowAction::ExportOutline,
        WindowAction::Undo,
        WindowAction::Redo,
        WindowAction::GenerateNotes,
    ];
    for action in actions {
        dir_app.run(Command::Window { action: action.clone(), canvas: super::canvas(&dir_app) });
        assert_eq!(dir_app.last_refusal().unwrap().sentence, READ_ONLY, "{action:?}");
    }
    dir_app.run(Command::Launch("/tmp/elsewhere.ydeck".into()));
    assert_eq!(dir_app.last_refusal().unwrap().sentence, READ_ONLY, "a launch's file is read as the person");
    settle(&dir_app);
    assert_eq!((titles(&dir_app), dir_app.model(|m| m.deck().content().clone())), before);
    assert!(listing(&folder).is_empty(), "no deck was written, copied out or exported");
    assert!(dir_app.model(|m| m.asking.is_none()), "and the Mind was not asked about the slide");
}

#[test]
fn looking_at_the_deck_slide_by_slide_never_writes_the_mask_over_a_slide() {
    let app = start_for_mind(kept());
    for action in [
        WindowAction::Next,
        WindowAction::Previous,
        WindowAction::Select(1),
        WindowAction::Last,
        WindowAction::First,
        WindowAction::TogglePresent,
        WindowAction::TogglePresent,
        WindowAction::OpenGallery,
        WindowAction::Escape,
    ] {
        press_masked(&app, action);
    }
    assert_eq!(titles(&app), vec![format!("one {SENTINEL}"), format!("two {SENTINEL}")]);
    let bodies: Vec<String> = app.model(|m| m.deck().slides().iter().map(|s| s.body.clone()).collect());
    assert!(bodies.iter().all(|b| b.contains(SENTINEL)), "{bodies:?}");
    assert!(!app.model(|m| m.deck().content().slides.iter().any(|s| s.title == HIDDEN || s.notes == HIDDEN)));
}

/// Pressed with the canvas the window holds, which is the mask.
fn press_masked(app: &TestApp<YPresent>, action: WindowAction) {
    app.run(Command::Window { action, canvas: super::canvas(app) });
    settle(app);
    assert!(app.last_refusal().is_none_or(|r| r.sentence != READ_ONLY), "looking is not refused");
}

#[test]
fn a_search_in_a_mind_view_window_finds_nothing_because_nothing_it_could_match_is_shown() {
    let app = start_for_mind(kept());
    press_masked(&app, WindowAction::Search("sentinel".into()));
    assert_eq!(app.ui(|ui| ui.get_search_count()), 0);
    assert_eq!(app.model(|m| m.hits.len()), 0);
}

#[test]
fn masking_is_visual_only_describe_and_calls_are_as_for_a_normal_instance() {
    let paths = kept();
    let what_a_caller_gets = |app: &TestApp<YPresent>| {
        let d = app.describe();
        let moved = act(app, "go_to", json!({ "index": 1 })).unwrap();
        json!({ "summary": d["summary"], "state": d["state"], "moved": moved })
    };
    let normal = TestApp::<YPresent>::start_at(paths.clone(), Vec::new());
    settle(&normal);
    let expected = what_a_caller_gets(&normal);
    let _ = normal.shutdown();
    drop(normal);
    let masked = start_for_mind(paths);
    let got = what_a_caller_gets(&masked);
    assert!(expected.to_string().contains(SENTINEL), "a Mind still reads the deck through describe");
    // The state differs only in where the deck's draft lives (each start made its own) and the
    // folder; the content and the answers are the same.
    assert_eq!(got["summary"], expected["summary"]);
    assert_eq!(got["state"]["current"], expected["state"]["current"]);
    assert_eq!(got["state"]["outline"], expected["state"]["outline"]);
    assert_eq!(got["moved"], expected["moved"]);
    // And the intents still change the deck, under the agent rules, for the Mind that drives it.
    act(&masked, "set_slide", json!({ "index": 0, "title": "changed by the Mind" })).unwrap();
    assert_eq!(outline(&masked)[0], "changed by the Mind");
    assert_eq!(app_text(&masked), HIDDEN, "and the window still shows no text");
}

fn app_text(app: &TestApp<YPresent>) -> String {
    app.ui(|ui| ui.get_slides().row_data(0).unwrap().title.to_string())
}

#[test]
fn a_hidden_deck_has_no_name_path_or_notice_in_a_mind_view_window() {
    // A deck outside the home is one an agent is not shown. Its draft is kept, and a Mind's window
    // starts on the same state.
    let (app, path) = super::with_file("mind-hidden", &["Plain"]);
    if path.starts_with(std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())) {
        return;
    }
    super::type_title(&app, &format!("edited {SENTINEL}"));
    app.run_writes();
    let paths = app.paths();
    let _ = app.shutdown();
    let mind = start_for_mind(paths);
    let text = window_text(&mind);
    let dir = path.parent().unwrap().display().to_string();
    assert!(!text.contains(&dir) && !text.contains("talk.ydeck"), "{text}");
    assert!(!text.contains("Talk"), "the deck's name is hidden too: {text}");
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(text.contains(HIDDEN), "{text}");
    assert_eq!(mind.ui(|ui| ui.get_presentation_title().to_string()), HIDDEN);
    assert_eq!(mind.ui(|ui| ui.get_file_path().to_string()), HIDDEN);
    assert!(!mind.ui(|ui| ui.get_notice().to_string()).contains(&dir));
}
