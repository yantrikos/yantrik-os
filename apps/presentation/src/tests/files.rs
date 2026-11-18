//! The deck and its file, through the framework's document layer on real files read back off the
//! disk. These carry what `tests/presentation-core` pinned of the file (it pulled `deck.rs` in
//! with `#[path]`, and is gone with it); the pure half moved to `deck/tests.rs`.

use serde_json::json;

use super::{
    act, deck_folder, deck_text, file, listing, modified, none, notice, outline, path_of, press, read_deck,
    scratch, started, state, with_file, write_deck,
};
use crate::command::WindowAction;
use crate::deck::{parse, MAX_FILE_BYTES};

fn ask(path: &std::path::Path) -> serde_json::Value {
    json!({ "path": path.display().to_string() })
}

#[test]
fn a_saved_deck_comes_back_with_every_field() {
    let (app, _) = with_file("round", &["x"]);
    let path = scratch("round-trip").join("round.ydeck");
    act(&app, "set_slide", json!({ "index": 0, "title": "First", "body": "body one", "notes": "note one" })).unwrap();
    act(&app, "add_slide", json!({ "title": "Second", "body": "body two", "notes": "note two" })).unwrap();
    press(&app, WindowAction::SetTheme(3));
    let saved = act(&app, "save_as", ask(&path)).unwrap();
    assert_eq!(saved["slides"], 2);
    assert!(!modified(&app), "a deck that was just written is not modified");
    assert_eq!(path_of(&app), Some(path.clone()));

    let back = read_deck(&path);
    assert_eq!((back.title.as_str(), back.theme, back.slides.len()), ("Talk", 3, 2));
    assert_eq!(back.slides[0].notes, "note one");
    assert_eq!(back.slides[1].body, "body two");

    let again = started();
    act(&again, "open", ask(&path)).unwrap();
    assert_eq!(outline(&again), vec!["First", "Second"]);
    assert_eq!(state(&again)["theme"], "Sand");
    assert!(!modified(&again));
}

#[test]
fn the_file_says_which_version_it_is_and_says_it_first() {
    let app = started();
    let path = scratch("version").join("v.ydeck");
    act(&app, "save_as", ask(&path)).unwrap();
    assert!(file(&path).trim_start().starts_with("{\n  \"version\": 1"), "{}", file(&path));
}

#[test]
fn a_file_from_a_newer_build_is_refused_by_name_and_the_deck_on_screen_is_untouched() {
    let (app, _) = with_file("open-here", &["Mine"]);
    let dir = scratch("ahead");
    let ahead = dir.join("ahead.ydeck");
    std::fs::write(&ahead, deck_text("T", &["x"]).replace("\"version\": 1", "\"version\": 99")).unwrap();
    let err = act(&app, "open", ask(&ahead)).unwrap_err();
    assert!(err.starts_with(&ahead.display().to_string()), "the path first: {err}");
    assert!(err.contains("version 99") && err.contains("reads version 1"), "{err}");
    assert_eq!(notice(&app), err, "said to the window as well");
    assert_eq!(outline(&app), vec!["Mine"], "nothing is half-loaded");
}

#[test]
fn what_is_not_a_deck_is_told_apart_and_costs_nothing() {
    let (app, _) = with_file("kept", &["Mine"]);
    let dir = scratch("not-decks");
    let cases = [
        ("plain.ydeck", "this is not json".to_string(), "it is not a yPresent deck"),
        ("unversioned.ydeck", "{\"title\":\"x\",\"slides\":[]}".to_string(), "it has no `version`"),
        ("shape.ydeck", "{\"version\":1,\"title\":\"x\",\"slides\":\"nope\"}".to_string(), "not that shape"),
        ("empty.ydeck", "{\"version\":1,\"title\":\"x\",\"slides\":[]}".to_string(), "it holds no slides"),
    ];
    for (name, text, why) in cases {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let err = act(&app, "open", ask(&path)).unwrap_err();
        assert!(err.contains(why), "{name}: {err}");
        assert_eq!(outline(&app), vec!["Mine"], "{name}");
    }
}

#[test]
fn a_file_that_is_not_there_is_unreadable_and_says_so() {
    let app = started();
    let missing = scratch("missing").join("nothing-here.ydeck");
    let err = act(&app, "open", ask(&missing)).unwrap_err();
    assert!(err.starts_with(&missing.display().to_string()) && err.contains("could not read it"), "{err}");
}

#[test]
fn a_deck_past_the_size_bound_or_not_text_or_not_a_file_is_refused_unread() {
    let app = started();
    let dir = scratch("bounds");
    let big = dir.join("big.ydeck");
    std::fs::write(&big, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
    let err = act(&app, "open", ask(&big)).unwrap_err();
    assert!(err.contains(&format!("it is {} bytes, over the {MAX_FILE_BYTES}-byte limit", MAX_FILE_BYTES + 1)), "{err}");
    let binary = dir.join("binary.ydeck");
    std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
    assert!(act(&app, "open", ask(&binary)).unwrap_err().contains("valid UTF-8"));
    assert!(act(&app, "open", ask(&dir)).unwrap_err().contains("not a regular file"));
    assert_eq!(outline(&app), vec!["Title Slide"]);
}

#[test]
fn a_layout_this_build_cannot_draw_and_a_theme_it_does_not_have_open_as_something_drawable() {
    let app = started();
    let path = scratch("clamps").join("c.ydeck");
    std::fs::write(
        &path,
        "{\"version\":1,\"title\":\"t\",\"theme\":41,\"slides\":[{\"title\":\"a\",\"body\":\"\",\"notes\":\"\",\"layout\":99}]}",
    )
    .unwrap();
    act(&app, "open", ask(&path)).unwrap();
    assert_eq!(app.model(|m| m.deck().current_slide().layout), crate::deck::LAYOUT_COUNT - 1);
    assert!(app.model(|m| m.deck().theme()) < crate::deck::THEMES.len());
    assert!(!modified(&app), "clamping on the way in is not an edit");
}

#[test]
fn a_deck_is_opened_by_the_name_every_link_in_its_path_resolves_to() {
    let app = started();
    let dir = scratch("links");
    let real = dir.join("real.ydeck");
    write_deck(&real, "T", &["linked"]);
    let alias = dir.join("alias.ydeck");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    act(&app, "open", ask(&alias)).unwrap();
    assert_eq!(path_of(&app), Some(real), "a save goes to the file, never through the link");
}

#[test]
fn saving_leaves_no_half_written_file_beside_the_deck() {
    let (app, path) = with_file("clean", &["One"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    act(&app, "save", none()).unwrap();
    assert_eq!(listing(path.parent().unwrap()), vec!["talk.ydeck"]);
    assert_eq!(read_deck(&path).slides[0].title, "Two");
}

#[test]
fn a_deck_that_has_a_home_keeps_it() {
    let (app, path) = with_file("home", &["One"]);
    act(&app, "add_slide", json!({ "title": "Two" })).unwrap();
    let answer = act(&app, "save", none()).unwrap();
    assert_eq!(answer["path"], json!(path.display().to_string()));
    assert_eq!(answer["bytes"], json!(file(&path).len()));
    assert_eq!(answer["slides"], 2);
    assert_eq!(answer["dirty"], false);
    assert_eq!(state(&app)["saves_to"], json!(path.display().to_string()));
}

#[test]
fn a_deck_with_no_name_is_given_one_in_the_deck_folder_and_two_never_share_it() {
    let app = started();
    let dir = deck_folder(&app);
    let first = act(&app, "save", none()).unwrap();
    assert_eq!(first["path"], json!(dir.join("untitled.ydeck").display().to_string()));
    assert_eq!(state(&app)["file"], first["path"], "describe names the file written");
    act(&app, "new_deck", none()).unwrap();
    let second = act(&app, "save", none()).unwrap();
    assert_eq!(second["path"], json!(dir.join("untitled-2.ydeck").display().to_string()));
    assert_eq!(listing(&dir), vec!["untitled-2.ydeck", "untitled.ydeck"]);
}

#[test]
fn the_deck_folder_is_made_when_it_is_not_there() {
    let app = started();
    let folder = scratch("fresh").join("Documents/Presentations");
    app.run(crate::command::Command::UseDeckDir(folder.clone()));
    act(&app, "save", none()).unwrap();
    assert!(folder.join("untitled.ydeck").exists());
}

#[test]
fn save_as_writes_to_the_path_and_the_next_save_follows_it() {
    let (app, first) = with_file("moves", &["One"]);
    let second = scratch("moves-2").join("copy.ydeck");
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    act(&app, "save_as", ask(&second)).unwrap();
    assert_eq!(path_of(&app), Some(second.clone()));
    assert_eq!(read_deck(&first).slides[0].title, "One", "the first file is untouched");
    act(&app, "set_slide", json!({ "index": 0, "title": "Three" })).unwrap();
    act(&app, "save", none()).unwrap();
    assert_eq!(read_deck(&second).slides[0].title, "Three");
    assert_eq!(read_deck(&first).slides[0].title, "One");
}

#[test]
fn save_as_refuses_a_file_that_is_somebody_elses_and_a_relative_path() {
    let app = started();
    let dir = scratch("clobber");
    let theirs = dir.join("theirs.ydeck");
    std::fs::write(&theirs, "THEIRS").unwrap();
    let err = act(&app, "save_as", ask(&theirs)).unwrap_err();
    assert!(err.contains("already exists") && err.contains("nothing was overwritten"), "{err}");
    assert_eq!(file(&theirs), "THEIRS");
    assert_eq!(path_of(&app), None);
    // With `overwrite` the same call replaces it, and the bytes on disk are the deck.
    let made = act(&app, "save_as", json!({ "path": theirs.display().to_string(), "overwrite": true })).unwrap();
    assert_eq!(made["slides"], 1);
    assert_eq!(read_deck(&theirs).slides[0].title, "Title Slide");
    assert_eq!(path_of(&app), Some(theirs.clone()));
    assert_eq!(listing(theirs.parent().unwrap()), vec!["theirs.ydeck"], "and no temporary is left");
    let again = started();
    std::fs::write(&theirs, "THEIRS AGAIN").unwrap();
    let off = act(&again, "save_as", json!({ "path": theirs.display().to_string(), "overwrite": false })).unwrap_err();
    assert!(off.contains("overwrite=true"), "{off}");
    assert_eq!(file(&theirs), "THEIRS AGAIN");
    let app = started();
    let relative = act(&app, "save_as", json!({ "path": "relative/x.ydeck" })).unwrap_err();
    assert!(relative.contains("absolute file path"), "{relative}");
    let empty = act(&app, "save_as", json!({ "path": "  " }));
    assert!(empty.is_err());
}

#[test]
fn a_deck_that_cannot_be_written_is_still_the_deck_that_is_open() {
    let app = started();
    let blocked = scratch("blocked").join("blocked.ydeck");
    std::fs::create_dir_all(&blocked).unwrap();
    let err = act(&app, "save_as", ask(&blocked)).unwrap_err();
    assert!(err.contains("blocked.ydeck"), "{err}");
    assert_eq!(path_of(&app), None, "the deck does not start claiming it lives there");
    assert_eq!(outline(&app), vec!["Title Slide"]);
    let missing_folder = scratch("blocked-2").join("no/such/folder/x.ydeck");
    assert!(act(&app, "save_as", ask(&missing_folder)).unwrap_err().contains("Cannot write into that folder"));
}

#[test]
fn an_export_goes_beside_the_deck_with_the_same_stem_and_replaces_its_own_earlier_one() {
    let (app, path) = with_file("export", &["First", "Second"]);
    let first = act(&app, "export_markdown", none()).unwrap();
    let md = path.with_file_name("talk.md");
    assert_eq!(first["path"], json!(md.display().to_string()));
    assert_eq!(first["slides"], 2);
    assert!(file(&md).starts_with("<!-- yPresent deck: Talk -->"), "{}", file(&md));
    act(&app, "set_slide", json!({ "index": 0, "title": "Renamed" })).unwrap();
    act(&app, "export_markdown", none()).unwrap();
    assert!(file(&md).contains("# Renamed"), "the earlier export is replaced, as it always was");
    assert_eq!(listing(path.parent().unwrap()), vec!["talk.md", "talk.ydeck"], "and nothing is left beside it");
    assert_eq!(read_deck(&path).slides[0].title, "First", "an export is a copy: the deck is where it was");
}

#[test]
fn an_export_to_a_path_the_caller_named_never_replaces_a_file() {
    let (app, path) = with_file("export-named", &["One"]);
    let target = path.with_file_name("named.md");
    let made = act(&app, "export_markdown", ask(&target)).unwrap();
    assert_eq!(made["path"], json!(target.display().to_string()));
    let again = act(&app, "export_markdown", ask(&target)).unwrap_err();
    assert!(again.contains("already exists"), "{again}");
    std::fs::write(&target, "MINE").unwrap();
    assert!(act(&app, "export_markdown", ask(&target)).is_err());
    assert_eq!(file(&target), "MINE");
}

#[test]
fn the_windows_export_buttons_write_the_markdown_and_the_outline() {
    let (app, path) = with_file("export-buttons", &["One", "Two"]);
    press(&app, WindowAction::ExportMarkdown);
    assert_eq!(notice(&app), format!("Exported to {}.", path.with_file_name("talk.md").display()));
    press(&app, WindowAction::ExportOutline);
    let outline_file = path.with_file_name("talk.outline.md");
    assert_eq!(file(&outline_file), "# Talk\n\n1. One\n2. Two\n");
    assert_eq!(listing(path.parent().unwrap()), vec!["talk.md", "talk.outline.md", "talk.ydeck"]);
}

#[test]
fn the_open_button_opens_the_deck_most_recently_worked_on() {
    let app = started();
    let dir = deck_folder(&app);
    assert!(press_open(&app).contains("there are no decks in"), "{}", notice(&app));
    write_deck(&dir.join("old.ydeck"), "Old", &["old"]);
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_deck(&dir.join("new.ydeck"), "New", &["new"]);
    std::fs::write(dir.join("notes.txt"), "not a deck").unwrap();
    press_open(&app);
    assert_eq!(path_of(&app), Some(dir.join("new.ydeck")));
    assert_eq!(outline(&app), vec!["new"]);
}

fn press_open(app: &yantrik_testing::TestApp<crate::YPresent>) -> String {
    press(app, WindowAction::OpenRecent);
    notice(app)
}

#[test]
fn a_decks_content_survives_the_text_a_slide_may_hold() {
    let app = started();
    let odd = "quotes \" and \\ and \n newlines, tabs\t, unicode नमस्ते 🦀, and {\"version\": 1}";
    act(&app, "set_slide", json!({ "index": 0, "title": odd, "body": odd, "notes": odd })).unwrap();
    let path = scratch("odd").join("odd.ydeck");
    act(&app, "save_as", ask(&path)).unwrap();
    let back = parse(&file(&path)).unwrap();
    assert_eq!((back.slides[0].title.as_str(), back.slides[0].notes.as_str()), (odd, odd));
}

#[test]
fn an_export_never_writes_over_the_deck_it_came_from() {
    let app = started();
    let dir = scratch("export-self");
    let deck = dir.join("talk.md");
    act(&app, "set_slide", json!({ "index": 0, "title": "My deck, saved with an md name" })).unwrap();
    act(&app, "save_as", ask(&deck)).unwrap();
    let before = file(&deck);
    let err = act(&app, "export_markdown", none()).unwrap_err();
    assert!(err.contains("never replaces the deck it comes from"), "{err}");
    assert_eq!(file(&deck), before, "the deck's file is as it was");
    assert!(!modified(&app));
    // The window counts up past the deck's own name, and says where it went.
    press(&app, WindowAction::ExportMarkdown);
    let counted = dir.join("talk-2.md");
    assert_eq!(notice(&app), format!("Exported to {}.", counted.display()));
    assert!(file(&counted).starts_with("<!-- yPresent deck:"));
    assert_eq!(file(&deck), before, "the deck is as it was");
}

#[test]
fn a_default_export_replaces_only_an_earlier_export_of_this_deck() {
    let (app, path) = with_file("export-guard", &["One"]);
    let md = path.with_file_name("talk.md");
    // The person's own file of that name, beside the deck.
    std::fs::write(&md, "MY OWN NOTES\n").unwrap();
    let err = act(&app, "export_markdown", none()).unwrap_err();
    assert!(err.contains(&md.display().to_string()) && err.contains("not an earlier export of this deck"), "{err}");
    assert!(!err.contains("   "), "no run of spaces in a sentence: {err}");
    assert_eq!(file(&md), "MY OWN NOTES\n", "byte-identical afterwards");
    // The window does not send the person to a tool they cannot reach: it counts up.
    press(&app, WindowAction::ExportMarkdown);
    let counted = path.with_file_name("talk-2.md");
    assert_eq!(notice(&app), format!("Exported to {}.", counted.display()));
    assert_eq!(file(&md), "MY OWN NOTES\n");
    // The same for the outline, whose earlier export starts with the deck's title and a number.
    let outline_file = path.with_file_name("talk.outline.md");
    std::fs::write(&outline_file, "# Talk\n\nnot a numbered outline\n").unwrap();
    press(&app, WindowAction::ExportOutline);
    assert_eq!(file(&outline_file), "# Talk\n\nnot a numbered outline\n");
    assert_eq!(file(&path.with_file_name("talk-2.outline.md")), "# Talk\n\n1. One\n");
    // Out of the way, an export is made; the next one replaces it.
    std::fs::remove_file(&md).unwrap();
    act(&app, "export_markdown", none()).unwrap();
    act(&app, "set_slide", json!({ "index": 0, "title": "Renamed" })).unwrap();
    act(&app, "export_markdown", none()).unwrap();
    assert!(file(&md).contains("# Renamed"), "a previous export is replaced");
}

#[test]
fn an_export_from_another_deck_with_the_same_stem_is_not_replaced() {
    let dir = scratch("export-other-deck");
    let first = dir.join("talk.ydeck");
    write_deck(&first, "Quarterly", &["a"]);
    let app = started();
    act(&app, "open", ask(&first)).unwrap();
    act(&app, "export_markdown", none()).unwrap();
    let md = dir.join("talk.md");
    let theirs = file(&md);
    assert!(theirs.starts_with("<!-- yPresent deck: Quarterly -->"));
    // Another deck, also called talk.ydeck elsewhere, exported into the same place.
    let second = dir.join("talk2/talk.ydeck");
    std::fs::create_dir_all(second.parent().unwrap()).unwrap();
    write_deck(&second, "Annual", &["b"]);
    std::fs::rename(&md, second.with_file_name("talk.md")).unwrap();
    act(&app, "open", ask(&second)).unwrap();
    let err = act(&app, "export_markdown", none()).unwrap_err();
    assert!(err.contains("not an earlier export of this deck"), "{err}");
    assert_eq!(file(&second.with_file_name("talk.md")), theirs, "the other deck's export is as it was");
}

#[test]
fn two_never_saved_decks_with_the_same_title_do_not_share_one_default_export() {
    let app = started();
    let dir = deck_folder(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "A's slide" })).unwrap();
    let first = act(&app, "export_markdown", none()).unwrap();
    assert_eq!(first["path"], json!(dir.join("untitled.md").display().to_string()));
    let a = file(&dir.join("untitled.md"));
    // A is kept as untitled.ydeck, and B is another deck that is also called Untitled.
    act(&app, "new_deck", none()).unwrap();
    act(&app, "set_slide", json!({ "index": 0, "title": "B's slide" })).unwrap();
    let refused = act(&app, "export_markdown", none()).unwrap_err();
    assert!(refused.contains("not an earlier export of this deck"), "{refused}");
    press(&app, WindowAction::ExportMarkdown);
    assert_eq!(notice(&app), format!("Exported to {}.", dir.join("untitled-2.md").display()));
    assert_eq!(file(&dir.join("untitled.md")), a, "A's export is as it was");
    assert!(file(&dir.join("untitled-2.md")).contains("B's slide"));
    // B's own earlier export is replaced by its next one.
    act(&app, "set_slide", json!({ "index": 0, "title": "B again" })).unwrap();
    press(&app, WindowAction::ExportMarkdown);
    assert!(file(&dir.join("untitled-2.md")).contains("B again"));
    assert_eq!(super::listing(&dir), vec!["untitled-2.md", "untitled.md", "untitled.ydeck"]);
}

#[test]
fn an_export_changed_since_it_was_made_is_not_replaced() {
    let (app, path) = with_file("export-edited", &["One"]);
    let md = path.with_file_name("talk.md");
    act(&app, "export_markdown", none()).unwrap();
    // The person adds to it: it still begins with the header, but it is no longer what was written.
    let mut mine = file(&md);
    mine.push_str("\nMY ADDITIONS\n");
    std::fs::write(&md, &mine).unwrap();
    assert!(act(&app, "export_markdown", none()).unwrap_err().contains("not an earlier export of this deck"));
    assert_eq!(file(&md), mine);
    press(&app, WindowAction::ExportMarkdown);
    assert_eq!(file(&md), mine);
    assert!(path.with_file_name("talk-2.md").exists());
}
