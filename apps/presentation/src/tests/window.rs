//! The window: what the buttons, keys and typing do to the deck, and what the window shows of it.
//! The fault this app was first fixed for (Next and Previous moved the index and nothing else)
//! is written out through the window's own callbacks.

use serde_json::json;
use slint::Model as _;
use std::time::Duration;
use yantrik_app::app::WindowId;
use yantrik_testing::TestApp;

use super::{act, canvas, current, modified, none, notice, outline, press, settle, started, state, summary};
use super::{type_body, type_notes, type_title};
use crate::command::WindowAction;
use crate::YPresent;

/// A deck of three slides, A, B and C, selected on the first.
fn three() -> TestApp<YPresent> {
    let app = started();
    act(&app, "set_slide", json!({ "index": 0, "title": "A", "body": "body a", "notes": "note a" })).unwrap();
    act(&app, "add_slide", json!({ "title": "B", "body": "body b" })).unwrap();
    act(&app, "add_slide", json!({ "title": "C", "body": "body c" })).unwrap();
    act(&app, "go_to", json!({ "index": 0 })).unwrap();
    app
}

/// A callback on the window, pressed.
fn click(app: &TestApp<YPresent>, f: impl FnOnce(&crate::PresentationApp) + Send + 'static) {
    app.ui(f);
    settle(app);
}

fn rows(app: &TestApp<YPresent>) -> Vec<String> {
    app.ui(|ui| ui.get_slides().iter().map(|r| r.title.to_string()).collect())
}

// ── The bug ──────────────────────────────────────────────────────────

#[test]
fn next_then_select_does_not_write_one_slide_over_the_next() {
    let app = started();
    act(&app, "add_slide", json!({ "title": "Slide 2", "body": "second body" })).unwrap();
    click(&app, |ui| ui.invoke_select_slide(0));

    // The person types into slide 1, then presses Next.
    type_title(&app, "Opening");
    type_body(&app, "What this talk is about");
    type_notes(&app, "smile");
    click(&app, |ui| ui.invoke_next_slide());

    let shown = canvas(&app);
    assert_eq!(shown.title, "Slide 2", "the canvas follows the selection");
    assert_eq!(shown.body, "second body");

    // The thumbnail click that used to commit slide 1's text into slide 2.
    click(&app, |ui| ui.invoke_select_slide(0));
    let first = canvas(&app);
    assert_eq!((first.title.as_str(), first.body.as_str(), first.notes.as_str()), ("Opening", "What this talk is about", "smile"));
    assert_eq!(outline(&app), vec!["Opening", "Slide 2"]);
    let second = app.model(|m| m.deck().slides()[1].clone());
    assert_eq!((second.title.as_str(), second.body.as_str()), ("Slide 2", "second body"));
}

#[test]
fn text_the_screen_has_not_announced_yet_is_still_committed_before_the_selection_moves() {
    // Typed, and the model has not heard of it: the move itself carries what the canvas holds.
    let app = three();
    app.ui(|ui| ui.set_current_title("typed, not announced".into()));
    click(&app, |ui| ui.invoke_next_slide());
    assert_eq!(outline(&app), vec!["typed, not announced", "B", "C"]);
    assert_eq!(canvas(&app).title, "B", "and the canvas is on the slide moved to");
}

#[test]
fn every_way_of_moving_commits_what_is_on_the_canvas_first() {
    type Press = fn(&crate::PresentationApp);
    let paths: [(&str, Press); 9] = [
        ("next", |ui| ui.invoke_next_slide()),
        ("previous", |ui| ui.invoke_prev_slide()),
        ("select", |ui| ui.invoke_select_slide(2)),
        ("first", |ui| ui.invoke_key_pressed("first".into())),
        ("last", |ui| ui.invoke_key_pressed("last".into())),
        ("add", |ui| ui.invoke_add_slide()),
        ("duplicate", |ui| ui.invoke_duplicate_slide()),
        ("move down", |ui| ui.invoke_move_slide_down()),
        ("present", |ui| ui.invoke_toggle_present()),
    ];
    for (name, path) in paths {
        let app = three();
        click(&app, |ui| ui.invoke_select_slide(1));
        type_title(&app, "B, as typed");
        click(&app, path);
        assert!(outline(&app).contains(&"B, as typed".to_string()), "{name}: {:?}", outline(&app));
        assert_eq!(outline(&app).iter().filter(|t| t.as_str() == "A").count(), 1, "{name}: A is untouched");
    }
}

#[test]
fn the_keys_move_and_stop_at_the_ends() {
    let app = three();
    click(&app, |ui| ui.invoke_key_pressed("last".into()));
    assert_eq!(current(&app), 2);
    click(&app, |ui| ui.invoke_key_pressed("next".into()));
    assert_eq!(current(&app), 2, "stopping at the end rather than wrapping to the title slide");
    click(&app, |ui| ui.invoke_key_pressed("first".into()));
    click(&app, |ui| ui.invoke_key_pressed("previous".into()));
    assert_eq!(current(&app), 0);
    click(&app, |ui| ui.invoke_key_pressed("no such key".into()));
    assert_eq!(current(&app), 0, "an unknown key does nothing");
}

// ── Editing ──────────────────────────────────────────────────────────

#[test]
fn typing_reaches_the_deck_says_unsaved_and_arms_the_draft() {
    let (app, _) = super::with_file("typing", &["One"]);
    assert!(!modified(&app));
    type_title(&app, "One, edited");
    assert!(modified(&app));
    assert_eq!(app.ui(|ui| (ui.get_is_modified(), ui.get_save_status().to_string())), (true, "Unsaved changes".to_string()));
    assert_eq!(rows(&app), vec!["One, edited"], "the rail follows what is typed");
    app.run_writes();
    assert!(super::draft_file(&app).exists(), "the draft is in the lane");
}

#[test]
fn a_render_that_is_not_about_the_canvas_leaves_what_is_typed_alone() {
    let app = three();
    // Typed, and not yet told to the model: a command that is not about the canvas must not put
    // the model's text back over it, or the cursor would jump to the start of the field.
    app.ui(|ui| ui.set_current_title("typing, not yet told".into()));
    app.run(crate::command::Command::Tick);
    press(&app, WindowAction::ToggleTimer);
    press(&app, WindowAction::ResetTimer);
    assert_eq!(canvas(&app).title, "typing, not yet told");
}

#[test]
fn a_layout_is_part_of_the_slide() {
    let app = three();
    click(&app, |ui| ui.invoke_set_layout(3));
    assert_eq!(app.model(|m| m.deck().current_slide().layout), 3);
    assert_eq!(canvas(&app).layout, 3);
    click(&app, |ui| ui.invoke_set_layout(99));
    assert_eq!(app.model(|m| m.deck().current_slide().layout), crate::deck::LAYOUT_COUNT - 1, "clamped");
}

#[test]
fn undo_and_redo_step_over_the_decks_content() {
    let app = three();
    click(&app, |ui| ui.invoke_delete_slide());
    assert_eq!(outline(&app), vec!["B", "C"]);
    click(&app, |ui| ui.invoke_undo());
    assert_eq!(outline(&app), vec!["A", "B", "C"]);
    assert_eq!(rows(&app), vec!["A", "B", "C"]);
    click(&app, |ui| ui.invoke_redo());
    assert_eq!(outline(&app), vec!["B", "C"]);
    click(&app, |ui| ui.invoke_redo());
    assert_eq!(notice(&app), "Nothing to redo.");
    let fresh = started();
    click(&fresh, |ui| ui.invoke_undo());
    assert_eq!(notice(&fresh), "Nothing to undo.");
}

#[test]
fn the_buttons_make_move_copy_and_remove_slides() {
    let app = three();
    click(&app, |ui| ui.invoke_add_slide());
    assert_eq!(outline(&app), vec!["A", "Slide 4", "B", "C"]);
    assert_eq!(current(&app), 1, "the new slide is selected");
    click(&app, |ui| ui.invoke_move_slide_up());
    assert_eq!(outline(&app), vec!["Slide 4", "A", "B", "C"]);
    click(&app, |ui| ui.invoke_move_slide_up());
    assert_eq!(current(&app), 0, "the top stays the top");
    click(&app, |ui| ui.invoke_move_slide_down());
    click(&app, |ui| ui.invoke_duplicate_slide());
    assert_eq!(outline(&app), vec!["A", "Slide 4", "Slide 4", "B", "C"]);
    click(&app, |ui| ui.invoke_delete_slide());
    assert_eq!(outline(&app), vec!["A", "Slide 4", "B", "C"]);
    let only = started();
    click(&only, |ui| ui.invoke_delete_slide());
    assert_eq!(outline(&only).len(), 1, "a deck keeps one slide");
    assert!(notice(&only).contains("at least one slide"), "{}", notice(&only));
}

// ── What the window shows ────────────────────────────────────────────

#[test]
fn the_window_shows_the_progress_the_next_card_and_the_deck() {
    let app = three();
    let shown = |app: &TestApp<YPresent>| {
        app.ui(|ui| (ui.get_slide_progress().to_string(), ui.get_next_title().to_string(), ui.get_next_body().to_string(), ui.get_slide_count()))
    };
    assert_eq!(shown(&app), ("Slide 1 of 3".into(), "B".into(), "body b".into(), 3));
    click(&app, |ui| ui.invoke_key_pressed("last".into()));
    assert_eq!(shown(&app), ("Slide 3 of 3".into(), "End of deck".into(), String::new(), 3));
    assert_eq!(app.ui(|ui| ui.get_presentation_title().to_string()), "Untitled");
}

#[test]
fn the_save_status_says_where_the_deck_stands() {
    let (app, path) = super::with_file("status", &["One"]);
    let status = |app: &TestApp<YPresent>| app.ui(|ui| (ui.get_save_status().to_string(), ui.get_file_path().to_string()));
    assert_eq!(status(&app), ("Saved".into(), path.display().to_string()));
    type_title(&app, "Two");
    assert_eq!(status(&app).0, "Unsaved changes");
    press(&app, WindowAction::Save);
    assert_eq!(status(&app).0, "Saved");
    let fresh = started();
    assert_eq!(status(&fresh), ("Not saved yet".into(), String::new()));
}

#[test]
fn the_chosen_theme_is_the_one_the_screen_draws_and_the_gallery_marks_it() {
    let app = three();
    click(&app, |ui| ui.invoke_set_theme(2));
    let (name, current_theme, marks) = app.ui(|ui| {
        let swatches: Vec<(String, bool)> = ui.get_theme_swatches().iter().map(|t| (t.name.to_string(), t.is_active)).collect();
        (ui.get_themes().row_data(0).unwrap().name.to_string(), ui.get_current_theme(), swatches)
    });
    assert_eq!((name.as_str(), current_theme), ("Ink", 2));
    assert_eq!(marks.iter().filter(|(_, on)| *on).map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["Ink"]);
    assert_eq!(marks.len(), crate::deck::THEMES.len());
    assert_eq!(state(&app)["theme"], "Ink");
    click(&app, |ui| ui.invoke_pres_select_template(4));
    assert_eq!(state(&app)["theme"], "Plum");
    assert!(modified(&app), "the theme is part of the deck");
}

#[test]
fn the_gallery_opens_and_escape_puts_the_overlays_away() {
    let app = three();
    click(&app, |ui| ui.invoke_pres_open_template_gallery());
    assert!(app.ui(|ui| ui.get_pres_template_gallery_open()));
    app.ui(|ui| ui.set_show_search(true));
    click(&app, |ui| ui.invoke_key_pressed("escape".into()));
    assert_eq!(app.ui(|ui| (ui.get_pres_template_gallery_open(), ui.get_show_search())), (false, false));
}

#[test]
fn presenting_is_the_models_and_escape_leaves_it() {
    let app = three();
    click(&app, |ui| ui.invoke_toggle_present());
    assert!(app.ui(|ui| ui.get_is_presenting()));
    assert_eq!(state(&app)["presenting"], true);
    click(&app, |ui| ui.invoke_key_pressed("next".into()));
    assert_eq!(current(&app), 1, "the keys drive the presenter view");
    click(&app, |ui| ui.invoke_key_pressed("escape".into()));
    assert!(!app.ui(|ui| ui.get_is_presenting()));
    assert_eq!(state(&app)["presenting"], false);
    click(&app, |ui| ui.invoke_toggle_present());
    click(&app, |ui| ui.invoke_toggle_present());
    assert!(!app.ui(|ui| ui.get_is_presenting()), "the button toggles");
}

#[test]
fn search_finds_counts_and_steps_round() {
    let app = three();
    click(&app, |ui| ui.invoke_search_slides("BODY".into()));
    assert_eq!(app.ui(|ui| (ui.get_search_count(), ui.get_search_query().to_string())), (3, "BODY".to_string()));
    assert_eq!(current(&app), 0);
    click(&app, |ui| ui.invoke_search_next());
    click(&app, |ui| ui.invoke_search_next());
    click(&app, |ui| ui.invoke_search_next());
    assert_eq!(current(&app), 0, "wraps round to the first match");
    click(&app, |ui| ui.invoke_search_slides("body c".into()));
    assert_eq!(current(&app), 2);
    click(&app, |ui| ui.invoke_search_slides("zzz".into()));
    assert_eq!(notice(&app), "No slide holds \u{201c}zzz\u{201d}.");
    assert_eq!(app.ui(|ui| ui.get_search_count()), 0);
    click(&app, |ui| ui.invoke_search_next());
    assert_eq!(notice(&app), "Nothing to step through; search first.");
}

// ── The presenter's clock ────────────────────────────────────────────

#[test]
fn the_clock_ticks_only_while_it_runs_and_a_deck_at_rest_has_no_timer() {
    let app = started();
    assert_eq!(app.timers_armed(), 0, "nothing is armed at rest");
    click(&app, |ui| ui.invoke_toggle_timer());
    assert_eq!(app.timers_armed(), 1);
    app.advance(Duration::from_secs(3));
    assert_eq!(app.ui(|ui| ui.get_timer_text().to_string()), "00:03");
    assert_eq!((state(&app)["timer"].clone(), state(&app)["timer_running"].clone()), (json!("00:03"), json!(true)));
    click(&app, |ui| ui.invoke_toggle_timer());
    assert_eq!(app.timers_armed(), 0, "stopped is not ticking");
    app.advance(Duration::from_secs(5));
    assert_eq!(state(&app)["timer"], "00:03");
    click(&app, |ui| ui.invoke_toggle_timer());
    app.advance(Duration::from_secs(2));
    click(&app, |ui| ui.invoke_reset_timer());
    assert_eq!((state(&app)["timer"].clone(), state(&app)["timer_running"].clone()), (json!("00:00"), json!(false)));
    assert_eq!(app.timers_armed(), 0);
}

#[test]
fn the_clock_does_not_make_the_deck_unsaved() {
    let (app, _) = super::with_file("clock", &["One"]);
    click(&app, |ui| ui.invoke_toggle_timer());
    app.advance(Duration::from_secs(4));
    assert!(!modified(&app));
}

// ── The window coming forward ────────────────────────────────────────

#[test]
fn the_window_coming_forward_asks_nothing_of_the_deck() {
    let app = three();
    let before = (outline(&app), summary(&app));
    app.activate(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.pending_commands(), 0);
    assert_eq!((outline(&app), summary(&app)), before, "yPresent follows no clock and no service");
    act(&app, "show", none()).unwrap();
}
