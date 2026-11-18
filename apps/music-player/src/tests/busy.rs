//! The player's commands never wait behind the library (#937 review, finding 1). They share the
//! framework's job pool, four threads, first in first out, with the scans and the checks, so
//! the app holds its own long work to two threads at most: one scan at a time, and one wait for
//! a file to load (a newer file handed over ends the wait for the last). A pause, a close and
//! its reaping always find a thread.

use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use yantrik_app::app::{ExitReason, WindowId};
use yantrik_app::jobs::{Cancel, Deliver, Executor, Finished, JobId, Threads};
use yantrik_testing::Call;

use super::{fake, open, runtime_in, scratch, AGENT};
use crate::library;
use crate::player::{ops, Forward, Player, Rig, Start};
use crate::update::SCANNING;

/// Four agents' rescans at once: one scan runs, the others are told so at once, and the person's
/// play and pause, and a close, go ahead as usual.
#[test]
fn rescans_run_one_at_a_time_and_the_transport_and_a_close_go_ahead() {
    let Some(f) = open("rescans") else { return };
    let first = f.app.act_as(Call::new("rescan", json!({})).as_agent(AGENT)).unwrap();
    for _ in 0..3 {
        let refused = f.app.act_as(Call::new("rescan", json!({})).as_agent(AGENT)).unwrap().settle();
        assert_eq!(refused.unwrap_err(), SCANNING);
    }
    assert_eq!(f.app.jobs_waiting(), 1, "one scan, and nothing else, is running");
    assert_eq!(f.app.model(|m| m.scans.running), 1);
    let played = f.act("play", json!({"index": 1})).unwrap();
    assert_eq!(played["playing"], true);
    assert_eq!(first.settle().unwrap()["library_size"], 300, "the scan that ran is answered");
    assert_eq!(f.act("pause", json!({})).unwrap()["paused"], true);
    // A scan the person starts while one runs still waits for it, as before.
    f.app.ui(|ui| ui.invoke_scan_rescan());
    f.app.ui(|ui| ui.invoke_scan_rescan());
    assert_eq!(f.app.jobs_waiting(), 1);
    let pid = f.pid().unwrap();
    f.app.close_requested(WindowId::PRIMARY);
    f.quiet();
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App));
    assert!(fake::reaped(pid));
}

/// Where a job's label and the moment it finished go.
struct Landed(Mutex<mpsc::Sender<(&'static str, Instant)>>);

impl Deliver<&'static str> for Landed {
    fn deliver(&self, finished: Finished<&'static str>) {
        if let Finished::Done { command, .. } = finished {
            let _ = self.0.lock().unwrap().send((command, Instant::now()));
        }
    }
}

/// On the framework's own pool, with a scan holding one thread and a call's wait for a file that
/// never loads holding another: a pause lands, a newer file ends the wait, and a close's reap
/// lands, all while the scan is still walking. The scan walks until they have, so the test is
/// decided by what can run, not by how fast.
#[test]
fn on_the_real_pool_a_scan_and_a_wait_never_hold_up_the_player() {
    if !fake::python() {
        return;
    }
    let dir = scratch("real-pool");
    let rig = Rig { program: fake::install(&dir).to_string_lossy().into_owned(), runtime: Some(runtime_in(&dir)), music: None };
    let player = Player::new(&rig);
    let forward: Forward = Arc::new(|_, _| {});
    let start = Start { volume: 0.0, muted: false, forward };
    let mut pool = Threads::<&'static str>::new("music-probe", 4, 64);
    let (tx, rx) = mpsc::channel();
    let landed: Arc<dyn Deliver<&'static str>> = Arc::new(Landed(Mutex::new(tx)));
    let mut id = 0;
    let mut run = |job: Box<dyn FnOnce(&Cancel) -> &'static str + Send>| {
        id += 1;
        pool.spawn(JobId(id), Cancel::new(), job, landed.clone());
    };
    let began = Instant::now();
    // The one scan, standing in for a walk of a large library: it walks until the test has seen
    // every player job land, so a job queued behind it could never land at all.
    let (release, walking) = mpsc::channel::<()>();
    run(Box::new(move |_| {
        let _ = walking.recv_timeout(Duration::from_secs(20));
        "scan"
    }));
    let track = |name: &str| library::track_from(&dir.join(name));
    let (p, t, s, stuck) = (player.clone(), player.ticket(1), start.clone(), track("stuck one.mp3"));
    run(Box::new(move |_| {
        let loaded = ops::load(&p, t, &stuck, &s, true, false);
        assert!(matches!(loaded, ops::Loaded::Sent { answer: Some(Err(_)), .. }), "{loaded:?}");
        "wait"
    }));
    let (p, t) = (player.clone(), player.ticket(2));
    run(Box::new(move |_| {
        assert!(ops::pause(&p, t, true).unwrap().unwrap().paused);
        "pause"
    }));
    let (p, t, s, next) = (player.clone(), player.ticket(3), start.clone(), track("next one.mp3"));
    run(Box::new(move |_| {
        assert!(matches!(ops::load(&p, t, &next, &s, false, false), ops::Loaded::Sent { .. }));
        "next"
    }));
    let mut at = std::collections::HashMap::new();
    let land = |at: &mut std::collections::HashMap<&'static str, Duration>| {
        let (label, when) = rx.recv_timeout(Duration::from_secs(10)).expect("a job landed while the scan walked");
        at.insert(label, when - began);
    };
    for _ in 0..3 {
        land(&mut at);
    }
    // The close's stop and reap, asked only now, so nothing but the newer file ended the wait.
    let pid = Arc::new(Mutex::new(None));
    let (p, t, seen) = (player.clone(), player.ticket(4), pid.clone());
    run(Box::new(move |_| {
        *seen.lock().unwrap() = p.pid();
        ops::retire(&p, t);
        "retire"
    }));
    land(&mut at);
    for label in ["wait", "pause", "next", "retire"] {
        assert!(at.contains_key(label), "{label} did not land while the scan walked: {at:?}");
    }
    let pid = pid.lock().unwrap().expect("the player ran");
    assert!(fake::reaped(pid), "mpv was reaped while the scan still walked");
    release.send(()).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().0, "scan");
    let _ = std::fs::remove_dir_all(&dir);
}
