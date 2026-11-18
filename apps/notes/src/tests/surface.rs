//! The surface a mind reads, and what it is told afterwards, through the real dispatch.

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::control::Action;
use yantrik_app::intents::Intents;
use yantrik_testing::{Call, TestApp};

use super::{act, failing, note, settle, started, state, vault};
use crate::command::Command;
use crate::Notes;

fn published() -> Vec<Action> {
    let mut table = Intents::<Notes>::new();
    Notes::intents(&mut table);
    table.specs()
}

/// The check that would have caught `"Notes: append"`.
///
/// Every one of these descriptions used to be `format!("Notes: {name}")` and every argument used
/// to have none, which is why a model asked for a note titled Groceries produced a note with no
/// title: nothing it was shown said `new_note` took one.
#[test]
fn every_action_says_what_it_does() {
    let published = published();
    assert!(published.len() >= 18, "the surface lost actions: {} published", published.len());
    for spec in &published {
        assert_ne!(
            spec.description,
            format!("Notes: {}", spec.name),
            "`{}` still carries the placeholder the old loop generated",
            spec.name
        );
        assert!(
            spec.description.len() >= 20,
            "`{}` has nothing a reader who cannot see the screen could use: {:?}",
            spec.name,
            spec.description
        );
        assert!(spec.description.split_whitespace().count() >= 5, "`{}` is not a sentence: {:?}", spec.name, spec.description);
        for p in &spec.params {
            assert!(p.description.len() >= 15, "`{}` takes `{}` and says nothing about what goes in it", spec.name, p.name);
        }
        assert!(
            ["safe", "standard", "sensitive", "dangerous"].contains(&spec.permission),
            "`{}` is graded `{}`, which is not a grade this OS defines",
            spec.name,
            spec.permission
        );
    }
    // The four that have to be able to do damage, and say so.
    for name in ["trash", "set_content", "import", "export"] {
        let spec = published.iter().find(|s| s.name == name).unwrap();
        assert_eq!(
            spec.permission, "sensitive",
            "`{name}` removes a note, overwrites one, or reads and writes outside the notes \
             directory; it cannot be graded `{}`",
            spec.permission
        );
    }
    // A pure read is `safe`: a mind is handed a safe read's whole answer, and anything graded
    // higher is cut short in its work log, which hides the very text the read was for.
    let spec = published.iter().find(|s| s.name == "read_notes").unwrap();
    assert_eq!(spec.permission, "safe", "`read_notes` only reads; it must be graded `safe`");
}

/// "Create a note titled Groceries listing milk, eggs and bread" — in one call, whose answer
/// names the file it wrote so the claim is checkable.
#[test]
fn one_call_makes_a_titled_note_with_a_body() {
    let app = started();
    let dir = vault(&app);
    let answer = act(&app, "new_note", json!({ "title": "Groceries", "text": "milk\neggs\nbread" }))
        .expect("new_note with a title and a body");
    let filename = answer["filename"].as_str().expect("the answer names the file it wrote").to_string();
    assert_eq!(answer["title"], "Groceries", "answer: {answer}");
    assert_eq!(answer["saved"], true, "answer: {answer}");
    assert_eq!(answer["matches_disk"], true, "the answer claims the bytes on disk are the note: {answer}");
    assert_eq!(answer["path"], dir.join(&filename).display().to_string());
    assert_eq!(answer["lines"], 5, "answer: {answer}");
    let on_disk = std::fs::read_to_string(dir.join(&filename)).expect("the note is on disk");
    assert!(on_disk.starts_with("# Groceries\n"), "the title has to be the note's first heading line: {on_disk:?}");
    assert!(on_disk.contains("milk\neggs\nbread"), "the body has to be in the note: {on_disk:?}");

    // The summary is the first thing a mind reads and the only thing `os_apps` prints per app.
    let summary = app.describe()["summary"].as_str().unwrap().to_string();
    assert!(summary.contains("\"Groceries\" open"), "the summary has to name the open note: {summary:?}");
    assert!(summary.contains("saved") && summary.contains("note"), "and whether it is saved: {summary:?}");

    // `append` adds and does not replace, and says how much is there now.
    let after = act(&app, "append", json!({ "text": "\nbutter\n" })).expect("append to the open note");
    assert_eq!(after["filename"], json!(filename));
    assert_eq!(after["title"], "Groceries", "appending must not lose the title");
    assert_eq!(after["saved"], true);
    assert!(std::fs::read_to_string(dir.join(&filename)).unwrap().ends_with("butter\n"), "append has to reach the file");
}

/// `tags` moved the note's `.meta` and answered with the same revision as before (#79): the
/// revision is the hash of the view, and the view must carry the sidecar.
#[test]
fn metadata_moves_the_revision() {
    let app = started();
    act(&app, "new_note", json!({ "title": "Talk" })).unwrap();
    let before = app.revision();
    let answer = act(&app, "tags", json!({ "text": "talk, work" })).expect("tags on the open note");
    assert_eq!(answer["tags"], "talk, work", "answer: {answer}");
    assert_eq!(answer["saved"], true, "the action says it saved: {answer}");
    assert_ne!(before, app.revision(), "the note's metadata moved, so its revision has to move with it");
    let filename = answer["filename"].as_str().expect("the answer names the file it wrote");
    assert!(
        std::fs::read_to_string(vault(&app).join(filename).with_extension("meta"))
            .expect("the sidecar is on disk")
            .contains("tags:talk, work"),
        "the tags the revision now hashes have to be the tags on disk"
    );
    // The field in the window says it too, as if typed there.
    assert_eq!(app.ui(|ui| ui.get_tags().to_string()), "talk, work");
}

/// `append` with nothing to append used to answer `{"accepted": true, "completed": false}`.
#[test]
fn a_missing_required_argument_is_refused_by_name() {
    let app = started();
    act(&app, "new_note", json!({ "title": "Groceries" })).unwrap();
    let spec = published().into_iter().find(|s| s.name == "append").unwrap();
    let text = spec.params.iter().find(|p| p.name == "text").expect("append takes `text`");
    assert!(text.required, "`text` has to be declared required, or the dispatch accepts `append` with no text");

    let before = state(&app)["content"].clone();
    let refusal = act(&app, "append", json!({ "text": "   " })).expect_err("append with no text must be refused");
    assert!(refusal.contains("text"), "the refusal has to name the argument that was missing: {refusal}");
    // Contract point 4: said twice — to the caller, and on screen for the person.
    assert!(app.ui(|ui| ui.get_notice().to_string()).contains("text"), "the refusal has to reach the window's notice");
    assert_eq!(state(&app)["content"], before, "a refused append must not have changed the note");
    assert!(app.act("append", json!({})).is_err(), "and a missing one never reaches the app");

    // An action that names the note it cannot find, rather than a bare failure.
    let missing = act(&app, "open_note", json!({ "title": "Nothing here" }))
        .expect_err("opening a note that does not exist must be refused");
    assert!(
        missing.contains("Nothing here") && missing.contains("Groceries"),
        "the refusal has to say what it could not find and what there is: {missing}"
    );
}

#[test]
fn a_call_while_the_store_works_is_refused_and_one_after_it_is_not() {
    let app = TestApp::<Notes>::start();
    let e = app.act("read_notes", json!({})).unwrap_err();
    assert_eq!(e, "Notes is in the middle of a file operation; read `describe` again in a moment.");
    settle(&app);
    assert_eq!(act(&app, "read_notes", json!({})).unwrap()["matching"], 0);
    // An agent reads as the person does: no rule of Notes' own stands between.
    let reply = app.act_as(Call::new("read_notes", json!({})).as_agent("mind")).unwrap().settle();
    assert!(reply.is_ok(), "{reply:?}");
}

#[test]
fn opening_by_title_saves_the_open_note_first_or_does_nothing() {
    let app = started();
    act(&app, "new_note", json!({ "title": "First" })).unwrap();
    act(&app, "new_note", json!({ "title": "Second" })).unwrap();
    // An edit not yet saved goes with the open note's file before the other opens.
    app.run(crate::command::Command::Edit("# Second\n\nunsaved line".into()));
    let answer = act(&app, "open_note", json!({ "title": "First" })).unwrap();
    assert_eq!(answer["title"], "First");
    let second = app.model(|m| m.library.iter().find(|n| n.title() == "Second").unwrap().content.text.clone());
    assert_eq!(second, "# Second\n\nunsaved line");
    // A save that cannot be made refuses the open, and nothing else is done.
    super::failing(&app).fail(yantrik_app::documents::disk::Step::Write, ".md", None);
    app.run(crate::command::Command::Edit("# First\n\nstuck".into()));
    let e = act(&app, "open_note", json!({ "title": "Second" })).unwrap_err();
    assert!(e.starts_with("The open note could not be saved, so nothing else was done: "), "{e}");
    assert_eq!(state(&app)["title"], "First");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), e, "said in the window too");
}

/// What the library holds: its entries, and the `.md` files in the vault.
fn held(app: &TestApp<Notes>) -> (usize, usize) {
    let files = std::fs::read_dir(vault(app))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
        .count();
    (app.model(|m| m.library.len()), files)
}

/// A note that cannot be made refuses the call with the reason, and the window's notice keeps it,
/// with another note open, whose answer must not stand in for the note that was not made.
#[test]
fn a_new_note_with_a_body_no_note_may_hold_is_refused_and_nothing_is_made() {
    let app = started();
    act(&app, "new_note", json!({ "title": "Open already" })).unwrap();
    let before = held(&app);
    let e = act(&app, "new_note", json!({ "title": "Bell", "text": "ring \u{7} ring" })).unwrap_err();
    assert_eq!(e, "Notes require UTF-8 text without control characters.");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), e, "the notice keeps the real reason");
    assert_eq!(held(&app), before, "nothing was created");
    assert_eq!(state(&app)["title"], "Open already", "the open note stays open");
}

#[test]
fn a_new_note_past_the_library_limit_is_refused_and_nothing_is_made() {
    let paths = yantrik_app::paths::Paths::temporary(Notes::ID);
    std::fs::create_dir_all(paths.data_dir()).unwrap();
    for i in 0..crate::store::MAX_NOTES {
        std::fs::write(paths.data_dir().join(format!("n{i:04}.md")), format!("# Note {i}\n")).unwrap();
    }
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    act(&app, "open_note", json!({ "title": "Note 7" })).unwrap();
    let before = held(&app);
    assert_eq!(before, (crate::store::MAX_NOTES, crate::store::MAX_NOTES));
    let e = act(&app, "new_note", json!({ "title": "One more" })).unwrap_err();
    assert_eq!(e, "The library supports up to 2,000 notes and 32 MiB of text. Nothing was created.");
    assert_eq!(app.ui(|ui| ui.get_notice().to_string()), e);
    assert_eq!(held(&app), before, "nothing was created");
    assert_eq!(state(&app)["title"], "Note 7");
}

#[test]
fn a_reload_call_whose_save_first_fails_is_refused_and_nothing_is_reloaded() {
    let app = started();
    note(&app, "Draft", "");
    failing(&app).fail(yantrik_app::documents::disk::Step::Write, ".md", None);
    app.run(Command::Edit("# Draft\n\nunsaved".into()));
    // A note another program added: a reload that ran would list it.
    std::fs::write(vault(&app).join("outside.md"), "# Outside\n").unwrap();
    let refused = act(&app, "reload", json!({})).unwrap_err();
    assert!(
        refused.starts_with("The open note could not be saved, so nothing else was done: ")
            && refused.contains("injected failure at Write"),
        "{refused}"
    );
    assert_eq!(state(&app)["note_count"], 1, "nothing was reloaded");
    assert_eq!(state(&app)["unsaved"], true);
}

/// The meeting brief opens a note by its file name. Two notes may share a title, and by title the
/// first listed wins, so a row for the second would open the first.
#[test]
fn a_note_is_opened_by_its_id_even_when_another_has_the_same_title() {
    let app = started();
    act(&app, "new_note", json!({ "title": "Twin" })).unwrap();
    act(&app, "new_note", json!({ "title": "Twin" })).unwrap();
    let ids: Vec<String> = app.model(|m| m.library.iter().filter(|n| n.title() == "Twin").map(|n| n.id.clone()).collect());
    assert_eq!(ids.len(), 2, "two notes with one title");
    for id in ids.iter().rev() {
        act(&app, "open_note", json!({ "title": id })).unwrap();
        assert_eq!(state(&app)["open_note"], *id, "the one asked for, not the first listed");
    }
}
