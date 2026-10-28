//! The queue: how many pictures are made at once, cancelling (a server is told to stop rendering),
//! and what a close, a quit and a logout do with work in flight.

use std::time::Duration;

use serde_json::json;
use yantrik_app::app::{ExitReason, WindowId};

use super::server::{Manners, Server};
use super::{apply, apply_as_agent, bare, fake, started};
use crate::command::{Command, Which};
use crate::model::RENDERING_AT_ONCE;
use crate::plan::Plan;

fn running_and_pending(f: &super::Fixture) -> (usize, usize) {
    let queue = &f.state()["queue"];
    (queue["running"].as_array().unwrap().len(), queue["pending"].as_array().unwrap().len())
}

#[test]
fn only_so_many_pictures_are_made_at_once_and_the_rest_wait_their_turn() {
    let f = fake("turns");
    let asked = RENDERING_AT_ONCE + 2;
    for n in 0..asked {
        f.ask("generate", json!({ "prompt": format!("harbour {n}"), "width": 64, "height": 64 })).unwrap();
    }
    // Nothing has run: the first few have a worker each, and the others are `pending` in describe.
    assert_eq!(running_and_pending(&f), (RENDERING_AT_ONCE, 2));
    assert!(f.summary().contains("2 being made, 2 waiting"), "{}", f.summary());
    assert!(f.app.ui(|ui| ui.get_status().to_string()).contains("harbour 0"));

    assert_eq!(f.settled(), "");
    assert_eq!(f.state()["gallery"]["count"], json!(asked));
    assert_eq!(running_and_pending(&f), (0, 0));
}

/// Cancelling is the Mind's and the person's, and it reaches a job that has not begun at once.
#[test]
fn cancel_all_stops_every_job_including_the_ones_not_started() {
    let f = fake("cancel-all");
    let asked = RENDERING_AT_ONCE + 1;
    for n in 0..asked {
        f.ask("generate", json!({ "prompt": format!("long render {n}"), "width": 64, "height": 64 })).unwrap();
    }
    let answer = f.ask("cancel_all", json!({})).unwrap();
    assert_eq!(answer["cancelled"], json!(format!("asked {asked} jobs to stop")));
    // "cancelling" rather than gone, because the workers are still in the middle of something; the
    // one that had not begun is gone, and says why.
    let queue = f.state()["queue"].clone();
    let listed: Vec<_> = queue["running"].as_array().unwrap().iter().chain(queue["pending"].as_array().unwrap()).collect();
    assert_eq!(listed.len(), RENDERING_AT_ONCE, "{queue}");
    assert!(listed.iter().all(|each| each["status"] == json!("cancelling")), "{queue}");
    assert!(f.notice().contains("Cancelled before anything was made"), "{}", f.notice());

    assert!(f.settled().contains("Cancelled"), "{}", f.summary());
    assert_eq!(f.state()["gallery"]["count"], json!(0), "a cancelled job still made a picture");
    assert!(!f.gallery().exists() || crate::gallery::listing(&f.gallery(), 12, false).is_empty());
}

#[test]
fn cancelling_one_job_leaves_the_others_to_finish() {
    let f = fake("cancel-one");
    let mut ids = Vec::new();
    for n in 0..RENDERING_AT_ONCE + 1 {
        let answer = f.ask("generate", json!({ "prompt": format!("harbour {n}"), "width": 64, "height": 64 })).unwrap();
        ids.push(answer["job"].as_i64().unwrap());
    }
    // The last one has not begun.
    let answer = f.ask("cancel", json!({ "job": ids[RENDERING_AT_ONCE] })).unwrap();
    assert_eq!(answer["cancelled"], json!(format!("asked job {} to stop", ids[RENDERING_AT_ONCE])));
    assert_eq!(running_and_pending(&f), (RENDERING_AT_ONCE, 0));
    f.settled();
    assert_eq!(f.state()["gallery"]["count"], json!(RENDERING_AT_ONCE));
}

#[test]
fn a_job_stopped_between_pictures_keeps_what_it_had_written() {
    let (_world, mut m) = bare("between", r#"{"backend":{"kind":"fake"}}"#);
    apply(&mut m, Command::Generate(Some(Box::new(Plan { count: 4, ..Plan::new("four harbours") }))));
    let id = m.queue[0].job.id;
    // One picture landed; the job is between pictures.
    m.queue[0].job.made = 1;
    m.queue[0].flying = false;
    apply(&mut m, Command::Cancel(Which::One(id)));
    // Whatever landed before the flag was seen stays: a cancellation is not a deletion, and the
    // notice says how many there were and that it cannot be resumed.
    assert!(m.queue.is_empty());
    assert!(
        m.notice.starts_with("Cancelled. 1 picture had already been written to the gallery and has been left there;"),
        "{}",
        m.notice
    );
    assert!(m.notice.contains("cannot be resumed"), "{}", m.notice);

    // And for several: plural, with the plural verb.
    let (_world, mut m) = bare("between-three", r#"{"backend":{"kind":"fake"}}"#);
    apply(&mut m, Command::Generate(Some(Box::new(Plan { count: 4, ..Plan::new("four harbours") }))));
    let id = m.queue[0].job.id;
    m.queue[0].job.made = 3;
    m.queue[0].flying = false;
    apply(&mut m, Command::Cancel(Which::One(id)));
    assert!(
        m.notice.starts_with("Cancelled. 3 pictures had already been written to the gallery and have been left there;"),
        "{}",
        m.notice
    );
}

/// The cancel command stops the job the picture in flight belongs to, answers from its own id, and
/// says what it did.
#[test]
fn cancel_sets_the_jobs_flag_and_answers_from_its_own_job() {
    let (_world, mut m) = bare("cancel-flag", r#"{"backend":{"kind":"fake"}}"#);
    apply(&mut m, Command::Generate(Some(Box::new(Plan::new("one")))));
    apply(&mut m, Command::Generate(Some(Box::new(Plan::new("two")))));
    let (first, second) = (m.queue[0].job.id, m.queue[1].job.id);
    let fx = apply_as_agent(&mut m, Command::Cancel(Which::One(second)));
    assert!(m.entry(second).unwrap().stop.is_set());
    assert_eq!(m.entry(second).unwrap().job.status, "cancelling");
    assert!(!m.entry(first).unwrap().stop.is_set(), "a job nobody named was stopped");
    assert_eq!(m.answer, json!({ "cancelled": format!("asked job {second} to stop") }));
    assert!(fx.requests().iter().any(|r| r.name() == "settle"));
}

/// A render nobody is waiting for is still burning a GPU somebody owns, so the server is told.
#[test]
fn stopping_a_render_in_flight_tells_the_server_to_stop() {
    let server = Server::start(Manners { never_finish: true, ..Manners::default() });
    let f = started("interrupt", &server.config("comfyui"));
    let pending =
        f.app.act("generate", json!({ "prompt": "a long render", "count": 2, "width": 64, "height": 64 })).unwrap();
    // The answer was settled by the command itself; the worker has not run yet.
    let answer = pending.settle().unwrap();
    let job = answer["job"].as_i64().unwrap();

    // The queue is how a caller watches work it cannot wait for.
    let running = f.state()["queue"]["running"].as_array().unwrap().clone();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0]["id"], json!(job));
    assert_eq!(running[0]["kind"], json!("generate"));
    assert_eq!(running[0]["label"], json!("2 pictures of \u{201c}a long render\u{201d}"));
    assert_eq!(running[0]["progress"], json!("0 of 2"));
    assert!(f.summary().contains("1 picture being made"), "{}", f.summary());

    // The flag the cancel command sets, set from another thread while the render is on its
    // worker: the server never finishes, so this is the only way out.
    let stop = f.app.model(|m| m.queue[0].stop.clone());
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        stop.set();
    });
    f.app.run_jobs();
    stopper.join().unwrap();

    assert_eq!(f.notice(), "Cancelled before anything was made.");
    assert!(server.with(|recorded| recorded.interrupts) >= 1, "the server was never told to stop");
    // Nothing was written, so there is no half-file to clean up, which is what `cancel`'s purpose
    // promises.
    assert_eq!(f.app.model(|m| m.gallery.len()), 0);
    assert!(!f.gallery().exists() || crate::gallery::listing(&f.gallery(), 12, false).is_empty());
    assert!(f.app.model(|m| m.queue.is_empty()));
}

/// The framework stops every job when the app ends: a close or a logout stops a render in flight
/// rather than leaving it to run into a process that is gone.
#[test]
fn the_apps_end_stops_a_picture_that_has_not_begun() {
    let f = fake("shutdown");
    f.ask("generate", json!({ "prompt": "a lighthouse", "width": 64, "height": 64 })).unwrap();
    // The worker has not run. The app ends; the framework cancels its jobs.
    f.app.shutdown();
    f.app.run_jobs();
    let made: Vec<_> = std::fs::read_dir(f.gallery()).map(|d| d.flatten().collect()).unwrap_or_default();
    assert!(made.is_empty(), "a picture was made after the app ended: {made:?}");
}

#[test]
fn a_close_a_quit_and_a_logout_end_the_app_at_once_whatever_is_being_made() {
    for (how, expected) in [("close", ExitReason::LastWindowClosed), ("quit", ExitReason::App), ("logout", ExitReason::App)] {
        let f = fake(&format!("end-{how}"));
        f.ask("generate", json!({ "prompt": "a lighthouse", "width": 64, "height": 64 })).unwrap();
        assert!(!f.app.model(|m| m.queue.is_empty()), "nothing was in flight");
        match how {
            "close" => f.app.close_requested(WindowId::PRIMARY),
            "quit" => f.app.event(yantrik_app::app::Event::ExitRequested(ExitReason::Person)),
            _ => f.app.display_gone(),
        }
        assert_eq!(f.app.exit_reason(), Some(expected), "{how} did not end the app");
        assert!(!f.app.lingering(), "{how} kept the app alive with nothing to wait for");
    }
}
