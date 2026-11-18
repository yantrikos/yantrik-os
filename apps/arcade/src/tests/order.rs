//! Answers per invocation, commands in the order they were asked, an older listing never replacing
//! a newer one, and the lists read again when the window comes forward.

use serde_json::json;
use yantrik_app::app::WindowId;

use super::{meadow_run, open, seeded, PIP};
use crate::command::{Command, Landing, Listing, Reply};
use crate::library::CharacterEntry;

/// Two calls in flight at once are each answered from their own work.
#[test]
fn two_calls_asked_together_are_each_answered_from_their_own_result() {
    let f = open();
    let first = f.app.act("new_character", json!({ "spec": PIP })).unwrap();
    let second = f.app.act("new_game", json!({ "spec": meadow_run("Second Run") })).unwrap();
    f.quiet();
    let (a, b) = (first.settle().unwrap(), second.settle().unwrap());
    assert_eq!(a["slug"], json!("pip"));
    assert_eq!(b["slug"], json!("second-run"));
    assert!(a.get("title").is_none() && b.get("archetype").is_none(), "{a} {b}");
}

/// A `build` asked right after an `update_game` compiles the spec that was just saved, whichever
/// job the pool runs first.
#[test]
fn commands_reach_the_library_in_the_order_they_were_asked() {
    let f = open();
    seeded(&f);
    let update = f.app.act("update_game", json!({ "spec": meadow_run("Meadow Run").replace("\"count\": 5", "\"count\": 9") })).unwrap();
    let build = f.app.act("build", json!({ "game": "meadow-run" })).unwrap();
    f.quiet();
    update.settle().unwrap();
    build.settle().unwrap();
    assert!(f.library().game_html("meadow-run").exists());
    assert_eq!(f.state()["games"][0]["built"], json!(true), "the build came after the update, so it stands");
}

fn landing(seq: u64, name: &str) -> Landing {
    Landing {
        seq,
        request: None,
        from_editor: false,
        result: Ok(Reply::Reloaded),
        listing: Listing {
            games: Vec::new(),
            characters: vec![CharacterEntry { slug: name.into(), name: name.into(), archetype: "critter".into() }],
        },
    }
}

#[test]
fn an_older_listing_never_replaces_a_newer_one() {
    let f = open();
    let names = |f: &super::Fixture| f.state()["characters"][0]["name"].clone();
    f.app.run(Command::Landed(Box::new(landing(5, "Newer"))));
    assert_eq!(names(&f), json!("Newer"));
    f.app.run(Command::Landed(Box::new(landing(3, "Older"))));
    assert_eq!(names(&f), json!("Newer"), "the older reading came late and is dropped");
}

#[test]
fn the_lists_are_read_again_when_the_window_comes_forward() {
    let f = open();
    assert_eq!(f.state()["games"], json!([]));
    // Written behind the app's back, as the CLI or another copy would.
    f.library().save_game(&meadow_run("From The Side")).unwrap();
    assert_eq!(f.state()["games"], json!([]), "describe is the model's, not the disk's");
    f.app.activate(WindowId::PRIMARY);
    f.quiet();
    assert_eq!(f.state()["games"][0]["title"], json!("From The Side"));
    assert_eq!(f.app.ui(|ui| slint::Model::row_count(&ui.get_games())), 1);
}

/// A game built from a terminal shows when the window is presented again, not only when it is
/// focused.
#[test]
fn presenting_the_window_reads_the_library_again() {
    let f = open();
    f.library().save_game(&meadow_run("From The Terminal")).unwrap();
    assert_eq!(f.state()["games"], json!([]));
    f.app.present().unwrap();
    f.quiet();
    assert_eq!(f.state()["games"][0]["title"], json!("From The Terminal"));
    f.library().save_game(&meadow_run("And Another")).unwrap();
    f.app.launch_again().unwrap();
    f.quiet();
    assert_eq!(f.state()["games"].as_array().unwrap().len(), 2);
}
