//! A working machine, measured as a display would draw it: every two-second sample says
//! something new (the CPU, each core, memory, the network's rates, the busiest processes), as on
//! VM 520 at 37% CPU, where System Monitor in front cost about 3,350 ticks a minute (#944's 520
//! evidence). Rest is measured in `rest_tests.rs`; this is the window at work.

use std::time::Duration;

use yantrik_app::app::{Event, WindowId};
use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_ipc_contracts::system_monitor::ProcessInfo;
use yantrik_testing::{AtRest, TestApp};

use crate::app_tests::start;
use crate::machine::fake::{self, process, FakeMachine};
use crate::outcome::Provenance;
use crate::sample::Reading;
use crate::SystemMonitor;

const ONE_MINUTE: Duration = Duration::from_secs(60);
/// Fine enough that a landing and anything 300 ms after it are drawn as separate frames.
const STEP: Duration = Duration::from_millis(100);
/// More readings than any test here takes; past them the fake reads the last again.
const READINGS: usize = 200;

/// The `i`th reading of a busy machine: every figure on screen moves from the last.
fn busy(i: usize) -> Reading {
    let mut snap = fake::snapshot(Some(2000));
    let step = |every: usize, span: usize| ((i * every) % span) as f64;
    snap.cpu.overall_percent = 30.0 + step(7, 23);
    for core in &mut snap.cpu.cores {
        core.usage_percent = 15.0 + step(11 + core.id as usize * 3, 61);
    }
    snap.cpu.load_avg_1 = 0.9 + step(3, 7) / 100.0;
    snap.memory.usage_percent = 48.0 + step(3, 5);
    snap.memory.used_bytes += (i as u64 % 5) * 64 * 1024 * 1024;
    snap.networks[0].rx_rate_bps = 2048 * (1 + (i as u64 * 5) % 9);
    snap.networks[0].tx_rate_bps = 512 * (1 + (i as u64 * 3) % 7);
    let procs: Vec<ProcessInfo> = vec![
        process(4242, "chromium", 20.0 + step(13, 40), 12.36),
        process(4300, "llvmpipe-0", 5.0 + step(5, 9), 0.5),
        process(77, "pi", 1.0 + step(2, 3), 0.2),
    ];
    fake::reading(snap, procs, Provenance::service())
}

/// System Monitor in front at the size it opens at, with the rail on screen and the Mind
/// reachable, reading a busy machine.
fn at_work() -> TestApp<SystemMonitor> {
    let machine = (0..READINGS).fold(FakeMachine::new(), |machine, i| machine.reads(busy(i)));
    let (app, _) = start(machine);
    app.run_jobs();
    app.seen(Seen::Reach(Reach::Ready));
    app.resize(1200, 800);
    app.advance(Duration::from_secs(1));
    app.run_jobs();
    app
}

#[test]
fn a_busy_machine_in_front_draws_one_frame_a_reading() {
    // Before: 1,260 frames a minute here, 42 for each reading: the rings and bars eased to every
    // sample for 300 ms, and the rail redrew its figures 300 ms after the cards did.
    let app = at_work();
    let work = app.on_screen(ONE_MINUTE, STEP);
    eprintln!("busy, on screen, one minute: {work:?}");
    assert_eq!(work, AtRest { frames: 30, fired: 60, jobs: 30 }, "a reading, a frame");
}

#[test]
fn off_screen_the_machine_is_read_twice_a_minute_and_nothing_is_drawn() {
    let app = at_work();
    app.on_screen(Duration::from_secs(10), STEP);
    // Minimised: the compositor lets it draw nothing. The reading taken as it went is never
    // drawn, so the next poll finds it off screen; from then on one reading every 30 s, for
    // `describe` (at 30, 60 and 90 s; the next is due as the span ends), and the polls between
    // read nothing and arm nothing: 60 polls and the rail's one look after each reading.
    let away = app.hidden(2 * ONE_MINUTE, STEP);
    eprintln!("busy, hidden, two minutes: {away:?}");
    assert_eq!(away, AtRest { frames: 0, fired: 64, jobs: 4 });
}

#[test]
fn a_window_brought_back_reads_at_once_and_every_two_seconds_again() {
    let app = at_work();
    app.on_screen(Duration::from_secs(10), STEP);
    app.hidden(ONE_MINUTE, STEP);
    app.event(Event::Activated(WindowId::PRIMARY));
    assert_eq!(app.jobs_waiting(), 1, "the numbers on screen may be 30 s old: read now");
    let back = app.on_screen(Duration::from_secs(10), STEP);
    eprintln!("busy, back, ten seconds: {back:?}");
    assert_eq!(back.frames, back.jobs, "{back:?}");
    assert_eq!(back.jobs, 6, "the reading asked for on the way back, and one every 2 s");
}

#[test]
fn a_window_drawn_again_without_being_activated_reads_again() {
    // Uncovered, not clicked: the compositor's next frame is the word that it is on screen.
    let app = at_work();
    app.on_screen(Duration::from_secs(10), STEP);
    app.hidden(ONE_MINUTE, STEP);
    let back = app.on_screen(Duration::from_secs(10), STEP);
    eprintln!("busy, uncovered, ten seconds: {back:?}");
    assert_eq!(back.jobs, 5, "{back:?}");
    assert_eq!(back.frames, back.jobs);
}
