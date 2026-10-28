//! Speaker notes from the companion: asked in a job, answered into the slide it was asked about
//! and to no other, and landing unsaved so it is read before it is kept.

use std::sync::Arc;

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::AskError;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{act, canvas, current, modified, notice, press, settle, type_body, type_notes};
use crate::command::WindowAction;
use crate::YPresent;

/// A deck of two slides, selected on the first, with a Mind that gives these answers.
fn asked(mind: &Arc<FakeMind>) -> TestApp<YPresent> {
    let app = TestApp::<YPresent>::start_at_with(
        Paths::temporary("presentation"),
        Vec::new(),
        mind.clone(),
        yantrik_app::services::Services::fakes(),
    );
    super::deck_folder(&app);
    settle(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "Why now", "body": "Costs fell by half" })).unwrap();
    act(&app, "add_slide", json!({ "title": "Next steps", "body": "Hire two people" })).unwrap();
    act(&app, "go_to", json!({ "index": 0 })).unwrap();
    app
}

fn notes_of(app: &TestApp<YPresent>, index: usize) -> String {
    app.model(move |m| m.deck().slides()[index].notes.clone())
}

#[test]
fn the_answer_is_written_to_the_slide_unsaved_and_the_prompt_holds_only_that_slide() {
    let mind = Arc::new(FakeMind::new().answering("  Open with the cost curve.\n"));
    let app = asked(&mind);
    press(&app, WindowAction::GenerateNotes);
    assert_eq!(notes_of(&app, 0), "Open with the cost curve.", "trimmed, and in the slide it was asked about");
    assert_eq!(canvas(&app).notes, "Open with the cost curve.", "and on the canvas");
    assert_eq!(notice(&app), "");
    assert!(modified(&app), "unsaved, so it is read before it is kept");
    let prompts = mind.prompts();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("Title: Why now") && prompts[0].contains("Costs fell by half"), "{}", prompts[0]);
    assert!(!prompts[0].contains("Hire two people"), "nothing of any other slide goes with it");
    assert!(prompts[0].contains("invent nothing"));
}

#[test]
fn the_answer_goes_to_the_slide_it_was_asked_about_though_the_person_moved_on() {
    let mind = Arc::new(FakeMind::new().answering("Notes for the first."));
    let app = asked(&mind);
    let canvas_now = canvas(&app);
    app.run(crate::command::Command::Window { action: WindowAction::GenerateNotes, canvas: canvas_now });
    assert_eq!(notice(&app), "Asking the companion for speaker notes\u{2026}");
    // Before the Mind answers, the person goes to the next slide and starts on its notes.
    press(&app, WindowAction::Next);
    type_notes(&app, "my own notes on slide two");
    settle(&app);
    assert_eq!(notes_of(&app, 0), "Notes for the first.");
    assert_eq!(notes_of(&app, 1), "my own notes on slide two", "the slide in front is untouched");
    assert_eq!(current(&app), 1);
    assert_eq!(canvas(&app).notes, "my own notes on slide two");
}

#[test]
fn an_answer_for_a_slide_that_changed_meanwhile_is_not_written_over_it() {
    let mind = Arc::new(FakeMind::new().answering("Stale notes."));
    let app = asked(&mind);
    let canvas_now = canvas(&app);
    app.run(crate::command::Command::Window { action: WindowAction::GenerateNotes, canvas: canvas_now });
    type_body(&app, "Costs fell by a third, not half");
    settle(&app);
    assert_eq!(notes_of(&app, 0), "", "the slide no longer says what the notes were written from");
    assert_eq!(notice(&app), "The slide changed while the companion was writing, so its notes were not added.");
    // And a slide that moved away from its place is not mistaken for the one asked about.
    let mind = Arc::new(FakeMind::new().answering("Stale again."));
    let app = asked(&mind);
    let canvas_now = canvas(&app);
    app.run(crate::command::Command::Window { action: WindowAction::GenerateNotes, canvas: canvas_now });
    press(&app, WindowAction::MoveDown);
    settle(&app);
    assert_eq!((notes_of(&app, 0), notes_of(&app, 1)), (String::new(), String::new()));
}

#[test]
fn a_slide_with_no_text_is_not_asked_about_and_a_second_request_waits_for_the_first() {
    let mind = Arc::new(FakeMind::new().answering("Notes."));
    let app = asked(&mind);
    act(&app, "set_slide", json!({ "index": 0, "title": " ", "body": "" })).unwrap();
    press(&app, WindowAction::GenerateNotes);
    assert_eq!(notice(&app), "This slide has no text to write notes from.");
    assert!(mind.prompts().is_empty());
    act(&app, "set_slide", json!({ "index": 0, "title": "Back" })).unwrap();
    let canvas_now = canvas(&app);
    app.run(crate::command::Command::Window { action: WindowAction::GenerateNotes, canvas: canvas_now.clone() });
    app.run(crate::command::Command::Window { action: WindowAction::GenerateNotes, canvas: canvas_now });
    assert_eq!(notice(&app), "Still waiting for the companion's speaker notes.");
    settle(&app);
    assert_eq!(mind.prompts().len(), 1, "asked once");
}

#[test]
fn a_mind_that_cannot_answer_is_said_and_the_slide_is_left_alone() {
    let mind = Arc::new(FakeMind::new().answers([Err(AskError::Failed("the shell is not running".into()))]));
    let app = asked(&mind);
    press(&app, WindowAction::GenerateNotes);
    assert!(notice(&app).contains("the shell is not running"), "{}", notice(&app));
    assert_eq!(notes_of(&app, 0), "");
    assert!(app.model(|m| m.asking.is_none()), "and it can be asked again");
    press(&app, WindowAction::GenerateNotes);
    assert_eq!(mind.prompts().len(), 2);
}

#[test]
fn an_answer_for_a_deck_that_was_replaced_is_not_written_into_the_next_one() {
    let mind = Arc::new(FakeMind::new().answering("For the old deck."));
    let app = asked(&mind);
    // A saved deck, so replacing it needs no write of its own first.
    let path = super::scratch("notes-replaced").join("talk.ydeck");
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    let canvas_now = canvas(&app);
    app.run(crate::command::Command::Window { action: WindowAction::GenerateNotes, canvas: canvas_now });
    // Before the answer, the deck is replaced by a blank one whose first slide reads the same.
    act(&app, "new_deck", json!({ "title": "Second" })).unwrap();
    act(&app, "set_slide", json!({ "index": 0, "title": "Why now", "body": "Costs fell by half" })).unwrap();
    settle(&app);
    assert_eq!(notes_of(&app, 0), "");
    assert!(app.model(|m| m.asking.is_none()));
}
