//! The library and its file, tested without a desktop (moved here from `tests/snippets-core`).
//!
//! The fault these cover is the one the survey found: `on_snip_save` was
//! `let _ = (code, tags); // suppress unused warnings`, nothing in the crate ever wrote a file,
//! and the next selection repainted the editor from a model that had never been updated — so an
//! edit reverted while you watched and a restart lost everything. Every test below goes through
//! a real file and a fresh load, because that is the only shape that would have caught it: an
//! in-memory assertion passes on the broken code too.
//!
//! What the app does when a write fails is tested through the app (`tests/writes.rs`): the
//! library here changes memory only, and the lane writes it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::file::{self, STATE_VERSION};
use super::text;
use super::{Library, Patch, Snippet, ALL, FAVORITES, MAX_CODE};

static ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "snippets-test-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    /// The state file, under a directory the writer has to create for itself — the real one is
    /// `~/.local/share/yantrik/snippets` and that directory may not exist.
    fn path(&self) -> PathBuf {
        self.0.join("snippets/snippets.json")
    }

    fn dir(&self) -> PathBuf {
        self.0.join("snippets")
    }

    /// The library written, as the app's lane writes it after every change.
    fn keep(&self, library: &Library) {
        file::write(&self.path(), &file::encode(library).unwrap()).unwrap();
    }

    /// A library read back off disk, which is what a restart is.
    fn reopen(&self) -> Library {
        file::load(&self.path()).library
    }

    fn listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.dir())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn write_state(&self, body: &str) {
        std::fs::create_dir_all(self.dir()).unwrap();
        std::fs::write(self.path(), body).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn seeded(library: &mut Library) -> (i32, i32) {
    let first = library
        .create("Tail the journal", "Shell", "journalctl -fu yantrik-ui", "logs, systemd", ALL)
        .unwrap();
    let second = library
        .create("Borrow checker dance", "Rust", "let mut guard = state.borrow_mut();", "rust, borrow", ALL)
        .unwrap();
    (first, second)
}

// ── It keeps things ──────────────────────────────────────────────

#[test]
fn a_saved_snippet_comes_back_whole_from_a_fresh_load() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let id = library.create("Compose up", "Shell", "docker compose up -d", "docker", ALL).unwrap();
    let collection = library.collection_create("Infra").unwrap();
    library
        .save(
            id,
            Patch {
                title: Some("Compose up, detached".into()),
                language: Some("Shell".into()),
                code: Some("docker compose up -d --build".into()),
                tags: Some("docker, compose".into()),
                favorite: Some(true),
                collection: Some(collection),
            },
        )
        .unwrap();
    library.mark_used(id).unwrap();
    fixture.keep(&library);
    let in_memory: Snippet = library.get(id).cloned().unwrap();

    // The restart. Nothing of the first library survives this line.
    let reopened = fixture.reopen();
    let on_disk = reopened.get(id).cloned().unwrap();

    assert_eq!(in_memory, on_disk, "every field has to survive the round trip, not just the ones the list draws");
    assert_eq!(on_disk.title, "Compose up, detached");
    assert_eq!(on_disk.code, "docker compose up -d --build");
    assert_eq!(on_disk.tags, "docker, compose");
    assert_eq!(on_disk.language, "Shell");
    assert!(on_disk.favorite);
    assert_eq!(on_disk.collection, collection);
    assert_eq!(on_disk.use_count, 1);
    assert!(on_disk.used > 0, "a copied snippet records when it was used");
    assert!(on_disk.created > 0 && on_disk.updated > 0);
    assert_eq!(
        reopened.collection_name(collection),
        "Infra",
        "the collection has to survive too, or its snippets come back orphaned"
    );
}

#[test]
fn editing_one_snippet_leaves_the_others_exactly_as_they_were() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let (first, second) = seeded(&mut library);
    fixture.keep(&library);
    let untouched = library.get(second).cloned().unwrap();

    library
        .save(first, Patch::edits("Tail the shell's journal", "Shell", "journalctl -fu yantrik-ui -n 200", "logs"))
        .unwrap();
    fixture.keep(&library);

    let reopened = fixture.reopen();
    assert_eq!(
        reopened.get(second).cloned().unwrap(),
        untouched,
        "a save writes the whole file, so the test that matters is the OTHER snippet"
    );
    assert_eq!(reopened.get(first).unwrap().code, "journalctl -fu yantrik-ui -n 200");
    assert_eq!(reopened.snippets().len(), 2);
}

#[test]
fn a_favourite_is_still_a_favourite_after_a_restart() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let (first, second) = seeded(&mut library);
    assert!(library.toggle_favorite(second).unwrap());
    fixture.keep(&library);

    let reopened = fixture.reopen();
    assert!(reopened.get(second).unwrap().favorite);
    assert!(!reopened.get(first).unwrap().favorite);
    assert_eq!(reopened.favorites(), 1);
    // Favorites is a filter over what is stored, so this is the same statement twice on purpose:
    // the sidebar count and the rows it would show have to agree.
    assert_eq!(reopened.matching("", "", FAVORITES).len(), 1);
}

#[test]
fn deleting_removes_the_one_named_and_nothing_else() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let (first, second) = seeded(&mut library);

    let removed = library.delete(first).unwrap();
    assert_eq!(removed.id, first);
    fixture.keep(&library);

    let reopened = fixture.reopen();
    assert!(reopened.get(first).is_none());
    assert!(reopened.get(second).is_some());
    assert_eq!(reopened.snippets().len(), 1);
    // The id is not handed out again: a new snippet after a delete must not land on the identity
    // of the one that was thrown away.
    let mut library = reopened;
    let third = library.create("Third", "Rust", "//", "", ALL).unwrap();
    assert!(third > second);
}

#[test]
fn a_change_that_changes_nothing_is_not_a_difference() {
    let mut library = Library::default();
    let (first, _) = seeded(&mut library);
    let stored = library.get(first).cloned().unwrap();
    let same = Patch::edits(&stored.title, &stored.language, &stored.code, &stored.tags);
    assert!(!library.differs(first, &same));
    // What a save would tidy away is not a difference either: the editor's spaces round a title.
    let padded = Patch::edits(&format!("  {}  ", stored.title), &stored.language, &stored.code, "logs,systemd");
    assert!(!library.differs(first, &padded));
    assert!(library.differs(first, &Patch::edits(&stored.title, "Bash", &stored.code, &stored.tags)));
    assert!(!library.differs(999, &same), "a snippet that is not kept has nothing to differ from");
}

// ── It finds things ──────────────────────────────────────────────

#[test]
fn search_matches_title_code_tags_and_language_whatever_the_case() {
    let mut library = Library::default();
    let (journal, borrow) = seeded(&mut library);

    let only = |query: &str| -> Vec<i32> { library.matching(query, "", ALL).iter().map(|s| s.id).collect() };

    assert_eq!(only("JOURNAL"), vec![journal], "title, upper case");
    assert_eq!(only("borrow_mut"), vec![borrow], "code");
    assert_eq!(only("SYSTEMD"), vec![journal], "tags, upper case");
    assert_eq!(only("rUsT"), vec![borrow], "language, mixed case");
    assert_eq!(only("").len(), 2, "an empty query narrows nothing");
    assert!(only("nothing here at all").is_empty());

    // The tag chips are an exact match on one tag, not a substring of the tag string: `log` is
    // not the `logs` chip.
    assert_eq!(library.matching("", "logs", ALL).len(), 1);
    assert!(library.matching("", "log", ALL).is_empty());
}

#[test]
fn a_snippet_can_be_named_by_id_or_by_title() {
    let mut library = Library::default();
    let (journal, _) = seeded(&mut library);

    assert_eq!(library.find(&journal.to_string()).unwrap().id, journal);
    assert_eq!(library.find("tail the journal").unwrap().id, journal);
    assert_eq!(library.find("journal").unwrap().id, journal);
    let ambiguous = library.find("o").unwrap_err();
    assert!(ambiguous.contains("matches 2"), "{ambiguous}");
    let missing = library.find("nothing").unwrap_err();
    assert!(missing.contains("nothing"), "{missing}");
}

// ── It does not lose things when the file is wrong ───────────────

#[test]
fn a_file_it_cannot_read_is_kept_aside_and_the_app_still_starts() {
    let fixture = Fixture::new();
    fixture.write_state("{this is not json at all");

    let loaded = file::load(&fixture.path());
    assert!(loaded.library.snippets().is_empty(), "an unreadable file starts empty");
    assert!(loaded.notice.contains("could not be read"), "and says so: {}", loaded.notice);

    let kept: Vec<String> =
        fixture.listing().into_iter().filter(|n| n.starts_with("snippets.json.corrupt-")).collect();
    assert_eq!(kept.len(), 1, "the bytes are kept: {:?}", fixture.listing());
    assert_eq!(
        std::fs::read_to_string(fixture.dir().join(&kept[0])).unwrap(),
        "{this is not json at all",
        "kept exactly, because it is the only record of what the person had"
    );
    assert!(!fixture.path().exists(), "and moved, not copied — the next save must not write over it");
}

#[test]
fn a_file_from_another_version_is_kept_aside_rather_than_guessed_at() {
    let fixture = Fixture::new();
    fixture.write_state(&format!(
        r#"{{"version": {}, "snippets": [{{"id": 1, "title": "From the future"}}]}}"#,
        STATE_VERSION + 7
    ));

    let loaded = file::load(&fixture.path());
    assert!(loaded.library.snippets().is_empty());
    assert!(loaded.notice.contains("version"), "{}", loaded.notice);
    assert!(fixture.listing().iter().any(|n| n.starts_with("snippets.json.corrupt-")));
}

#[test]
fn one_unusable_row_does_not_cost_the_others() {
    let fixture = Fixture::new();
    fixture.write_state(&format!(
        r#"{{"version": {STATE_VERSION}, "snippets": [
            {{"id": 0, "title": "No id"}},
            {{"id": 4, "title": "Real", "code": "ok"}},
            {{"id": 4, "title": "Same id again", "code": "not ok"}}
        ]}}"#
    ));

    let loaded = file::load(&fixture.path());
    assert_eq!(loaded.library.snippets().len(), 1, "id 0 and the repeat are dropped");
    assert_eq!(loaded.library.get(4).unwrap().title, "Real");
    assert!(loaded.notice.is_empty(), "a readable file is not a fault");
    // The next id has to clear what was loaded, or the next create lands on top of id 4.
    let mut library = loaded.library;
    assert_eq!(library.create("Next", "Rust", "//", "", ALL).unwrap(), 5);
}

#[test]
fn a_read_back_never_moves_a_file_aside() {
    let fixture = Fixture::new();
    fixture.write_state("{half a file");
    assert!(file::read_back(&fixture.path()).is_err(), "an unreadable file is said, not taken for empty");
    assert!(fixture.path().exists(), "and left where it is: moving it aside is the start's decision");
    let empty = Fixture::new();
    assert_eq!(file::read_back(&empty.path()).unwrap(), Library::default(), "no file is nothing kept");
}

#[test]
fn no_half_written_file_is_left_beside_the_real_one() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let (first, _) = seeded(&mut library);
    library.mark_used(first).unwrap();
    library.collection_create("Infra").unwrap();
    fixture.keep(&library);
    fixture.keep(&library);

    assert_eq!(fixture.listing(), vec!["snippets.json".to_string()], "a temp file left behind is the next start's corrupt file");
}

// ── Collections, export and import ───────────────────────────────

#[test]
fn deleting_a_collection_keeps_its_snippets_and_moves_them_to_all() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let infra = library.collection_create("Infra").unwrap();
    let id = library.create("Compose up", "Shell", "docker compose up", "docker", infra).unwrap();
    assert_eq!(library.get(id).unwrap().collection, infra);
    assert_eq!(library.collection_rename(infra, "Machines").unwrap(), "Machines");
    assert!(library.collection_create("machines").is_err(), "a name already in use is refused whatever its case");

    assert_eq!(library.collection_delete(infra).unwrap(), 1);
    fixture.keep(&library);
    let reopened = fixture.reopen();
    assert!(reopened.collections().is_empty());
    assert_eq!(reopened.get(id).unwrap().collection, ALL, "the folder went, the code stayed");
}

#[test]
fn an_export_can_be_imported_into_an_empty_store() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let (first, _) = seeded(&mut library);
    library.toggle_favorite(first).unwrap();
    let bundle = fixture.0.join("export.json");
    file::write(&bundle, &file::encode_bundle(&library).unwrap()).unwrap();

    let elsewhere = Fixture::new();
    let mut fresh = elsewhere.reopen();
    let imported = fresh.import(file::read_import(&bundle).unwrap(), ALL).unwrap();
    assert_eq!(imported.kind, "bundle");
    assert_eq!(imported.ids.len(), 2);
    elsewhere.keep(&fresh);

    let reopened = elsewhere.reopen();
    assert_eq!(reopened.snippets().len(), 2);
    assert_eq!(reopened.favorites(), 1);
    assert!(reopened.snippets().iter().any(|s| s.code == "journalctl -fu yantrik-ui"));
    // Fresh ids: an import is a copy, and a bundle carrying id 1 must not land on the id 1 that
    // is already here.
    let mut second_pass = reopened;
    let again = second_pass.import(file::read_import(&bundle).unwrap(), ALL).unwrap();
    assert!(again.ids.iter().all(|id| !imported.ids.contains(id)));
    assert_eq!(second_pass.snippets().len(), 4);
    // The state file itself reads as an export.
    elsewhere.keep(&second_pass);
    let third = Library::default().import(file::read_import(&elsewhere.path()).unwrap(), ALL).unwrap();
    assert_eq!(third.ids.len(), 4);
}

#[test]
fn a_bundle_with_one_row_too_big_is_refused_whole() {
    let mut library = Library::default();
    let (first, _) = seeded(&mut library);
    library.save(first, Patch { code: Some("x".repeat(MAX_CODE)), ..Patch::default() }).unwrap();
    let mut rows: Vec<Snippet> = library.snippets().to_vec();
    rows[0].code.push('x');
    let mut target = Library::default();
    let before = target.clone();
    assert!(target.import(file::Incoming::Bundle(rows), ALL).unwrap_err().contains("KiB"));
    assert_eq!(target, before, "a bundle half imported is worse than one not imported at all");
}

#[test]
fn a_source_file_becomes_one_snippet_with_its_language_guessed() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let source = fixture.0.join("deploy.sh");
    std::fs::write(&source, "#!/bin/sh\nset -eu\nrsync -a ./ vm:/opt/yantrik\n").unwrap();

    let imported = library.import(file::read_import(&source).unwrap(), ALL).unwrap();
    assert_eq!(imported.kind, "file");
    let kept = library.get(imported.ids[0]).cloned().unwrap();
    assert_eq!(kept.title, "deploy.sh");
    assert_eq!(kept.language, "Shell");
    assert!(kept.code.contains("rsync -a ./ vm:/opt/yantrik"));
    fixture.keep(&library);
    assert_eq!(
        fixture.reopen().get(imported.ids[0]).unwrap().code,
        kept.code,
        "a file taken from the command line is kept like anything else"
    );
    let missing = file::read_import(&fixture.0.join("nothing.sh")).err().unwrap();
    assert!(missing.starts_with("no file at"), "{missing}");
}

#[test]
fn tags_are_tidied_once_so_the_chips_do_not_repeat() {
    let mut library = Library::default();
    let id = library.create("Thing", "Rust", "//", " rust ,, RUST, borrow ", ALL).unwrap();
    assert_eq!(library.get(id).unwrap().tags, "rust, borrow");
    assert_eq!(library.tags(), vec!["borrow".to_string(), "rust".to_string()]);
}

#[test]
fn a_snippet_always_has_a_title_and_never_more_code_than_the_limit() {
    let fixture = Fixture::new();
    let mut library = fixture.reopen();
    let id = library.create("   ", "Rust", "//", "", ALL).unwrap();
    assert_eq!(library.get(id).unwrap().title, "Untitled Snippet");

    let huge = "x".repeat(MAX_CODE + 1);
    let refused = library.create("Too big", "Rust", &huge, "", ALL).unwrap_err();
    assert!(refused.contains("KiB"), "{refused}");
    assert_eq!(library.snippets().len(), 1, "and it was not kept");
    fixture.keep(&library);
    assert_eq!(fixture.reopen().snippets().len(), 1);
}

#[test]
fn a_snippet_prints_without_its_code() {
    let mut library = Library::default();
    let id = library.create("Token", "Shell", "export TOKEN=hunter2-SENTINEL", "", ALL).unwrap();
    let printed = format!("{:?}", library.get(id).unwrap());
    assert!(!printed.contains("SENTINEL") && printed.contains("bytes"), "{printed}");
    assert!(!format!("{library:?}").contains("SENTINEL"));
}

#[test]
fn dates_are_read_as_dates_and_not_as_epoch_seconds() {
    // 2026-09-20T00:00:00Z. The list draws these, and an off-by-one in the civil-date arithmetic
    // would be invisible until somebody read a snippet's created date.
    assert_eq!(text::date(1_789_862_400), "2026-09-20");
    assert_eq!(text::date(0), "1970-01-01");
    assert_eq!(text::relative(0, 1_789_862_400), "");
    assert_eq!(text::relative(1_789_862_400, 1_789_862_410), "just now");
    assert_eq!(text::relative(1_789_862_400, 1_789_866_000), "1 hour ago");
}
