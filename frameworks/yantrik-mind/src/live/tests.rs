//! Tests for `live.rs`: the reach probe and the at-rest/backoff interval functions.

use super::*;

fn rpc_error(code: i32) -> yantrik_ipc_transport::RpcError {
    yantrik_ipc_transport::RpcError {
        code,
        message: "no".into(),
        data: None,
    }
}

/// The core of the defect: a no-model refusal used to arrive as an ordinary string, and
/// every app showed it — or something wrapped around it — as the answer.
#[test]
fn the_no_model_code_maps_to_the_one_sentence() {
    let e = ask_error(yantrik_ipc_contracts::ERR_NO_MODEL, "no AI model is set up");
    assert!(matches!(e, AskError::NoModel));
    assert_eq!(
        e.to_string(),
        "No AI model answered. Check Settings \u{2192} AI."
    );
    assert_eq!(e.hint(), NO_MODEL_HINT);
}

#[test]
fn any_other_failure_keeps_its_reason_and_the_old_hint() {
    let e = ask_error(-32000, "companion timed out");
    assert_eq!(
        e.to_string(),
        "The companion did not answer: [-32000] companion timed out"
    );
    assert_eq!(e.hint(), OFFLINE_HINT);
}

/// A shell that answers "not online" is running: the hint must not send the person to
/// start it. That wrong sentence is what every AI control showed after the first offline
/// reply flipped the flag.
#[test]
fn reach_tells_a_missing_shell_apart_from_a_shell_with_no_model() {
    assert_eq!(
        reach_of(Ok(serde_json::json!({ "online": true }))),
        Reach::Ready
    );
    assert_eq!(
        reach_of(Ok(serde_json::json!({ "online": false }))),
        Reach::NoModel
    );
    assert_eq!(reach_of(Err(rpc_error(-1))), Reach::NoShell);

    assert_eq!(Reach::Ready.hint(), None);
    assert_eq!(Reach::NoModel.hint(), Some(NO_MODEL_HINT));
    assert_eq!(Reach::NoShell.hint(), Some(OFFLINE_HINT));
}

/// A probe that takes `ms` and then answers `reach`, counting how often it was asked.
fn slow_probe(
    ms: u64,
    reach: Reach,
    asked: Arc<std::sync::atomic::AtomicUsize>,
) -> Box<dyn Fn() -> Reach + Send + Sync> {
    Box::new(move || {
        asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(ms));
        reach
    })
}

/// The defect: the rail asked the shell on the window's thread, and a describe queued behind
/// the round trip. Asking now costs a bounded first wait and nothing after it, however slow
/// the shell is.
#[test]
fn asking_does_not_wait_on_a_slow_shell() {
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let watch = ReachWatch::new(
        slow_probe(1500, Reach::Ready, asked.clone()),
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_secs(30),
    );

    let first = Instant::now();
    assert_eq!(watch.get(), Reach::NoShell, "nothing heard yet");
    assert!(
        first.elapsed() < Duration::from_millis(600),
        "the first ask waited {:?}",
        first.elapsed()
    );

    // While the probe is still out, every ask is a read.
    for _ in 0..20 {
        let t = Instant::now();
        assert_eq!(watch.get(), Reach::NoShell);
        assert!(
            t.elapsed() < Duration::from_millis(50),
            "an ask waited {:?}",
            t.elapsed()
        );
    }
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one probe, not one per ask"
    );

    // And once the shell answers, the answer is what the next ask reads.
    let deadline = Instant::now() + Duration::from_secs(10);
    while watch.get() != Reach::Ready {
        assert!(Instant::now() < deadline, "the probe's answer never landed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A shell that is there answers inside the first wait, so a rail filled at startup says the
/// right thing rather than "not connected" for a tick.
#[test]
fn a_quick_shell_is_read_on_the_first_ask() {
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let watch = ReachWatch::new(
        slow_probe(0, Reach::NoModel, asked),
        Duration::from_millis(50),
        Duration::from_secs(5),
        Duration::from_secs(30),
    );
    assert_eq!(watch.get(), Reach::NoModel);
}

/// Nobody asking, nobody polling: the watcher stops, and the next ask starts it again.
#[test]
fn the_watcher_stops_when_nobody_asks_and_starts_again_when_somebody_does() {
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let watch = ReachWatch::new(
        slow_probe(0, Reach::Ready, asked.clone()),
        Duration::from_millis(10),
        Duration::from_secs(5),
        Duration::from_millis(60),
    );
    assert_eq!(watch.get(), Reach::Ready);

    let deadline = Instant::now() + Duration::from_secs(10);
    while watch.state.lock().unwrap().running {
        assert!(Instant::now() < deadline, "the watcher never went idle");
        std::thread::sleep(Duration::from_millis(10));
    }
    let idle_at = asked.load(std::sync::atomic::Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        idle_at,
        "an idle watcher asked the shell"
    );

    assert_eq!(watch.get(), Reach::Ready);
    assert!(
        watch.state.lock().unwrap().running,
        "an ask wakes the watcher"
    );
}

/// The defect: the watcher probed the shell on a socket round trip every `every` even
/// when the answer kept arriving the same (#983). A probe that always answers the same
/// is then followed for a window bounded by `idle` (not by the clock: the watcher stops
/// once nobody asks), and at most half the probes the fixed-interval watcher of before
/// could fit in the same window — a margin, not a hair, so a slow machine cannot
/// schedule its way through it either way.
#[test]
fn an_unchanging_answer_is_not_reprobed_every_tick() {
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let every = Duration::from_millis(50);
    let idle = Duration::from_millis(800);
    let watch = ReachWatch::new(
        slow_probe(0, Reach::Ready, asked.clone()),
        every,
        Duration::from_secs(5),
        idle,
    );
    watch.get();

    // The watcher stops once `idle` passes without an ask, so the probe count below is
    // bounded by `idle` and needs no stopwatch of its own.
    let deadline = Instant::now() + Duration::from_secs(10);
    while watch.state.lock().unwrap().running {
        assert!(Instant::now() < deadline, "the watcher never went idle");
        std::thread::sleep(Duration::from_millis(10));
    }
    let count = asked.load(std::sync::atomic::Ordering::SeqCst);
    // The old fixed cadence fitted at least idle/every probes into the same window;
    // from the second one on, the backoff makes that cadence impossible. The margin
    // (a factor, not a hair) is what keeps this honest on a slow CI box.
    let fixed = (idle.as_millis() / every.as_millis()) as usize;
    assert!(
        count * 2 <= fixed,
        "{count} probes in a window the fixed watcher filled with at least {fixed}"
    );
}

/// The interval rule as a number, with no thread and no clock: a repeated answer backs
/// the next probe off by the factor, a changing one goes straight back to `every`.
#[test]
fn the_next_probe_waits_every_while_answers_change_and_the_backoff_while_they_repeat() {
    assert_eq!(next_probe_interval(REACH_EVERY, false), REACH_EVERY);
    assert_eq!(
        next_probe_interval(REACH_EVERY, true),
        REACH_EVERY.saturating_mul(REACH_BACKOFF_FACTOR)
    );
    assert_eq!(
        next_probe_interval(REACH_EVERY, true),
        Duration::from_secs(10),
        "two seconds backs off to ten"
    );
}
