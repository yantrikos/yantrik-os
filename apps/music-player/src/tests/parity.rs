//! The parity pin: the action table and `describe` exactly as Music published them before the
//! framework. The table below was extracted mechanically from the old `apps/music-player/src/main.rs`
//! (`surface`), with the string literals evaluated as rustc reads them, not copied by hand from the
//! new intents, so a word changed in `intents.rs` fails here. The keys of `describe` were extracted
//! from the old `view` the same way.

use serde_json::{json, Value};
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{open, open_with, Setup};
use crate::Music;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    expected_seconds: Option<u32>,
    /// Name, JSON type, required, description.
    params: &'static [(&'static str, &'static str, bool, &'static str)],
}

const BEFORE: &[Golden] = &[
    Golden {
        name: "play",
        risk: "standard",
        description: r#"Start or resume playback. With `index`, play that track of the library list (1 is the first) and make the list the queue; without it, resume what is paused, or play the queue's current track, or the library from the top. Answers with what mpv reports once it is playing."#,
        expected_seconds: None,
        params: &[
            ("index", "integer", false, r#"A track number in the library list, from 1, as `describe.library` numbers them."#),
        ],
    },
    Golden {
        name: "pause",
        risk: "standard",
        description: r#"Pause what is playing. Answers with mpv's own report of pause and position."#,
        expected_seconds: None,
        params: &[],
    },
    Golden {
        name: "next",
        risk: "standard",
        description: r#"Play the next track in the queue. Refused at the end of the queue."#,
        expected_seconds: None,
        params: &[],
    },
    Golden {
        name: "previous",
        risk: "standard",
        description: r#"Go back: to the start of the track if it has played more than three seconds, otherwise to the track before it in the queue."#,
        expected_seconds: None,
        params: &[],
    },
    Golden {
        name: "seek",
        risk: "standard",
        description: r#"Move to a point in the playing track. Answers with the position mpv reports after the move."#,
        expected_seconds: None,
        params: &[
            ("seconds", "number", true, r#"Seconds from the start of the track."#),
        ],
    },
    Golden {
        name: "set_volume",
        risk: "standard",
        description: r#"Set the volume, 0 to 100. Answers with the volume mpv reports."#,
        expected_seconds: None,
        params: &[
            ("percent", "number", true, r#"0 is silent, 100 is full volume."#),
        ],
    },
    Golden {
        name: "play_file",
        risk: "standard",
        description: r#"Play one sound file now, by its path, as the whole queue. An agent may name only files in the person's home outside its protected places."#,
        expected_seconds: None,
        params: &[
            ("path", "string", true, r#"The file's path: absolute, or starting with ~/. mp3, flac, ogg, opus, wav or m4a."#),
        ],
    },
    Golden {
        name: "rescan",
        risk: "standard",
        description: r#"Scan the library folders again and answer with what was found: how many tracks in each folder, and any folder that could not be read."#,
        expected_seconds: Some(30),
        params: &[],
    },
];

/// The keys of `describe`'s state, in the order the old `view` set them.
const KEYS: &[&str] = &[
    "track", "path", "position", "duration", "playing", "paused", "volume", "muted", "queue_length",
    "queue_position", "library_size", "library", "library_listed", "folders", "scanning", "player",
    "player_problem", "status", "notice",
];

#[test]
fn the_eight_actions_are_published_as_they_were() {
    let mut table = Intents::<Music>::new();
    Music::intents(&mut table);
    let specs = table.specs();
    let names: Vec<&str> = specs.iter().map(|a| a.name.as_str()).collect();
    let before: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    assert_eq!(names, before, "the same actions, in the same order");
    for (spec, golden) in specs.iter().zip(BEFORE) {
        let name = golden.name;
        assert_eq!(spec.description, golden.description, "{name}: description");
        assert_eq!(spec.permission, golden.risk, "{name}: grade");
        assert_eq!(spec.expected_seconds, golden.expected_seconds, "{name}: expected seconds");
        assert!(!spec.deferred, "{name}: answered once its work is done, as before");
        assert!(!spec.stateless && spec.explainer.is_none(), "{name}");
        let params: Vec<(&str, &str, bool, &str)> =
            spec.params.iter().map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str())).collect();
        assert_eq!(params, golden.params, "{name}: parameters");
    }
}

/// What a caller sees in `describe`: the eight, and the framework's `launch`, which every app on
/// the framework publishes. (`present`, the framework's other one, is added by the runner when it
/// serves the surface.)
#[test]
fn describe_lists_the_eight_and_the_frameworks_launch() {
    let Some(f) = open("parity-actions") else { return };
    let actions = f.app.describe()["actions"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let mut expected: Vec<String> = BEFORE.iter().map(|g| g.name.to_string()).collect();
    expected.push("launch".to_string());
    names.sort();
    expected.sort();
    assert_eq!(names, expected);
    for golden in BEFORE {
        let action = actions.iter().find(|a| a["name"] == json!(golden.name)).expect("published");
        assert_eq!(action["permission"], json!(golden.risk), "{}", golden.name);
        assert_eq!(action["settles"], json!("on return"), "{}", golden.name);
    }
}

/// docs/app-control.md's rows for Music say what the app is and what it publishes.
#[test]
fn the_guides_rows_are_still_true() {
    let doc = include_str!("../../../../docs/app-control.md");
    let surface = doc.lines().find(|l| l.starts_with("| `music` |")).expect("the surfaces table names music");
    let cells: Vec<&str> = surface.split('|').map(str::trim).collect();
    assert_eq!(cells[2], "`app-music.sock`", "the socket is the id's: {surface}");
    assert_eq!(cells[3], "`music-player`");
    let grades = doc.lines().find(|l| l.starts_with("| music | `")).expect("the grade table has music's row");
    let cells: Vec<&str> = grades.split('|').map(str::trim).collect();
    let mut listed: Vec<&str> = cells[2].split(',').map(|n| n.trim().trim_matches('`')).collect();
    let mut all: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    listed.sort();
    all.sort();
    assert_eq!(listed, all, "every action, once");
    assert!(BEFORE.iter().all(|g| g.risk == cells[3]), "{grades}");
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
fn nothing_playing_is_described_as_it_was() {
    let Some(f) = open("parity-idle") else { return };
    let d = f.app.describe();
    assert_eq!(d["summary"], json!("Music \u{2014} nothing playing, library 300 tracks"));
    let s = &d["state"];
    assert_eq!(keys(s), expected());
    let music = f.music.to_string_lossy().into_owned();
    let mut state = s.clone();
    state["library"] = json!(state["library"].as_array().map(Vec::len));
    assert_eq!(
        state,
        json!({
            "track": null,
            "path": null,
            "position": null,
            "duration": null,
            "playing": false,
            "paused": false,
            "volume": 40.0,
            "muted": false,
            "queue_length": 0,
            "queue_position": null,
            "library_size": 300,
            "library": 50,
            "library_listed": 300,
            "folders": [{"path": music, "tracks": 300}],
            "scanning": false,
            "player": "not started",
            "player_problem": null,
            "status": "300 tracks in 1 folder",
            "notice": "",
        })
    );
    assert_eq!(s["library"][0], json!({"index": 1, "title": "Alpha 000", "folder": "Alpha"}));
    assert_eq!(s["library"][49], json!({"index": 50, "title": "Alpha 049", "folder": "Alpha"}));
}

#[test]
fn playing_and_paused_are_described_as_they_were() {
    let Some(f) = open("parity-playing") else { return };
    f.act("play", json!({"index": 1})).unwrap();
    f.until("mpv's report of the length is in", |f| f.state()["duration"] == 200.0);
    let d = f.app.describe();
    assert_eq!(d["summary"], json!("Music \u{2014} playing \u{201c}Alpha 000\u{201d} (0:00 of 3:20), library 300 tracks"));
    let s = &d["state"];
    assert_eq!(keys(s), expected());
    assert_eq!(s["track"], "Alpha 000");
    assert_eq!(s["path"], json!(f.track("Alpha", 0)));
    assert_eq!((s["position"].clone(), s["playing"].clone(), s["paused"].clone()), (json!(0.0), json!(true), json!(false)));
    assert_eq!((s["queue_length"].clone(), s["queue_position"].clone()), (json!(300), json!(1)));
    assert_eq!(s["player"], "running");
    assert_eq!(s["status"], "Playing \u{201c}Alpha 000\u{201d}");
    f.act("pause", json!({})).unwrap();
    f.until("paused", |f| f.state()["paused"] == true);
    assert_eq!(f.summary(), "Music \u{2014} paused on \u{201c}Alpha 000\u{201d} at 0:00, library 300 tracks");
    // A notice and a scan are said in the summary.
    let _ = f.act("seek", json!({"seconds": 500.0}));
    f.app.ui(|ui| ui.invoke_scan_rescan());
    assert_eq!(
        f.summary(),
        "Music \u{2014} paused on \u{201c}Alpha 000\u{201d} at 0:00, library 300 tracks \u{b7} scanning \u{b7} The track is 3:20 long; 500 seconds is past its end."
    );
}

#[test]
fn a_player_that_cannot_start_is_described_as_it_was() {
    let Some(f) = open_with("parity-unavailable", Setup { player: false, ..Setup::default() }) else { return };
    let _ = f.act("play", json!({}));
    let s = f.state();
    assert_eq!(s["player"], "unavailable");
    assert_eq!(s["player_problem"], "Music cannot play: the player it uses, mpv, is not installed on this machine.");
    assert_eq!(s["notice"], s["player_problem"]);
}

/// Nothing the screen used to draw over nothing is back (`tests/app-lints` checks that every
/// callback it declares is wired).
#[test]
fn nothing_the_screen_once_drew_over_nothing_is_back() {
    let screen: String = include_str!("../../../../crates/yantrik-ui-slint/ui/music_player.slint")
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for gone in ["playlist", "shuffle", "repeat", "equalizer", "playback-speed", "folder-watch", "ai-explain"] {
        assert!(!screen.contains(gone), "`{gone}` is back on the screen with nothing behind it");
    }
}
