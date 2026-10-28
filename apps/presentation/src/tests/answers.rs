//! What each action answers, and refuses with. Every answer is read back out of the deck after the
//! change, not composed from the arguments that went in, and a refusal is said twice: to the
//! caller, and in the window's notice.

use serde_json::json;

use super::{act, current, modified, none, notice, outline, settle, started, state, summary, with_file};

#[test]
fn add_slide_answers_where_the_deck_now_is_and_where_the_slide_went() {
    let app = started();
    let answer = act(&app, "add_slide", json!({ "title": "Second", "body": "two", "notes": "say hi" })).unwrap();
    assert_eq!(answer, json!({ "index": 1, "title": "Second", "slides": 2, "dirty": true, "added_at": 1 }));
    let after = act(&app, "add_slide", json!({ "title": "Inserted", "after": 0 })).unwrap();
    assert_eq!((after["added_at"].clone(), after["index"].clone()), (json!(1), json!(1)));
    assert_eq!(outline(&app), vec!["Title Slide", "Inserted", "Second"]);
    let untitled = act(&app, "add_slide", none()).unwrap();
    assert_eq!(untitled["title"], "Slide 4", "named for its place when it is not named");
    assert_eq!(outline(&app), vec!["Title Slide", "Inserted", "Slide 4", "Second"], "after the one selected");
    let notes = |at: usize| app.model(move |m| m.deck().slides()[at].notes.clone());
    assert_eq!((notes(1), notes(3)), (String::new(), "say hi".to_string()), "no notes unless given");
}

#[test]
fn set_slide_answers_the_slide_as_it_now_is() {
    let app = started();
    let answer = act(&app, "set_slide", json!({ "index": 0, "title": "Opening", "notes": "smile" })).unwrap();
    assert_eq!(
        answer,
        json!({ "index": 0, "title": "Opening", "body": "Click to add subtitle", "notes": "smile", "dirty": true })
    );
    assert_eq!(app.ui(|ui| ui.get_current_title().to_string()), "Opening", "and the canvas shows it");
    let err = act(&app, "set_slide", json!({ "index": 0 })).unwrap_err();
    assert_eq!(err, "give at least one of `title`, `body` or `notes`");
    assert_eq!(notice(&app), err, "said to the window as well");
}

#[test]
fn a_slide_number_is_read_the_way_a_model_writes_it() {
    let app = started();
    act(&app, "add_slide", json!({ "title": "B" })).unwrap();
    act(&app, "go_to", json!({ "index": 0.0 })).expect("2.0 is a whole number");
    assert_eq!(current(&app), 0);
    let fractional = act(&app, "go_to", json!({ "index": 1.5 })).unwrap_err();
    assert!(fractional.contains("must be a whole number"), "{fractional}");
    let negative = act(&app, "go_to", json!({ "index": -1 })).unwrap_err();
    assert_eq!(negative, "`index` is -1; slides are numbered from 0");
    assert_eq!(current(&app), 0, "a refused move moves nothing");
}

#[test]
fn a_slide_that_is_not_there_is_refused_naming_the_deck_it_looked_in() {
    let app = started();
    for (name, args) in [
        ("go_to", json!({ "index": 999 })),
        ("set_slide", json!({ "index": 999, "title": "x" })),
        ("move_slide", json!({ "from": 999, "to": 0 })),
    ] {
        let err = act(&app, name, args).unwrap_err();
        assert!(err.contains("slide 1000") && err.contains("deck of 1"), "{name}: {err}");
    }
    let only = act(&app, "delete_slide", json!({ "index": 0 })).unwrap_err();
    assert!(only.contains("at least one slide"), "{only}");
    assert_eq!(outline(&app), vec!["Title Slide"]);
}

#[test]
fn delete_and_move_answer_what_they_did() {
    let app = started();
    act(&app, "add_slide", json!({ "title": "B" })).unwrap();
    act(&app, "add_slide", json!({ "title": "C" })).unwrap();
    let moved = act(&app, "move_slide", json!({ "from": 2, "to": 0 })).unwrap();
    assert_eq!(moved, json!({ "index": 0, "title": "C", "slides": 3, "dirty": true, "moved_to": 0 }));
    assert_eq!(outline(&app), vec!["C", "Title Slide", "B"]);
    let gone = act(&app, "delete_slide", json!({ "index": 0 })).unwrap();
    assert_eq!(gone["deleted"], "C");
    assert_eq!((gone["slides"].clone(), gone["dirty"].clone()), (json!(2), json!(true)));
    assert_eq!(outline(&app), vec!["Title Slide", "B"]);
}

#[test]
fn next_and_previous_answer_where_they_landed_and_stop_at_the_ends() {
    let app = started();
    act(&app, "add_slide", json!({ "title": "B" })).unwrap();
    let back = act(&app, "previous", none()).unwrap();
    assert_eq!((back["index"].clone(), back["title"].clone()), (json!(0), json!("Title Slide")));
    let stay = act(&app, "previous", none()).unwrap();
    assert_eq!(stay["index"], 0, "stopping at the start");
    let forward = act(&app, "next", none()).unwrap();
    assert_eq!((forward["index"].clone(), forward["title"].clone()), (json!(1), json!("B")));
    assert_eq!(act(&app, "next", none()).unwrap()["index"], 1, "stopping at the end");
}

#[test]
fn the_presenter_view_takes_the_words_a_model_guesses() {
    let app = started();
    for (mode, wanted) in [
        ("fullscreen", true), ("windowed", false), ("on", true), ("off", false), ("start", true), ("stop", false),
        ("true", true), ("false", false), ("toggle", true), ("exit", false), ("  ON ", true),
    ] {
        let answer = act(&app, "presenter_view", json!({ "mode": mode })).unwrap();
        assert_eq!(answer["presenting"], wanted, "{mode}");
        assert_eq!(app.ui(|ui| ui.get_is_presenting()), wanted, "{mode}: and the window shows it");
        assert_eq!(state(&app)["presenting"], wanted, "{mode}");
    }
    let err = act(&app, "presenter_view", json!({ "mode": "zoom" })).unwrap_err();
    for word in ["on", "fullscreen", "windowed", "toggle"] {
        assert!(err.contains(word), "{err}");
    }
}

#[test]
fn new_deck_starts_empty_and_names_itself() {
    let app = started();
    let answer = act(&app, "new_deck", json!({ "title": "Quarterly" })).unwrap();
    assert_eq!(
        answer,
        json!({ "deck_title": "Quarterly", "slides": 1, "index": 0, "previous_deck_saved_to": null })
    );
    assert_eq!(act(&app, "new_deck", json!({ "title": "   " })).unwrap()["deck_title"], "Untitled");
    assert_eq!(act(&app, "new_deck", none()).unwrap()["deck_title"], "Untitled");
    assert!(!modified(&app), "a new deck has nothing unsaved in it");
    assert_eq!(summary(&app), "yPresent \u{2014} Untitled \u{b7} slide 1 of 1");
}

#[test]
fn show_answers_which_deck_is_there() {
    let (app, _) = with_file("show", &["One"]);
    assert_eq!(act(&app, "show", none()).unwrap(), json!({ "showing": "Talk" }));
}

#[test]
fn a_success_clears_the_notice_and_a_refusal_sets_it() {
    let app = started();
    act(&app, "go_to", json!({ "index": 5 })).unwrap_err();
    assert!(notice(&app).contains("slide 6"), "{}", notice(&app));
    act(&app, "go_to", json!({ "index": 0 })).unwrap();
    assert_eq!(notice(&app), "", "what is on screen is about the last thing that happened");
}

#[test]
fn a_call_while_a_file_is_read_or_written_is_refused_and_show_still_answers() {
    let (app, _) = with_file("busy", &["One"]);
    let saving = app.act("save", none()).unwrap();
    assert!(app.model(|m| m.busy.is_some()));
    let err = app.act("add_slide", none()).unwrap_err();
    assert!(err.contains("`add_slide` cannot run while yPresent is reading or writing a file"), "{err}");
    app.act("show", none()).expect("show reads and changes nothing");
    settle(&app);
    saving.settle().expect("the save is answered");
    assert_eq!(outline(&app), vec!["One"], "and the refused call changed nothing");
}

#[test]
fn an_answer_is_its_own_calls_and_nobody_elses() {
    let app = started();
    let first = act(&app, "add_slide", json!({ "title": "B" })).unwrap();
    let second = act(&app, "set_slide", json!({ "index": 0, "title": "Z" })).unwrap();
    assert_eq!(first["title"], "B");
    assert_eq!((second["index"].clone(), second["title"].clone()), (json!(0), json!("Z")));
    assert_eq!(app.model(|m| m.answer.clone()), second, "the model holds the newest, and each call took its own");
}
