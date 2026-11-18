//! A shell that prints without pause: one command a frame reaches the UI thread, memory is the
//! scrollback and nothing else, and the person can still interrupt it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use yantrik_app::cx::Inbound;
use yantrik_app::queue;

use super::open;
use crate::hub::Hub;
use crate::session::HISTORY_LINES;
use crate::Terminal;

/// However many times, from however many threads, the shells call the notifier, one command is
/// queued until the frame after it has been taken.
#[test]
fn a_flood_of_wakes_queues_one_command_until_the_frame_is_taken() {
    let (post, inbox) = queue::bounded::<Inbound<Terminal>>(queue::CAPACITY, Arc::new(|| {}));
    let hub = Hub::default();
    hub.connect(post);
    let notify = hub.notifier();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let notify = notify.clone();
            std::thread::spawn(move || (0..20_000).for_each(|_| notify()))
        })
        .collect();
    threads.into_iter().for_each(|t| t.join().unwrap());
    assert_eq!(inbox.len(), 1, "eighty thousand wakes, one command");
    hub.release();
    notify();
    notify();
    assert_eq!(inbox.len(), 2, "and the next frame's output posts again");
}

#[test]
fn a_wake_with_nowhere_to_go_is_tried_again_next_time() {
    let hub = Hub::default();
    let notify = hub.notifier();
    notify();
    let (post, inbox) = queue::bounded::<Inbound<Terminal>>(queue::CAPACITY, Arc::new(|| {}));
    hub.connect(post);
    notify();
    assert_eq!(inbox.len(), 1, "the first wake, with no door, did not use up the flag");
}

/// `yes` for a while: the history stays at its cap, the window stays alive, and a control
/// character from a mind stops it and gets the prompt back.
#[test]
fn yes_cannot_starve_the_window_or_grow_the_history_and_can_be_interrupted() {
    let f = open();
    f.app.run(crate::Command::Input(b"yes\r".to_vec()));
    let started = Instant::now();
    let mut slowest = Duration::ZERO;
    let revision_before = f.app.model(|m| m.session().map(|s| s.revision()).unwrap_or(0));
    while started.elapsed() < Duration::from_millis(1500) {
        let step = Instant::now();
        f.step();
        slowest = slowest.max(step.elapsed());
        std::thread::sleep(Duration::from_millis(5));
    }
    let revisions = f.app.model(|m| m.session().map(|s| s.revision()).unwrap_or(0)) - revision_before;
    assert!(revisions > 100, "the flood really ran: {revisions} revisions");
    assert!(slowest < Duration::from_secs(1), "the UI thread was held for {slowest:?} by a step");

    let (lines, rows) = f.app.model(|m| {
        let s = m.session().unwrap();
        (s.history().len(), s.geometry().1 .0 as usize)
    });
    assert!(lines <= HISTORY_LINES + rows, "{lines} lines kept of an endless stream");
    assert!(f.app.model(|m| m.dying.len() + m.tabs.len()) <= 2);

    // The mind interrupts it while it is still flooding, and is answered.
    let answer = f.act("send_input", json!({ "text": "\u{3}" })).unwrap();
    assert_eq!(answer["sent_bytes"], json!(1));
    f.until("the prompt back", |f| {
        let screen = f.screen();
        screen.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| l.trim_end().ends_with(super::PROMPT))
    });
    assert!(!f.app.model(|m| m.session().is_some_and(|s| s.has_children())));
}
