//! Answers per invocation, and commands that reach mpv in the order they were asked: what a job
//! answers is about its own work, what lands late is never drawn over what is newer, and a
//! refused command leaves the model as it was.

use std::sync::Arc;

use serde_json::json;
use yantrik_app::app::Changed;
use yantrik_app::effects::Effects;
use yantrik_app::App as _;

use super::{fake, open, runtime_in, scratch};
use crate::command::{Command, Scanned};
use crate::kept::Kept;
use crate::library::{self, Library, Track};
use crate::model::{Model, PART_STATUS, PART_TRANSPORT};
use crate::mpv::Event;
use crate::player::ops::{self, Loaded};
use crate::player::{Forward, Player, Rig, Start};
use crate::Music;

/// The player started a file for a call, and was handed another before it said the first was
/// playing: the call is answered about its own file, and told where the player has gone since.
#[test]
fn a_load_is_answered_about_its_own_track_when_another_is_asked_for_meanwhile() {
    if !fake::python() {
        return;
    }
    let dir = scratch("moved-on");
    let rig = Rig { program: fake::install(&dir).to_string_lossy().into_owned(), runtime: Some(runtime_in(&dir)), music: None };
    let player = Player::new(&rig);
    let forward: Forward = Arc::new(|_, _| {});
    let start = Start { volume: 50.0, muted: false, forward };
    let (slow, then) = (dir.join("slow one.mp3"), dir.join("then this.mp3"));
    let (first, second) = (library::track_from(&slow), library::track_from(&then));
    let (p1, t1, s1) = (player.clone(), player.ticket(1), start.clone());
    let asked = std::thread::spawn(move || ops::load(&p1, t1, &first, &s1, true, false));
    let (p2, t2) = (player.clone(), player.ticket(2));
    let other = std::thread::spawn(move || ops::load(&p2, t2, &second, &start, false, false));
    let Loaded::Sent { answer: Some(Ok(played)), .. } = asked.join().unwrap() else { panic!("the first load was not played") };
    other.join().unwrap();
    assert_eq!(played.report.path.as_deref(), Some(slow.to_str().unwrap()), "the report is the asked-for file's");
    assert_eq!(played.moved_to.as_ref().map(|(p, _)| p.as_str()), Some(then.to_str().unwrap()));
    drop(player);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Several commands from the person and the Mind, landing in the order they were asked.
#[test]
fn the_players_commands_land_in_the_order_they_were_asked() {
    let Some(f) = open("order") else { return };
    let pending = f.app.act("play", json!({"index": 3})).unwrap();
    f.app.ui(|ui| ui.invoke_play_track_index(9));
    f.app.ui(|ui| ui.invoke_volume_changed(0.2));
    f.app.ui(|ui| ui.invoke_volume_changed(0.7));
    // The play, the row, and the first volume step; the second waits for the first to land.
    assert_eq!(f.app.model(|m| m.player_jobs), 3);
    let answer = f.settled(pending).unwrap();
    assert_eq!(answer["track"], "Alpha 002", "the Mind is told about its own play: {answer}");
    f.until("the person's row plays", |f| f.playing_path().is_some_and(|p| p.ends_with("Alpha 009.mp3")));
    f.until("the last volume is mpv's", |f| f.state()["volume"].as_f64().is_some_and(|v| (v - 70.0).abs() < 0.001));
    assert_eq!(f.app.model(|m| m.player_jobs), 0);
}

fn model() -> Model {
    let rig = Rig { program: "/nonexistent".into(), runtime: None, music: None };
    Model::new(&rig, &Kept { folders: Some(vec!["/m".into()]), volume: 50.0 }, "/c/music.json".into())
}

fn scanned(scan: u64, titles: &[&str]) -> Command {
    let tracks: Vec<Track> = titles.iter().map(|t| library::track_from(std::path::Path::new(&format!("/m/a/{t}.mp3")))).collect();
    let found = Library { per_folder: vec![tracks.len()], tracks, unreadable: Vec::new() };
    Command::Scanned(Box::new(Scanned { scan, folders: vec!["/m".into()], found, for_agents: vec![("/m".into(), false)] }))
}

#[test]
fn a_scan_that_lands_after_a_later_one_changes_nothing_on_screen() {
    let mut m = model();
    m.scans.started = 2;
    m.scans.running = 2;
    Music::update(&mut m, scanned(2, &["new", "newer"]), &mut Effects::recording()).unwrap();
    Music::update(&mut m, scanned(1, &["old"]), &mut Effects::recording()).unwrap();
    let titles: Vec<String> = m.library.iter().map(|t| t.title.clone()).collect();
    assert_eq!(titles, ["new", "newer"], "the later scan is what is shown");
    assert_eq!(m.status, "2 tracks in 1 folder");
    assert_eq!(m.scans.running, 0);
}

#[test]
fn the_position_redraws_the_transport_alone_and_a_scans_progress_the_status_line_alone() {
    let mut m = model();
    let mut fx = Effects::<Music>::recording();
    Music::update(&mut m, Command::Mpv { player: 1, event: Event::Property("time-pos".into(), json!(12.0)) }, &mut fx).unwrap();
    assert_eq!(*fx.changed_parts(), Changed::Parts(vec![PART_TRANSPORT]));
    assert_eq!(m.observed.time_pos, Some(12.0));
    m.scans.started = 1;
    m.scans.running = 1;
    let mut fx = Effects::<Music>::recording();
    Music::update(&mut m, Command::Scanning { scan: 1, found: 100 }, &mut fx).unwrap();
    assert_eq!(*fx.changed_parts(), Changed::Parts(vec![PART_STATUS]));
    assert_eq!(m.status, "Scanning\u{2026} 100 tracks so far");
}

#[test]
fn what_a_stopped_player_says_is_not_heard() {
    let mut m = model();
    Music::update(&mut m, Command::Mpv { player: 2, event: Event::Property("volume".into(), json!(33.0)) }, &mut Effects::recording()).unwrap();
    assert_eq!(m.volume, 33.0);
    Music::update(&mut m, Command::Retired(2), &mut Effects::recording()).unwrap();
    for player in [1, 2] {
        Music::update(&mut m, Command::Mpv { player, event: Event::Gone }, &mut Effects::recording()).unwrap();
    }
    assert!(m.notice.is_empty(), "the player stopped on purpose is not reported dead: {}", m.notice);
    assert_eq!(m.state.word(), "not started");
    Music::update(&mut m, Command::Mpv { player: 3, event: Event::Gone }, &mut Effects::recording()).unwrap();
    assert_eq!(m.state.word(), "stopped", "a new player's death is");
}

#[test]
fn a_command_refused_while_closing_leaves_the_model_as_it_was() {
    let mut m = model();
    m.closing = true;
    let (tickets, jobs) = (m.tickets, m.player_jobs);
    let mut fx = Effects::<Music>::recording();
    let refused = Music::update(&mut m, Command::Play(None), &mut fx).unwrap_err();
    assert_eq!(refused.sentence, crate::update::CLOSING);
    assert_eq!((m.tickets, m.player_jobs), (tickets, jobs));
    assert!(fx.requests().is_empty(), "and asks for nothing");
    // What mpv says while the player is being stopped starts nothing either.
    m.queue.start(vec![library::track_from(std::path::Path::new("/m/a/x.mp3"))], 0);
    let eof = Event::EndFile { reason: "eof".into(), error: None };
    let mut fx = Effects::<Music>::recording();
    Music::update(&mut m, Command::Mpv { player: 1, event: eof }, &mut fx).unwrap();
    assert!(fx.requests().is_empty());
}

/// A scan that ends without a result leaves no follow-up owed: the person is told to press
/// Rescan, and that press scans once.
#[test]
fn a_scan_that_fails_owes_no_follow_up() {
    let mut m = model();
    m.scans.started = 1;
    m.scans.running = 1;
    m.scans.again = true;
    Music::update(&mut m, Command::WorkFailed(crate::command::Work::Scan), &mut Effects::recording()).unwrap();
    assert_eq!((m.scans.running, m.scans.again), (0, false));
}

/// The person's volume job that did not run lets the newest waiting value go, rather than drop it.
#[test]
fn a_volume_change_that_did_not_run_sends_the_one_waiting() {
    let mut m = model();
    m.volume_sending = true;
    m.volume_next = Some(55.0);
    m.player_jobs = 1;
    let mut fx = Effects::<Music>::recording();
    Music::update(&mut m, Command::WorkFailed(crate::command::Work::Volume), &mut fx).unwrap();
    assert!(fx.requests().iter().any(|r| r.name() == "spawn"), "the 55 is sent");
    assert_eq!((m.volume_sending, m.volume_next), (true, None));
}
