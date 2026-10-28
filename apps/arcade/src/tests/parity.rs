//! The parity pin: the action table and `describe` exactly as Arcade published them before the
//! framework. The table below was extracted mechanically from the old `specs()` in
//! `apps/arcade/src/main.rs` (at fe2b8b41), with the string literals evaluated as rustc reads them,
//! not copied by hand from the new intents, so a word changed in `intents.rs` fails here. The two
//! long argument texts hold the old `{}` where the vocabulary line went; it is filled in from
//! `spec::vocabulary_line` as the old `format!` did. The keys of `describe` were extracted from
//! the old `view` the same way, in the order it set them.

use serde_json::{json, Value};
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{open, seeded};
use crate::Arcade;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Whether the action says the work is only started when it returns.
    deferred: bool,
    /// Name, JSON type, required, description.
    params: Vec<(&'static str, &'static str, bool, String)>,
}

fn before() -> Vec<Golden> {
    vec![
        Golden {
            name: "new_character",
            risk: "standard",
            description: r####"Save a character spec into the library"####,
            deferred: false,
            params: vec![
                ("spec", "string", true, r####"The character JSON. Required: name, archetype, proportions (head_body, limb_length, width). Optional: ears, tail, palette (base/belly/accent/nose/eye as #rrggbb), expression, stance. The enumerated fields take {}. Ranges and the rest are in `describe`'s spec_grammar. A value outside any of them is refused with a sentence naming the field."####.replace("{}", &crate::spec::vocabulary_line("character"))),
            ],
        },
        Golden {
            name: "new_game",
            risk: "standard",
            description: r####"Save a game spec into the library"####,
            deferred: false,
            params: vec![
                ("spec", "string", true, r####"The game JSON. Required: title, arena (size, theme), player (character, speed), collectible (kind, count), hazards[] (kind, speed, count), lives, music. `player.character` may name a saved character or inline a whole one, in which case the character vocabulary applies to it too. The enumerated fields take {}. Ranges are in `describe`'s spec_grammar."####.replace("{}", &crate::spec::vocabulary_line("game"))),
            ],
        },
        Golden {
            name: "update_game",
            risk: "standard",
            description: r####"Replace a saved game's spec; the old build, verdict and screenshot go with the old spec, and `build` runs again"####,
            deferred: false,
            params: vec![
                ("spec", "string", true, r####"The whole game JSON, same grammar as new_game; its title says which saved game it replaces. The spec it replaces is kept beside it as spec.previous.json."####.to_string()),
                ("game", "string", false, r####"Optional: title or slug of the saved game to replace, for when the new spec changes the title"####.to_string()),
            ],
        },
        Golden {
            name: "update_character",
            risk: "standard",
            description: r####"Replace a saved character's spec; games that cast it by name lose their builds and need `build` again"####,
            deferred: false,
            params: vec![
                ("spec", "string", true, r####"The whole character JSON, same grammar as new_character; its name says which saved character it replaces. Games that inline a copy are untouched."####.to_string()),
            ],
        },
        Golden {
            name: "build",
            risk: "standard",
            description: r####"Compile a saved game into its one HTML file"####,
            deferred: false,
            params: vec![
                ("game", "string", true, r####"Title or slug of a saved game"####.to_string()),
            ],
        },
        Golden {
            name: "play",
            risk: "standard",
            description: r####"Open a built game in the desktop Browser and confirm it draws; if the Browser has no WebGL, say so and take a headless screenshot instead"####,
            deferred: true,
            params: vec![
                ("game", "string", true, r####"Title or slug of a built game"####.to_string()),
            ],
        },
        Golden {
            name: "verify",
            risk: "standard",
            description: r####"Run the headless gates: boots, clean console, frame renders, input moves, bot wins, bot loses, frame budget. Each gate ends passed, failed or inconclusive; inconclusive means this machine could not settle it (too slow to run a bot to the end of its simulated budget, or too slow for the frame budget to judge the game at all) and is not a verdict on the game, though it is not a pass either"####,
            deferred: true,
            params: vec![
                ("game", "string", true, r####"Title or slug of a built game"####.to_string()),
            ],
        },
        Golden {
            name: "screenshot",
            risk: "standard",
            description: r####"Take a headless PNG of a built game"####,
            deferred: true,
            params: vec![
                ("game", "string", true, r####"Title or slug of a built game"####.to_string()),
            ],
        },
        Golden {
            name: "delete",
            risk: "sensitive",
            description: r####"Move a game to the Trash, where Files can bring it back"####,
            deferred: false,
            params: vec![
                ("game", "string", true, r####"Title or slug of a saved game"####.to_string()),
            ],
        },
    ]
}

/// The keys of `describe`'s state, in the order the old `view` set them.
const KEYS: &[&str] = &["library", "selected", "games", "characters", "jobs", "spec_grammar"];

/// The grammar's note, word for word (spaces and all).
const NOTE: &str = "Every enumerated field and its words, and every numeric field and its                          range, exactly as new_character and new_game enforce them.";

#[test]
fn the_nine_actions_are_published_as_they_were() {
    let mut table = Intents::<Arcade>::new();
    Arcade::intents(&mut table);
    let specs = table.specs();
    let golden = before();
    let names: Vec<&str> = specs.iter().map(|a| a.name.as_str()).collect();
    let was: Vec<&str> = golden.iter().map(|g| g.name).collect();
    assert_eq!(names, was, "the same actions, in the same order");
    for (spec, golden) in specs.iter().zip(&golden) {
        let name = golden.name;
        assert_eq!(spec.description, golden.description, "{name}: description");
        assert_eq!(spec.permission, golden.risk, "{name}: grade");
        assert_eq!(spec.expected_seconds, None, "{name}: expected seconds");
        assert_eq!(spec.deferred, golden.deferred, "{name}: deferred");
        assert!(!spec.stateless && spec.explainer.is_none(), "{name}");
        let params: Vec<(&str, &str, bool, &str)> =
            spec.params.iter().map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str())).collect();
        let want: Vec<(&str, &str, bool, &str)> = golden.params.iter().map(|(n, k, r, d)| (*n, *k, *r, d.as_str())).collect();
        assert_eq!(params, want, "{name}: parameters");
    }
}

/// What a caller sees in `describe`: the nine, and the framework's `launch`, which every app on
/// the framework publishes. (`present`, the framework's other one, is added by the runner when it
/// serves the surface.)
#[test]
fn describe_lists_the_nine_and_the_frameworks_launch() {
    let f = open();
    let actions = f.app.describe()["actions"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let mut expected: Vec<String> = before().iter().map(|g| g.name.to_string()).collect();
    expected.push("launch".to_string());
    names.sort();
    expected.sort();
    assert_eq!(names, expected);
    for golden in before() {
        let action = actions.iter().find(|a| a["name"] == json!(golden.name)).expect("published");
        assert_eq!(action["permission"], json!(golden.risk), "{}", golden.name);
    }
}

/// docs/app-control.md's rows for Arcade say what the app is and what it publishes.
#[test]
fn the_guides_rows_are_still_true() {
    let doc = include_str!("../../../../docs/app-control.md");
    let surface = doc.lines().find(|l| l.starts_with("| `arcade` |")).expect("the surfaces table names it");
    let cells: Vec<&str> = surface.split('|').map(str::trim).collect();
    assert_eq!(cells[2], "`app-arcade.sock`", "the socket is the id's: {surface}");
    let mut listed: Vec<(String, String)> = Vec::new();
    for row in doc.lines().filter(|l| l.starts_with("| arcade | `")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        for name in cells[2].split(',') {
            listed.push((name.trim().trim_matches('`').to_string(), cells[3].to_string()));
        }
    }
    listed.sort();
    let mut all: Vec<(String, String)> = before().iter().map(|g| (g.name.to_string(), g.risk.to_string())).collect();
    all.sort();
    assert_eq!(listed, all, "every action once, at the grade the code holds");
}

fn keys(state: &Value) -> Vec<String> {
    let mut keys: Vec<String> = state.as_object().expect("an object").keys().cloned().collect();
    keys.sort();
    keys
}

fn expected() -> Vec<String> {
    let mut keys: Vec<String> = KEYS.iter().map(|k| k.to_string()).collect();
    keys.sort();
    keys
}

#[test]
fn an_empty_library_is_described_as_it_was() {
    let f = open();
    let d = f.app.describe();
    assert_eq!(d["summary"], json!("Arcade \u{2014} 0 games, 0 characters"));
    let s = &d["state"];
    assert_eq!(keys(s), expected());
    assert_eq!(
        *s,
        json!({
            "library": f.paths.data_dir().display().to_string(),
            "selected": "",
            "games": [],
            "characters": [],
            "jobs": [],
            "spec_grammar": {
                "vocabularies": crate::spec::vocabularies(),
                "ranges": crate::spec::ranges(),
                "note": NOTE,
            },
        })
    );
}

#[test]
fn a_library_with_a_character_and_a_game_is_described_as_it_was() {
    let f = open();
    seeded(&f);
    let d = f.app.describe();
    assert_eq!(d["summary"], json!("Arcade \u{2014} 1 games, 1 characters"));
    assert_eq!(keys(&d["state"]), expected());
    assert_eq!(d["state"]["selected"], json!("meadow-run"), "a saved game is left selected, as before");
    assert_eq!(d["state"]["games"], json!([{ "slug": "meadow-run", "title": "Meadow Run", "built": false, "last_build": null, "verification": null }]));
    assert_eq!(d["state"]["characters"], json!([{ "name": "Pip", "slug": "pip", "archetype": "critter" }]));
}
