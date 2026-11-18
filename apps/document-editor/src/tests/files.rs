//! The document and its files, through real files in scratch folders and read back off the disk.
//!
//! These are the assertions yDoc's old `tests/document-core` held for `document.rs`, which the
//! port split across `files/`, `markdown/`, `update/` and the framework's `documents/`. The first
//! of them is the one about the bug that started it: Save was structurally unreachable (#84, #85),
//! so the no-file case must produce a SENTENCE, and every save here lands in a real directory.

use std::path::{Path, PathBuf};

use serde_json::json;
use yantrik_app::documents::disk::RealDisk;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{
    act, draft_file, file, killed_and_started_again, modified, none, notice, outside_writes, path_of, scratch, settle,
    started, state, text, typed, with_file,
};
use crate::files::save::{save, SaveJob};
use crate::limits::MAX_BYTES;
use crate::YDoc;

/// The names in a folder, sorted.
fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("readable")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn sentence(app: &TestApp<YDoc>) -> String {
    notice(app)
}

// ── The bug ─────────────────────────────────────────────────────────────────

#[test]
fn save_with_no_file_names_both_ways_out_and_a_place_that_would_do() {
    let app = started();
    act(&app, "set_content", json!({ "text": "# Notes\n\nSomething worth keeping.\n" })).unwrap();
    let message = act(&app, "save", none()).unwrap_err();
    assert!(message.contains("no file yet"), "{message}");
    assert!(message.contains("Save As"), "{message}");
    assert!(message.contains("`save_as`"), "the call a mind can make: {message}");
    assert!(message.contains(".md"), "and where it would go: {message}");
}

#[test]
fn the_suggested_home_is_a_markdown_file_named_after_the_document() {
    let app = started();
    act(&app, "set_content", json!({ "text": "# Quarterly Plan, 2026\n\nbody\n" })).unwrap();
    let home = app.model(|m| m.home());
    assert_eq!(home.extension().and_then(|e| e.to_str()), Some("md"));
    assert_eq!(home.file_name().and_then(|n| n.to_str()), Some("quarterly-plan-2026.md"));
}

#[test]
fn a_document_with_nothing_in_it_still_has_a_name_to_propose() {
    let app = started();
    let home = app.model(|m| m.home());
    assert_eq!(home.file_name().and_then(|n| n.to_str()), Some("untitled.md"));
}

#[test]
fn the_title_is_the_first_h1_and_falls_back_to_the_file_name() {
    let app = started();
    act(&app, "set_content", json!({ "text": "## Second\n# First\n" })).unwrap();
    assert_eq!(app.model(|m| m.title()), "First");
    assert_eq!(state(&app)["title"], "First");
    act(&app, "set_content", json!({ "text": "no headings here\n" })).unwrap();
    assert_eq!(app.model(|m| m.title()), "Untitled");
    // A document from a file with no heading is named by the file.
    let (app, _) = with_file("title", "no headings here\n");
    assert_eq!(app.model(|m| m.title()), "notes.md");
}

// ── Round trips ─────────────────────────────────────────────────────────────

#[test]
fn save_then_load_returns_exactly_what_was_written() {
    let app = started();
    let path = scratch("round-trip").join("round-trip.md");
    // Unicode, a tab, CRLF and a trailing newline: four things a careless writer loses.
    let body = "# Título\n\nनमस्ते — \"quoted\"\ttabbed\r\nlast line\n";
    act(&app, "set_content", json!({ "text": body })).unwrap();
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert!(!modified(&app), "a document that has just been written is not dirty");
    assert_eq!(std::fs::read(&path).unwrap(), body.as_bytes(), "byte for byte");

    act(&app, "new", none()).unwrap();
    act(&app, "open", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(text(&app), body, "including the trailing newline");
    assert!(!modified(&app));
}

#[test]
fn a_document_without_a_trailing_newline_does_not_grow_one() {
    let app = started();
    let path = scratch("no-newline").join("no-newline.md");
    act(&app, "set_content", json!({ "text": "one line, no newline" })).unwrap();
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one line, no newline");
}

#[test]
fn save_as_moves_the_document_and_the_next_save_follows_it() {
    let app = started();
    let dir = scratch("moves");
    let (first, second) = (dir.join("first.md"), dir.join("second.md"));
    act(&app, "set_content", json!({ "text": "one\n" })).unwrap();
    act(&app, "save_as", json!({ "path": first.display().to_string() })).unwrap();
    act(&app, "save_as", json!({ "path": second.display().to_string() })).unwrap();
    assert_eq!(path_of(&app), Some(second.clone()));

    typed(&app, "two\n");
    assert!(modified(&app));
    let answer = act(&app, "save", none()).unwrap();
    assert_eq!(answer["saved"], json!(second));
    assert_eq!(file(&second), "two\n");
    // The original is where it was. Save As is a move of the document, not of the file.
    assert_eq!(file(&first), "one\n");
}

#[test]
fn a_relative_path_is_refused_by_name() {
    let app = started();
    act(&app, "set_content", json!({ "text": "x" })).unwrap();
    let error = act(&app, "save_as", json!({ "path": "notes.md" })).unwrap_err();
    assert!(error.contains("absolute"), "{error}");
    assert!(error.contains("notes.md"), "{error}");
}

// ── The conflict ────────────────────────────────────────────────────────────

#[test]
fn a_refused_save_over_a_changed_file_keeps_the_draft_and_says_so() {
    let (app, path) = with_file("conflict-draft", "as opened\n");
    typed(&app, "my edit\n");
    std::fs::write(&path, "their edit\n").unwrap();
    let error = act(&app, "save", none()).unwrap_err();
    assert!(error.contains("changed on disk"), "{error}");
    assert!(error.contains("draft is intact"), "{error}");
    assert!(error.contains("Save As"), "{error}");
    // Nothing was written while it refused (contract point 4), and the draft is still the draft.
    assert_eq!(file(&path), "their edit\n");
    assert_eq!(text(&app), "my edit\n");
}

#[test]
fn a_touch_that_changed_no_bytes_does_not_block_a_save() {
    let (app, path) = with_file("touched", "same\n");
    // Rewritten with identical content: the stamp moves, the bytes do not.
    std::thread::sleep(std::time::Duration::from_millis(12));
    outside_writes(&app, &path, "same\n");
    typed(&app, "changed\n");
    act(&app, "save", none()).expect("the bytes still agree, so this save is safe");
    assert_eq!(file(&path), "changed\n");
}

#[test]
fn a_save_reports_the_size_and_time_of_the_file_it_actually_wrote() {
    let app = started();
    let path = scratch("observed").join("observed.md");
    let body = "# Observed\n\nnot a number the caller supplied.\n";
    act(&app, "set_content", json!({ "text": body })).unwrap();
    let answer = act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    let on_disk = std::fs::metadata(&path).expect("it is there");
    assert_eq!(answer["bytes"], json!(on_disk.len()), "{answer}");
    assert_eq!(answer["bytes"], json!(body.len()), "{answer}");
    let modified_unix = on_disk.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert_eq!(answer["modified_unix"], json!(modified_unix), "a real mtime, read back off the file: {answer}");
}

// ── Moved out from under it ─────────────────────────────────────────────────
//
// Files moves a document with `rename`, so a document open in yDoc can have its file carried off
// while somebody is typing into it. On 22 Sep 2026 that left the app with three true refusals and
// no way through. The watch has not looked in these: the save and the Save As go looking.

/// The document's file carried into a folder beside it, as Files does it; the new path.
fn carried(path: &Path, into: &str) -> PathBuf {
    let folder = path.parent().unwrap().join(into);
    std::fs::create_dir_all(&folder).unwrap();
    let to = folder.join(path.file_name().unwrap());
    std::fs::rename(path, &to).unwrap();
    to
}

#[test]
fn a_save_onto_a_file_that_was_moved_says_where_it_went() {
    let (app, from) = with_file("moved-says", "# Pricing\n\nrates\n");
    typed(&app, "# Pricing\n\nrates, revised\n");
    let to = carried(&from, "pricing");
    let error = act(&app, "save", none()).unwrap_err();
    assert!(error.contains(&to.display().to_string()), "it names the way through: {error}");
    assert!(error.contains("Save As"), "{error}");
    assert!(error.contains("draft is intact"), "{error}");
    assert!(!from.exists() && file(&to) == "# Pricing\n\nrates\n", "nothing was written while it refused");
    assert_eq!(text(&app), "# Pricing\n\nrates, revised\n");
}

#[test]
fn a_save_onto_a_file_that_was_really_deleted_still_offers_a_way_to_write_it() {
    let (app, path) = with_file("deleted-offer", "one\n");
    std::fs::remove_file(&path).unwrap();
    typed(&app, "two\n");
    let error = act(&app, "save", none()).unwrap_err();
    assert!(error.contains("moved or deleted"), "{error}");
    assert!(error.contains("overwrite=true"), "{error}");
    assert!(!path.exists(), "and it really did not write while it refused");

    // The refusal is answerable, and what it offers is what it does.
    act(&app, "save_as", json!({ "path": path.display().to_string(), "overwrite": true })).unwrap();
    assert_eq!(file(&path), "two\n");
    assert!(!modified(&app));
}

#[test]
fn save_as_onto_the_moved_original_writes_into_it_rather_than_calling_it_somebody_elses() {
    let (app, from) = with_file("moved-into", "first\n");
    typed(&app, "second\n");
    let to = carried(&from, "archive");
    // The file at `to` has this document's own bytes under this document's own name, and the path
    // it was opened from no longer exists. That is not a stranger's file to refuse.
    let answer = act(&app, "save_as", json!({ "path": to.display().to_string() })).expect("its own file, moved");
    assert_eq!(answer["saved"], json!(to));
    assert_eq!(path_of(&app), Some(to.clone()));
    assert_eq!(file(&to), "second\n");
    assert!(!modified(&app));
}

#[test]
fn save_as_onto_somebody_elses_file_says_how_to_mean_it_and_leaves_no_temporary() {
    let (app, path) = with_file("occupied", "mine\n");
    let dir = path.parent().unwrap().to_path_buf();
    let occupied = dir.join("occupied.md");
    std::fs::write(&occupied, "someone else's work\n").unwrap();
    let args = json!({ "path": occupied.display().to_string() });
    let error = act(&app, "save_as", args).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert!(error.contains("nothing was overwritten"), "{error}");
    assert!(error.contains("overwrite=true"), "{error}");
    assert_eq!(file(&occupied), "someone else's work\n");

    // And the refusal is answerable rather than final.
    act(&app, "save_as", json!({ "path": occupied.display().to_string(), "overwrite": true })).unwrap();
    assert_eq!(file(&occupied), "mine\n");
    assert_eq!(path_of(&app), Some(occupied));
    assert_eq!(listing(&dir), vec!["notes.md".to_string(), "occupied.md".to_string()], "and no temporary left over");
}

// ── What an open would cost ─────────────────────────────────────────────────

#[test]
fn a_document_with_nothing_unsaved_has_nothing_to_warn_about() {
    let (app, _) = with_file("warn-none", "kept\n");
    assert_eq!(app.model(|m| m.unsaved(false)), None);
}

#[test]
fn an_unsaved_document_says_how_much_is_at_stake_and_where_the_rest_of_it_is() {
    let (app, path) = with_file("warn-edit", "# Pricing\n\nrates\n");
    typed(&app, "# Pricing\n\nrates and terms\n");
    let warning = app.model(|m| m.unsaved(false)).expect("there is something to lose");
    // The size of the edit, not the size of the document: " and terms" is ten characters.
    assert!(warning.contains("10 characters"), "{warning}");
    assert!(warning.contains("Pricing"), "{warning}");
    assert!(warning.contains(&path.display().to_string()), "{warning}");
}

#[test]
fn a_draft_that_is_in_no_file_at_all_says_that_rather_than_naming_one() {
    let app = started();
    act(&app, "set_content", json!({ "text": "# Thought\n\nnot written down anywhere.\n" })).unwrap();
    let warning = app.model(|m| m.unsaved(false)).expect("there is something to lose");
    assert!(warning.contains("no file at all"), "{warning}");
    assert!(warning.contains("Thought"), "{warning}");
}

#[test]
fn a_recovered_draft_is_at_stake_whole_because_nobody_has_agreed_to_any_of_it() {
    // A file deleted outside leaves its text on no disk; its draft comes back as the whole of it.
    let (app, path) = with_file("warn-recovered", "# Half a thought\n\nfrom last time\n");
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    settle(&app);
    app.run_writes();
    let app = killed_and_started_again(app);
    let warning = app.model(|m| m.unsaved(false)).expect("a recovered draft is always at stake");
    assert!(warning.contains("recovered draft"), "{warning}");
    let chars = text(&app).chars().count();
    assert!(warning.contains(&chars.to_string()), "{warning}");
}

// ── What it will not hold ───────────────────────────────────────────────────

#[test]
fn a_file_that_is_not_utf8_is_refused_without_a_lossy_conversion() {
    let app = started();
    let path = scratch("latin1").join("latin1.md");
    std::fs::write(&path, [0x48, 0x65, 0x6c, 0x6c, 0xf8, 0x0a]).unwrap();
    let error = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(error.contains("not valid UTF-8"), "{error}");
    assert!(error.contains("no lossy conversion"), "{error}");
}

#[test]
fn a_file_past_the_editing_limit_is_refused_and_says_how_big_it_is() {
    let app = started();
    let path = scratch("huge").join("huge.md");
    std::fs::write(&path, vec![b'a'; MAX_BYTES + 1]).unwrap();
    let error = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(error.contains("1 MiB"), "{error}");
    assert!(error.contains(&(MAX_BYTES + 1).to_string()), "{error}");
}

#[test]
fn a_binary_file_is_refused_as_one() {
    let app = started();
    let path = scratch("binary").join("binary.md");
    std::fs::write(&path, b"text\x00more text\n").unwrap();
    let error = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(error.contains("control bytes"), "{error}");
}

#[test]
fn a_document_grown_past_the_limit_is_refused_before_it_is_written() {
    // No call can put that much text in the document (the commit refuses it), so the writer's own
    // guard is asked directly: it is the last line before the disk.
    let dir = scratch("grown");
    let path = dir.join("grown.md");
    let job = SaveJob {
        text: "x".repeat(MAX_BYTES + 1),
        target: path.clone(),
        own: None,
        on_disk: None,
        unsure: None,
        inode: None,
        overwrite: false,
        agent: false,
        others: Vec::new(),
        elsewhere: dir.join("kept"),
    };
    let failed = save(&job, &RealDisk).expect_err("refused");
    assert!(failed.error.sentence.contains("1 MiB"), "{}", failed.error.sentence);
    assert!(!path.exists(), "nothing was written");
    assert_eq!(listing(&dir), Vec::<String>::new());
}

#[test]
fn a_failed_save_leaves_no_temporary_file_beside_the_target() {
    let (app, path) = with_file("no-temp-failed", "kept\n");
    let dir = path.parent().unwrap().to_path_buf();
    let other = dir.join("other.md");
    std::fs::write(&other, "not mine\n").unwrap();

    // Every way this can fail, one after another, in one directory.
    let _ = act(&app, "save_as", json!({ "path": other.display().to_string() })); // refused: the path exists
    let _ = act(&app, "save_as", json!({ "path": dir.join("missing-folder/child.md").display().to_string() }));
    let _ = act(&app, "set_content", json!({ "text": "x".repeat(MAX_BYTES + 1) })); // refused before any save
    std::fs::write(&path, "theirs\n").unwrap();
    let _ = act(&app, "save", none()); // refused: changed on disk

    assert_eq!(listing(&dir), vec!["notes.md".to_string(), "other.md".to_string()]);
    assert_eq!(file(&other), "not mine\n");
}

#[test]
fn a_successful_save_leaves_no_temporary_file_either() {
    let (app, path) = with_file("no-temp-saved", "one\n");
    typed(&app, "two\n");
    act(&app, "save", none()).unwrap();
    typed(&app, "three\n");
    act(&app, "save", none()).expect("the replacing save");
    assert_eq!(file(&path), "three\n");
    assert_eq!(listing(path.parent().unwrap()), vec!["notes.md".to_string()]);
    assert_eq!(sentence(&app), "", "and a plain save leaves nothing to say");
}

// ── Recovery ────────────────────────────────────────────────────────────────

#[test]
fn an_unsaved_draft_survives_being_written_out_and_read_back() {
    let app = started();
    let body = "# Half a thought\n\nनमस्ते, \"quoted\"\n";
    typed(&app, body);
    assert!(modified(&app));
    app.run_writes();
    assert!(draft_file(&app).exists());
    let back = killed_and_started_again(app);
    assert_eq!(text(&back), body);
    assert_eq!(path_of(&back), None);
    assert_eq!(state(&back)["recovered_draft"], json!(true), "a recovered draft says so");
    assert!(modified(&back), "and stays dirty until a person agrees to it");
}

#[test]
fn a_draft_the_lane_cannot_read_is_reported_and_left_alone() {
    let paths = Paths::temporary("documents");
    let bad = yantrik_app::drafts::path_in(paths.state_dir(), "doc-unreadable");
    std::fs::create_dir_all(bad.parent().unwrap()).unwrap();
    std::fs::write(&bad, "{ this is not the file we wrote").unwrap();
    let app = TestApp::<YDoc>::start_at(paths, Vec::new());
    settle(&app);
    assert!(notice(&app).contains("1 more draft could not be opened at once"), "{}", notice(&app));
    assert!(notice(&app).contains("kept in yDoc's drafts folder"), "{}", notice(&app));
    assert_eq!(text(&app), "", "nothing was guessed out of it");
    typed(&app, "new work\n");
    app.run_writes();
    settle(&app);
    assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ this is not the file we wrote", "and it really was left alone");
}
