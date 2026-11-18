//! A download half written when the window closes, on a logout, and when the display goes: the
//! writers are paused at a whole chunk and the list is written, so a partial file is a prefix of
//! the body and the next start continues from it. Nothing is lost, duplicated or corrupt.

use std::time::{Duration, Instant};

use serde_json::json;
use yantrik_app::app::{ExitReason, WindowId};
use yantrik_app::cx::Inbound;
use yantrik_app::quit::Signal;

use super::{body, open, server::File, Fixture};

const SIZE: usize = 700_000;
const HELD: usize = 90_000;

/// Adds the held file and waits until some of it is on disk.
fn half_written(f: &Fixture) -> i64 {
    let id = f.act("add", json!({ "url": f.server.url("/big.bin") })).unwrap()["id"].as_i64().unwrap();
    f.until("the first part is on disk", |f| {
        let have = std::fs::metadata(f.default_dir().join("big.bin")).map(|m| m.len()).unwrap_or(0);
        have as usize >= HELD
    });
    id
}

fn on_disk(f: &Fixture) -> Vec<u8> {
    std::fs::read(f.default_dir().join("big.bin")).unwrap()
}

/// What the stopped download left: a non-empty prefix of the body, shorter than the whole.
fn assert_a_clean_partial(f: &Fixture, data: &[u8]) {
    let have = on_disk(f);
    assert!(!have.is_empty() && have.len() < data.len(), "{} of {} bytes", have.len(), data.len());
    assert_eq!(have, data[..have.len()], "what arrived is a prefix of the body");
}

/// The server lets the held file go on after the app has been asked to stop, from a thread of its
/// own: the test's thread is the UI thread, and it waits for the writers inside the job.
fn release_soon(f: &Fixture) {
    let server_gate = f.server.releaser();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        server_gate();
    });
}

#[test]
fn closing_the_window_pauses_the_writer_at_a_whole_chunk_and_then_ends() {
    let data = body(SIZE);
    let f = open(vec![("/big.bin", File::held_at(data.clone(), HELD))]);
    let id = half_written(&f);
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.windows().is_empty(), "the window is closed at once");
    assert!(f.app.lingering(), "and the app runs on without it while the writer stops");
    assert_eq!(f.app.exit_reason(), None);
    release_soon(&f);
    f.until("the app ends", |f| f.app.exit_reason().is_some());
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App));
    assert_a_clean_partial(&f, &data);
    // The list says paused, at the bytes that are on disk.
    let listed = std::fs::read_to_string(f.paths.data_dir().join("list/state.json")).unwrap();
    assert!(listed.contains("\"paused\""), "{listed}");
    assert!(!listed.contains("\"downloading\""), "{listed}");

    // The next start finds it, paused, at the same bytes; resuming continues and finishes whole.
    let have = on_disk(&f).len();
    let f = f.restart();
    assert_eq!(f.row(id)["status"], "paused");
    f.act("resume", json!({ "id": id })).unwrap();
    f.until("it completes", |f| f.row(id)["status"] == "completed");
    assert_eq!(on_disk(&f), data, "the file is whole, not restarted and not stitched wrongly");
    assert_eq!(f.server.ranges.lock().unwrap().last().cloned().flatten(), Some(have.to_string()));
}

#[test]
fn closing_with_nothing_downloading_ends_at_once() {
    let f = open(vec![("/a.bin", File::plain(body(5_000)))]);
    f.download("/a.bin");
    f.app.close_requested(WindowId::PRIMARY);
    assert_eq!(f.app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert!(!f.app.lingering());
}

#[test]
fn a_logout_by_a_signal_pauses_the_writer_and_ends_without_asking() {
    for signal in [Signal::Terminate, Signal::HangUp] {
        let data = body(SIZE);
        let f = open(vec![("/big.bin", File::held_at(data.clone(), HELD))]);
        half_written(&f);
        let post = f.app.post();
        std::thread::spawn(move || post.post(Inbound::Signal(signal)).is_ok()).join().unwrap();
        f.app.tick();
        assert_eq!(f.app.exit_reason(), None, "the writer is stopped first");
        let refused = f.act("add", json!({ "url": f.server.url("/big.bin") })).unwrap_err();
        assert_eq!(refused, crate::update::CLOSING);
        // The window coming back does not cancel a logout.
        f.app.present().unwrap();
        release_soon(&f);
        f.until("the app ends", |f| f.app.exit_reason().is_some());
        assert!(f.app.exit_reason().is_some(), "{signal:?}");
        assert_a_clean_partial(&f, &data);
    }
}

#[test]
fn the_display_going_is_a_logout_too() {
    let data = body(SIZE);
    let f = open(vec![("/big.bin", File::held_at(data.clone(), HELD))]);
    half_written(&f);
    f.app.display_gone();
    release_soon(&f);
    f.until("the app ends", |f| f.app.exit_reason().is_some());
    assert!(f.app.exit_reason().is_some());
    assert_a_clean_partial(&f, &data);
}

#[test]
fn a_logout_with_nothing_downloading_ends_at_once() {
    let f = open(vec![]);
    f.app.display_gone();
    assert!(f.app.exit_reason().is_some());
}

#[test]
fn a_second_launch_during_the_wait_cancels_a_close_but_not_a_logout() {
    let data = body(SIZE);
    let f = open(vec![("/big.bin", File::held_at(data.clone(), HELD))]);
    half_written(&f);
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.lingering());
    f.app.launch_again().unwrap();
    assert!(!f.app.windows().is_empty(), "the window is back");
    assert!(!f.app.model(|m| m.closing), "and the close is over");
    release_soon(&f);
    f.until("the writer has stopped", |f| f.row(1)["status"] == "paused");
    f.quiet();
    assert_eq!(f.app.exit_reason(), None, "the app stays, with its download paused for the person to resume");
    assert_a_clean_partial(&f, &data);
}

#[test]
fn a_writer_that_never_stops_does_not_hold_the_app_past_its_backstop() {
    let data = body(SIZE);
    let f = open(vec![("/big.bin", File::held_at(data.clone(), HELD))]);
    half_written(&f);
    f.app.close_requested(WindowId::PRIMARY);
    // The server never lets go. The desk waits for the writer in a thread of its own, and the
    // framework's linger ends the app at its backstop whatever the desk is doing.
    let started = Instant::now();
    f.app.advance(crate::update::life::CLOSE_BACKSTOP + Duration::from_secs(1));
    assert!(f.app.exit_reason().is_some() && !f.app.lingering(), "the framework ended the app at the backstop");
    assert!(started.elapsed() < Duration::from_secs(5), "the backstop is the virtual clock's, not a real wait");
    assert_a_clean_partial(&f, &data);
}
