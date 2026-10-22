//! The audio stops with the app, as it always did, and no player outlives it: mpv is stopped and
//! reaped when the window closes, on a logout (a signal, the display going), and when the app is
//! killed outright.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use yantrik_app::app::{ExitReason, WindowId};
use yantrik_app::cx::Inbound;
use yantrik_app::quit::Signal;

use super::{fake, open, runtime_in, scratch, Fixture};
use crate::player::{ops, Forward, Player, Rig, Start};

/// A signal, posted from a thread of its own as the signal thread posts it, then heard.
fn signal(f: &Fixture, signal: Signal) {
    let post = f.app.post();
    std::thread::spawn(move || post.post(Inbound::Signal(signal)).is_ok()).join().expect("the posting thread");
    f.app.tick();
}

/// Playing, with its player's process id.
fn playing(f: &Fixture) -> u32 {
    f.act("play", json!({"index": 1})).unwrap();
    f.pid().expect("a player is running")
}

#[test]
fn closing_the_window_stops_the_player_reaps_it_and_then_ends() {
    let Some(f) = open("close") else { return };
    let pid = playing(&f);
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.windows().is_empty(), "the window is closed at once");
    assert!(f.app.lingering(), "and the app runs on without it while the player is stopped");
    assert_eq!(f.app.exit_reason(), None);
    let e = f.act("play", json!({})).unwrap_err();
    assert_eq!(e, crate::update::CLOSING);
    f.quiet();
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App), "the player was reaped, and the app ended itself");
    assert!(fake::reaped(pid), "no mpv is left behind, not even a zombie");
}

/// With nothing playing and nothing on its way to a player, a close ends the app at once.
#[test]
fn closing_the_window_with_no_player_ends_at_once() {
    let Some(f) = open("close-idle") else { return };
    f.app.close_requested(WindowId::PRIMARY);
    assert_eq!(f.app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert!(!f.app.lingering());
}

/// A play pressed just before the close: the player it starts is stopped after it, in turn.
#[test]
fn a_close_with_a_play_still_on_its_way_stops_the_player_that_play_starts() {
    let Some(f) = open("close-queued") else { return };
    f.app.ui(|ui| ui.invoke_play_track_index(0));
    assert_eq!(f.app.model(|m| m.player_jobs), 1);
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.lingering());
    f.app.run_one_job();
    let pid = f.pid().expect("the play started a player");
    f.quiet();
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App));
    assert!(fake::reaped(pid));
}

/// A second launch during the close wait opens the window again and cancels the close; the next
/// play starts a new player.
#[test]
fn a_window_presented_during_the_close_wait_cancels_the_close() {
    let Some(f) = open("close-cancelled") else { return };
    let pid = playing(&f);
    f.app.close_requested(WindowId::PRIMARY);
    f.app.present().unwrap();
    assert_eq!(f.app.windows(), vec![WindowId::PRIMARY]);
    f.quiet();
    assert_eq!(f.app.exit_reason(), None, "the app keeps running");
    assert!(fake::reaped(pid), "the player the person closed is still stopped");
    assert_eq!(f.state()["player"], "not started");
    let again = f.act("play", json!({"index": 2})).unwrap();
    assert_eq!(again["playing"], true, "{again}");
    f.app.advance(crate::update::CLOSE_BACKSTOP + Duration::from_secs(1));
    assert_eq!(f.app.exit_reason(), None, "and runs on past the backstop");
}

/// A second launch with a file during the close wait: the window comes back and the file plays.
#[test]
fn a_file_handed_over_during_the_close_wait_cancels_the_close_and_plays() {
    let Some(f) = open("close-handover") else { return };
    let pid = playing(&f);
    f.app.close_requested(WindowId::PRIMARY);
    let profile = f.app.paths().profile().to_string();
    let launch = yantrik_app::launch::Launch::of_files(&profile, &f.dir, [f.track("Gamma", 1)]);
    // As the `launch` action applies it: the launch, then the window shown.
    f.app.event(yantrik_app::app::Event::Launch(launch));
    f.app.present().unwrap();
    let want = f.track("Gamma", 1).to_string_lossy().into_owned();
    f.until("the handed-over file plays", |f| f.playing_path().as_deref() == Some(want.as_str()));
    assert_eq!(f.app.exit_reason(), None);
    assert!(fake::reaped(pid), "the player the person closed was stopped first");
    assert_eq!(f.app.windows(), vec![WindowId::PRIMARY]);
}

#[test]
fn a_logout_stops_the_player_reaps_it_and_ends() {
    let Some(f) = open("logout") else { return };
    let pid = playing(&f);
    signal(&f, Signal::Terminate);
    assert_eq!(f.app.exit_reason(), None, "the player is stopped first");
    assert_eq!(f.act("next", json!({})).unwrap_err(), crate::update::CLOSING);
    // The window coming back does not cancel a logout.
    f.app.present().unwrap();
    f.quiet();
    assert!(f.app.exit_reason().is_some(), "then the app ends");
    assert!(fake::reaped(pid));
}

#[test]
fn the_display_going_stops_the_player_too() {
    let Some(f) = open("display-gone") else { return };
    let pid = playing(&f);
    f.app.display_gone();
    f.quiet();
    assert!(f.app.exit_reason().is_some());
    assert!(fake::reaped(pid));
    let _ = f.app.shutdown();
}

#[test]
fn a_logout_with_no_player_ends_at_once() {
    let Some(f) = open("logout-idle") else { return };
    signal(&f, Signal::HangUp);
    assert!(f.app.exit_reason().is_some());
}

/// Where the killed app's child writes its player's process id.
const PID_FILE: &str = "YANTRIK_MUSIC_TEST_PID_FILE";

/// The app killed outright (SIGKILL, nothing of it runs again): the kernel stops its player
/// (`PR_SET_PDEATHSIG`), and nothing is left. The app is a copy of this test binary, running
/// [`killed_app`] alone.
#[test]
fn a_player_ends_with_an_app_that_is_killed() {
    if !fake::python() {
        return;
    }
    let dir = scratch("killed");
    let pid_file = dir.join("mpv.pid");
    let mut app = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::closing::killed_app", "--ignored", "--nocapture", "--test-threads=1"])
        .env(PID_FILE, &pid_file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pid_file).ok().and_then(|t| t.trim().parse::<u32>().ok()) {
            break pid;
        }
        assert!(Instant::now() < deadline, "the app never started its player");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!fake::reaped(pid), "the player runs while the app does");
    app.kill().unwrap();
    app.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake::reaped(pid) {
        assert!(Instant::now() < deadline, "mpv {pid} outlived the app that was killed");
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The app [`a_player_ends_with_an_app_that_is_killed`] kills: it starts a player from a thread
/// that lives as long as it does, as the job pool's threads do, and waits to be killed.
#[test]
#[ignore = "run by a_player_ends_with_an_app_that_is_killed, as the app it kills"]
fn killed_app() {
    let Some(pid_file) = std::env::var_os(PID_FILE) else { return };
    let dir = scratch("killed-app");
    let rig = Rig { program: fake::install(&dir).to_string_lossy().into_owned(), runtime: Some(runtime_in(&dir)), music: None };
    let player = Player::new(&rig);
    let forward: Forward = Arc::new(|_, _| {});
    let start = Start { volume: 0.0, muted: false, forward };
    let track = crate::library::track_from(&dir.join("a.mp3"));
    let loaded = ops::load(&player, player.ticket(1), &track, &start, false, false);
    assert!(matches!(loaded, ops::Loaded::Sent { .. }), "{loaded:?}");
    std::fs::write(pid_file, player.pid().unwrap().to_string()).unwrap();
    std::thread::sleep(Duration::from_secs(60));
}
