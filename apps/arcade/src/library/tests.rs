//! The library, against real folders under the temp directory.


use super::trash::{copy_dir, percent_encode};
use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn temp_library() -> (Library, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "arcade-lib-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    (Library::at(&dir), dir)
}

const PIP: &str = r##"{
    "name": "Pip",
    "archetype": "critter",
    "ears": "pointy",
    "tail": "curl",
    "palette": { "base": "#44cc88", "belly": "#f2ead8", "accent": "#ff9f43", "nose": "#2f3640", "eye": "#2f3640" }
}"##;

const GRUMBLE: &str = r##"{
    "name": "Grumble",
    "archetype": "brute",
    "ears": "none",
    "stance": "crouched",
    "palette": { "base": "#7a6ce0", "belly": "#d9d2f5", "accent": "#ffd23f", "nose": "#3a2f66", "eye": "#ffd23f" }
}"##;

fn meadow_run(character: &str) -> String {
    format!(
        r##"{{
            "title": "Meadow Run",
            "player": {{ "character": {character} }},
            "collectible": {{ "kind": "berry", "count": 5 }},
            "hazards": [{{ "kind": "wanderer", "speed": 2.0, "count": 2 }}]
        }}"##
    )
}

#[test]
fn slugify_collapses_and_trims() {
    assert_eq!(slugify("Pip's Meadow Run!"), "pip-s-meadow-run");
    assert_eq!(slugify("  Spaced   Out  "), "spaced-out");
    assert_eq!(slugify("!!!"), "");
}

#[test]
fn characters_round_trip() {
    let (lib, dir) = temp_library();
    let entry = lib.save_character(PIP).unwrap();
    assert_eq!(entry.slug, "pip");
    assert_eq!(entry.name, "Pip");
    let loaded = lib.load_character("Pip").unwrap();
    assert_eq!(loaded.name, "Pip");
    // Slug works too, case-insensitively.
    assert_eq!(lib.load_character("pip").unwrap().name, "Pip");
    assert_eq!(lib.load_character("PIP").unwrap().name, "Pip");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_duplicate_character_is_refused_with_a_sentence() {
    let (lib, dir) = temp_library();
    lib.save_character(PIP).unwrap();
    let err = lib.save_character(PIP).unwrap_err();
    assert!(err.contains("already saved"), "{err}");
    assert!(err.contains("Pip"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_invalid_character_is_refused_before_touching_disk() {
    let (lib, dir) = temp_library();
    let err = lib.save_character(r##"{ "name": "X", "palette": { "base": "green", "belly": "#fff", "accent": "#000", "nose": "#111", "eye": "#222" } }"##).unwrap_err();
    assert!(err.contains("palette.base"), "{err}");
    assert!(!lib.characters_dir().exists() || lib.list_characters().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_character_says_the_library_is_empty() {
    let (lib, dir) = temp_library();
    let err = lib.load_character("Nobody").unwrap_err();
    assert!(err.contains("empty"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn characters_list_sorted_by_name() {
    let (lib, dir) = temp_library();
    lib.save_character(PIP).unwrap();
    lib.save_character(GRUMBLE).unwrap();
    let list = lib.list_characters();
    let names: Vec<_> = list.iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, vec!["Grumble", "Pip"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn games_save_then_build_with_a_named_character() {
    let (lib, dir) = temp_library();
    lib.save_character(PIP).unwrap();
    let entry = lib.save_game(&meadow_run("\"Pip\"")).unwrap();
    assert_eq!(entry.slug, "meadow-run");
    assert!(!entry.built);
    let (slug, html) = lib.build_game("Meadow Run").unwrap();
    assert_eq!(slug, "meadow-run");
    assert!(html.exists());
    let text = fs::read_to_string(&html).unwrap();
    assert!(text.contains("window.ARCADE_GAME"));
    assert!(text.contains("\"Pip\""));
    // The list now shows it built, with a timestamp.
    let games = lib.list_games();
    assert_eq!(games.len(), 1);
    assert!(games[0].built);
    assert!(games[0].last_build.is_some());
    assert!(games[0].verification.is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn building_with_an_inline_character_needs_no_library() {
    let (lib, dir) = temp_library();
    lib.save_game(&meadow_run(PIP)).unwrap();
    let (_, html) = lib.build_game("meadow-run").unwrap();
    assert!(html.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn building_with_a_missing_character_names_it() {
    let (lib, dir) = temp_library();
    lib.save_game(&meadow_run("\"Ghost\"")).unwrap();
    let err = lib.build_game("meadow-run").unwrap_err();
    assert!(err.contains("Ghost"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_duplicate_game_is_refused_with_a_sentence() {
    let (lib, dir) = temp_library();
    lib.save_game(&meadow_run(PIP)).unwrap();
    let err = lib.save_game(&meadow_run(PIP)).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rebuild_clears_a_stale_verdict() {
    let (lib, dir) = temp_library();
    lib.save_game(&meadow_run(PIP)).unwrap();
    lib.build_game("meadow-run").unwrap();
    lib.write_verify("meadow-run", &serde_json::json!({
        "passed": true, "when": "2026-09-22 10:00",
        "gates": [{ "gate": "boots", "passed": true }]
    })).unwrap();
    assert!(lib.list_games()[0].verification.as_ref().unwrap().contains("passed all 1 gates"));
    lib.build_game("meadow-run").unwrap();
    assert!(lib.list_games()[0].verification.is_none(), "a fresh build has no verdict yet");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn verify_summary_names_the_first_failed_gate() {
    // A report from before verdicts existed: only `passed` per gate.
    let report = serde_json::json!({
        "passed": false, "when": "2026-09-22 10:00",
        "gates": [
            { "gate": "boots", "passed": true },
            { "gate": "bot_reaches_win", "passed": false, "detail": "bot collected 3 of 10 in 30 s" }
        ]
    });
    let line = verify_summary_line(&report);
    assert!(line.contains("bot_reaches_win"), "{line}");
    assert!(line.contains("3 of 10"), "{line}");
    assert!(line.contains("failed at"), "{line}");
}

#[test]
fn verify_summary_says_when_the_machine_and_not_the_game_decided_it() {
    let report = serde_json::json!({
        "passed": false, "correctness": "inconclusive", "performance": "passed", "when": "2026-09-22 19:21",
        "gates": [
            { "gate": "boots", "passed": true, "verdict": "passed", "detail": "up" },
            { "gate": "bot_reaches_win", "passed": false, "verdict": "inconclusive",
              "detail": "in 285 s of wall clock this machine simulated only 19 of the 95 s budget" },
            { "gate": "frame_budget", "passed": true, "verdict": "passed", "detail": "fine" }
        ]
    });
    let line = verify_summary_line(&report);
    assert!(line.starts_with("verified 2026-09-22 19:21: inconclusive at bot_reaches_win"), "{line}");
    assert!(line.contains(verify::MACHINE_NOT_GAME), "{line}");
    assert!(line.contains("19 of the 95 s"), "{line}");
}

#[test]
fn a_game_can_be_updated_in_place_and_is_then_not_built() {
    let (lib, dir) = temp_library();
    lib.save_game(&meadow_run(PIP)).unwrap();
    lib.build_game("meadow-run").unwrap();
    lib.write_verify("meadow-run", &serde_json::json!({ "passed": true, "when": "x", "gates": [] })).unwrap();
    fs::write(lib.screenshot_path("meadow-run"), b"png").unwrap();
    assert!(lib.list_games()[0].built);

    let softer = meadow_run(PIP).replace("\"count\": 5", "\"count\": 3");
    let entry = lib.update_game(&softer, None).unwrap();
    assert_eq!(entry.slug, "meadow-run");
    assert!(!entry.built, "an updated spec is not built");
    assert!(entry.verification.is_none());
    assert!(!lib.game_html("meadow-run").exists());
    assert!(!lib.verify_path("meadow-run").exists());
    assert!(!lib.screenshot_path("meadow-run").exists());
    // The new spec is what is filed; the old one is one step back.
    let (_, spec) = lib.load_game_spec("Meadow Run").unwrap();
    assert_eq!(spec.collectible.count, 3);
    let previous: GameSpec =
        serde_json::from_str(&fs::read_to_string(lib.game_dir("meadow-run").join("spec.previous.json")).unwrap()).unwrap();
    assert_eq!(previous.collectible.count, 5);
    // Still exactly one game in the list.
    assert_eq!(lib.list_games().len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn updating_a_game_that_does_not_exist_points_at_new_game() {
    let (lib, dir) = temp_library();
    let err = lib.update_game(&meadow_run(PIP), None).unwrap_err();
    assert!(err.contains("Meadow Run"), "{err}");
    assert!(err.contains("new_game"), "{err}");
    assert!(!lib.game_dir("meadow-run").exists(), "a refused update creates nothing");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_update_can_retitle_a_game_when_told_which_one() {
    let (lib, dir) = temp_library();
    lib.save_game(&meadow_run(PIP)).unwrap();
    lib.build_game("meadow-run").unwrap();
    let retitled = meadow_run(PIP).replace("Meadow Run", "Meadow Dusk");
    // Without naming the game, the new title matches nothing.
    let err = lib.update_game(&retitled, None).unwrap_err();
    assert!(err.contains("Meadow Dusk"), "{err}");
    // Naming it moves the game under its new title.
    let entry = lib.update_game(&retitled, Some("Meadow Run")).unwrap();
    assert_eq!(entry.slug, "meadow-dusk");
    assert_eq!(entry.title, "Meadow Dusk");
    assert!(!entry.built);
    assert!(!lib.game_dir("meadow-run").exists());
    assert!(lib.game_dir("meadow-dusk").join("spec.previous.json").exists());
    assert_eq!(lib.list_games().len(), 1);
    // A retitle onto another game's title is refused, and both games stay.
    lib.save_game(&meadow_run(PIP)).unwrap();
    let err = lib.update_game(&retitled, Some("Meadow Run")).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(lib.list_games().len(), 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_invalid_update_leaves_the_saved_game_alone() {
    let (lib, dir) = temp_library();
    lib.save_game(&meadow_run(PIP)).unwrap();
    lib.build_game("meadow-run").unwrap();
    let err = lib.update_game(r##"{ "title": "Meadow Run" }"##, None).unwrap_err();
    assert!(!err.is_empty());
    assert!(lib.game_html("meadow-run").exists(), "the build survives a refused update");
    assert_eq!(lib.load_game_spec("meadow-run").unwrap().1.collectible.count, 5);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn updating_a_character_drops_the_builds_that_cast_it_by_name() {
    let (lib, dir) = temp_library();
    lib.save_character(PIP).unwrap();
    lib.save_character(GRUMBLE).unwrap();
    // Three games: one casts Pip by name, one inlines Pip, one casts Grumble.
    lib.save_game(&meadow_run("\"Pip\"")).unwrap();
    lib.save_game(&meadow_run(PIP).replace("Meadow Run", "Inline Run")).unwrap();
    lib.save_game(&meadow_run("\"Grumble\"").replace("Meadow Run", "Grumble Run")).unwrap();
    for g in ["meadow-run", "inline-run", "grumble-run"] {
        lib.build_game(g).unwrap();
    }
    let redder = PIP.replace("#44cc88", "#cc4444");
    let (entry, stale) = lib.update_character(&redder).unwrap();
    assert_eq!(entry.name, "Pip");
    assert_eq!(stale, vec!["meadow-run".to_string()]);
    assert!(!lib.game_html("meadow-run").exists(), "built with the old Pip, so no longer built");
    assert!(lib.game_html("inline-run").exists(), "an inlined copy is its own");
    assert!(lib.game_html("grumble-run").exists());
    assert_eq!(lib.load_character("Pip").unwrap().palette.unwrap().base, "#cc4444");
    assert_eq!(lib.list_characters().len(), 2, "the kept previous spec is not a third character");
    assert!(lib.characters_dir().join("previous/pip.json").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn updating_a_character_that_does_not_exist_points_at_new_character() {
    let (lib, dir) = temp_library();
    let err = lib.update_character(PIP).unwrap_err();
    assert!(err.contains("Pip"), "{err}");
    assert!(err.contains("new_character"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn delete_moves_the_game_to_the_trash() {
    let (lib, dir) = temp_library();
    // A private pretend-XDG so the test does not touch the real Trash.
    let fake_data = dir.join("home/data");
    let lib = lib.with_trash_at(&fake_data);
    lib.save_game(&meadow_run(PIP)).unwrap();
    lib.build_game("meadow-run").unwrap();
    let msg = lib.delete_game("Meadow Run").unwrap();
    assert!(msg.contains("Trash"), "{msg}");
    assert!(!lib.game_dir("meadow-run").exists());
    let moved: Vec<_> = fs::read_dir(fake_data.join("Trash/files")).unwrap().flatten().collect();
    assert_eq!(moved.len(), 1);
    assert!(moved[0].path().join("index.html").exists());
    let infos: Vec<_> = fs::read_dir(fake_data.join("Trash/info")).unwrap().flatten().collect();
    assert_eq!(infos.len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn percent_encoding_keeps_the_trash_info_legal() {
    assert_eq!(percent_encode("/home/me/My Games"), "/home/me/My%20Games");
}

#[test]
fn deleting_an_unknown_game_refuses_with_a_sentence() {
    let (lib, dir) = temp_library();
    let err = lib.delete_game("nope").unwrap_err();
    assert!(err.contains("nope"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

/// Review M2 (probe): a verdict that lands after its game was deleted must not make the
/// folder again. A folder with only a verify.json in it is invisible to the list and to
/// `delete`, and it keeps the title from being used again.
#[test]
fn a_verdict_for_a_deleted_game_makes_no_ghost_folder() {
    let (lib, dir) = temp_library();
    let lib = lib.with_trash_at(dir.join("trash"));
    lib.save_game(&meadow_run(PIP)).unwrap();
    lib.delete_game("meadow-run").unwrap();
    let err = lib.write_verify("meadow-run", &serde_json::json!({ "passed": true })).unwrap_err();
    assert!(err.contains("meadow-run"), "{err}");
    assert!(!lib.game_dir("meadow-run").exists(), "no folder came back");
    assert!(lib.save_game(&meadow_run(PIP)).is_ok(), "and the title is free");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_verdict_about_a_build_that_changed_is_dropped() {
    let (lib, dir) = temp_library();
    lib.save_character(PIP).unwrap();
    lib.save_game(&meadow_run("\"pip\"")).unwrap();
    lib.build_game("meadow-run").unwrap();
    let judged = stamp_of(&lib.game_html("meadow-run"));
    lib.write_verdict("meadow-run", &serde_json::json!({ "passed": true }), &judged).unwrap();
    assert!(lib.verify_path("meadow-run").exists(), "a verdict about the current build stands");

    // Built again while a run was judging the old file: the old verdict must not be recorded.
    std::thread::sleep(std::time::Duration::from_millis(20));
    lib.update_game(&meadow_run("\"pip\"").replace("\"count\": 5", "\"count\": 6"), None).unwrap();
    lib.build_game("meadow-run").unwrap();
    assert!(!lib.verify_path("meadow-run").exists(), "a build clears the verdict");
    let err = lib.write_verdict("meadow-run", &serde_json::json!({ "passed": true }), &judged).unwrap_err();
    assert!(err.contains("built again"), "{err}");
    assert!(!lib.verify_path("meadow-run").exists(), "so the new build is not shown as verified");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_screenshot_is_not_written_into_a_folder_that_is_gone() {
    let (_, dir) = temp_library();
    let shot = dir.join("games/gone/screenshot.png");
    let err = write_in_place(&shot, b"png").unwrap_err();
    assert!(err.contains("gone"), "{err}");
    assert!(!dir.join("games/gone").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn the_trash_copy_keeps_a_link_a_link() {
    let (_, dir) = temp_library();
    let (from, to) = (dir.join("from"), dir.join("to"));
    fs::create_dir_all(from.join("sub")).unwrap();
    fs::write(from.join("sub/a.txt"), "a").unwrap();
    let secret = dir.join("secret.txt");
    fs::write(&secret, "do not copy me").unwrap();
    std::os::unix::fs::symlink(&secret, from.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(&dir, from.join("dirlink")).unwrap();
    copy_dir(&from, &to).unwrap();
    assert!(fs::symlink_metadata(to.join("link.txt")).unwrap().file_type().is_symlink(), "a link, not its contents");
    assert!(fs::symlink_metadata(to.join("dirlink")).unwrap().file_type().is_symlink(), "a folder link is not walked");
    assert_eq!(fs::read_to_string(to.join("sub/a.txt")).unwrap(), "a");
    let _ = fs::remove_dir_all(&dir);
}

/// Review M4: the window and the command line read one rule. `XDG_DATA_HOME` is not part of it,
/// so a library the old build made under `$HOME/.local/share` is where the window looks.
#[test]
fn the_library_is_where_it_always_was_whatever_xdg_data_home_says() {
    let home = Some(std::ffi::OsString::from("/home/pranab"));
    let expected = PathBuf::from("/home/pranab/.local/share/yantrik/arcade");
    assert_eq!(resolve_root(None, home.clone()), expected);
    assert_eq!(resolve_root(Some("".into()), home.clone()), expected, "an empty override is none");
    assert_eq!(resolve_root(Some("/srv/games".into()), home), PathBuf::from("/srv/games"));
    // `open()` is this rule over the environment and nothing else, so nothing here sets
    // `HOME` or `XDG_DATA_HOME` in a process other tests are running in.
}

