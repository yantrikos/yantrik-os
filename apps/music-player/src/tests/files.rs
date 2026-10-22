//! One sound file by its path: `play_file`, a file named on the command line, a second launch
//! with a file, and the files that are not sound: missing, not audio, unreadable to mpv, or a
//! playlist dressed as an mp3.

use serde_json::json;
use yantrik_app::app::Event;
use yantrik_app::launch::Launch;

use super::{open, open_with, Setup};

#[test]
fn play_file_makes_the_file_the_whole_queue_and_refuses_what_is_not_a_sound_file() {
    let Some(f) = open("play-file") else { return };
    let one = f.act("play_file", json!({"path": f.track("Gamma", 7)})).unwrap();
    assert_eq!(one["track"], "Gamma 007");
    assert_eq!(f.state()["queue_length"], 1);
    let refused = f.act("play_file", json!({"path": f.music.join("cover.jpg")})).unwrap_err();
    assert!(refused.contains("not a sound file"), "{refused}");
    assert_eq!(f.notice(), refused, "and the window says so too");
    assert!(f.act("play_file", json!({"path": f.dir.join("nothing.mp3")})).unwrap_err().contains("no file"));
    assert!(f.act("play_file", json!({"path": f.music})).unwrap_err().ends_with("is not a file."));
    assert_eq!(f.act("play_file", json!({"path": "  "})).unwrap_err(), "`play_file` needs `path`.");
    assert_eq!(f.state()["queue_length"], 1, "a refused file leaves the queue as it was");
}

#[test]
fn what_mpv_says_about_a_file_it_cannot_read_is_what_the_caller_hears() {
    let Some(f) = open("unreadable") else { return };
    let broken = f.music.join("broken.mp3");
    std::fs::write(&broken, b"").unwrap();
    let err = f.act("play_file", json!({"path": broken})).unwrap_err();
    assert!(err.contains("unrecognized file format"), "{err}");
    f.until("the window hears it too", |f| f.notice().contains("unrecognized file format"));
    assert_eq!(f.notice(), "Could not play \u{201c}broken\u{201d}: unrecognized file format.");
    // An old failure is not the answer to the next load.
    let good = f.act("play_file", json!({"path": f.track("Gamma", 8)})).unwrap();
    assert_eq!(good["track"], "Gamma 008", "{good}");
    assert!(f.notice().is_empty(), "the complaint went with the next play: {}", f.notice());
}

#[test]
fn a_playlist_dressed_as_an_mp3_is_refused_stopped_and_never_reported_as_playing() {
    let Some(f) = open("redirect") else { return };
    let playlist = f.music.join("playlist.mp3");
    std::fs::write(&playlist, b"#EXTM3U\nhttp://127.0.0.1:9/beacon.mp3\n").unwrap();
    let err = f.act("play_file", json!({"path": playlist})).unwrap_err();
    assert!(err.contains("playlist"), "the answer says what mpv did: {err}");
    f.until("what the playlist pointed at was stopped", |f| f.playing_path().is_none());
    assert!(f.notice().contains("playlist"), "{}", f.notice());
    assert!(!f.playing());
    let good = f.act("play_file", json!({"path": f.track("Gamma", 9)})).unwrap();
    assert_eq!(good["track"], "Gamma 009", "{good}");
}

#[test]
fn a_file_named_on_the_command_line_plays_at_start() {
    let dir = super::scratch("argv-file");
    let file = dir.join("opening.flac");
    std::fs::write(&file, b"").unwrap();
    let Some(f) = open_with("argv", Setup { args: vec![file.clone().into()], ..Setup::default() }) else { return };
    let want = file.to_string_lossy().into_owned();
    f.until("the file plays", |f| f.playing_path().as_deref() == Some(want.as_str()));
    assert_eq!(f.state()["queue_length"], 1);
    assert_eq!(f.state()["library_size"], 300, "and the library was scanned beside it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_launch_with_a_file_plays_it_in_the_window_already_open() {
    let Some(f) = open("handover") else { return };
    let profile = f.app.paths().profile().to_string();
    let launch = Launch::of_files(&profile, &f.dir, [f.track("Beta", 4)]);
    f.app.event(Event::Launch(launch));
    let want = f.track("Beta", 4).to_string_lossy().into_owned();
    f.until("the handed-over file plays", |f| f.playing_path().as_deref() == Some(want.as_str()));
    // A launch with nothing to open opens nothing, and leaves what plays.
    f.app.launch_again().unwrap();
    f.quiet();
    assert_eq!(f.playing_path().as_deref(), Some(want.as_str()));
    // A file that is not sound is said in the window; nobody else is there to hear it.
    let launch = Launch::of_files(&profile, &f.dir, [f.music.join("cover.jpg")]);
    f.app.event(Event::Launch(launch));
    f.quiet();
    assert!(f.notice().contains("not a sound file"), "{}", f.notice());
}

/// A name with a newline or a direction override is said as one line of plain text.
#[test]
fn a_name_is_said_as_one_line_of_plain_text() {
    let Some(f) = open("names") else { return };
    let odd = f.dir.join("two\nlines\u{202E}.txt");
    std::fs::write(&odd, b"").unwrap();
    let err = f.act("play_file", json!({"path": odd})).unwrap_err();
    assert!(err.contains("two\u{FFFD}lines\u{FFFD}.txt is not a sound file"), "{err}");
    assert!(!f.notice().contains('\n') && !f.notice().contains('\u{202E}'), "{:?}", f.notice());
    let missing = f.dir.join("gone\n.mp3");
    assert!(!f.act("play_file", json!({"path": missing})).unwrap_err().contains('\n'));
    let not_folder = odd.to_string_lossy().into_owned();
    f.app.ui(move |ui| ui.invoke_scan_add_folder(not_folder.into()));
    f.quiet();
    assert!(f.notice().starts_with("Not a folder: ") && !f.notice().contains('\n'), "{:?}", f.notice());
}
