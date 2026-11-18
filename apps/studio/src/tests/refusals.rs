//! A call that cannot work is refused in words a caller can act on, and the refusal is said twice:
//! to whoever asked, and in the notice bar the person is looking at. The two forms differ only
//! where the caller's names an action the person cannot call.

use serde_json::json;

use super::fake;
use crate::backend;
use crate::command::{Command, Which};
use crate::config;
use crate::gallery::{self, Sidecar};

#[test]
fn a_prompt_that_cannot_be_made_is_refused_and_the_person_is_told_without_an_action_name() {
    let f = fake("refuse-prompt");
    // An empty prompt. The dispatch refuses a *missing* required argument before the app runs;
    // this is the other half, one that is present and blank.
    let problem = f.act("generate", json!({ "prompt": "   " })).unwrap_err();
    assert!(problem.contains("a prompt is needed"), "{problem}");
    assert!(problem.contains("generate prompt="), "the refusal has to show the shape that works: {problem}");
    // Said twice: to whoever asked, and in the notice bar the person is looking at, which cannot
    // send a person to an action they cannot call.
    let notice = f.notice();
    assert!(notice.contains("a prompt is needed"), "{notice}");
    assert!(!notice.contains("generate prompt="), "the person is sent to an action: {notice}");
    assert!(notice.contains("prompt box"), "{notice}");

    // A count that would spend real money on a hosted backend.
    let problem = f.act("generate", json!({ "prompt": "a cliff", "count": 9 })).unwrap_err();
    assert!(problem.contains("more than the 4"), "{problem}");
    assert!(problem.contains("charges for each one"), "{problem}");

    // A prompt nobody's model would read.
    let problem = f.act("generate", json!({ "prompt": "x".repeat(4001) })).unwrap_err();
    assert!(problem.contains("4001 characters"), "{problem}");
    assert!(f.app.model(|m| m.queue.is_empty()), "a refused ask left a job in the queue");

    // Numbers, by contrast, are brought onto the range rather than refused: a mind that asks for
    // 9000 pixels wants a picture, and the record beside it says what was actually made. Two
    // shapings happen: the plan clamps the ask to 4096x64 (edges live in 64..=4096), and the fake
    // backend then caps its own long edge at 1024 and keeps the aspect, so the file on disk is
    // 1024x16. The record says what is really there, not what was asked for.
    f.act("generate", json!({ "prompt": "a wide sea", "width": 9000, "height": 3, "steps": 900 })).unwrap();
    assert_eq!(f.settled(), "");
    let shot = f.state()["gallery"]["newest"][0].clone();
    assert_eq!(shot["size"], json!("1024x16"), "{shot}");
    assert_eq!(Sidecar::read(std::path::Path::new(shot["path"].as_str().unwrap())).unwrap().steps, Some(150));
}

#[test]
fn a_name_that_is_not_a_picture_is_refused_before_anything_is_changed() {
    let f = fake("refuse-names");
    // Missing and empty arguments are named: the dispatch names a missing one, the app a blank one.
    let problem = f.act("variations", json!({})).unwrap_err();
    assert!(problem.contains("`variations` needs argument `of`"), "{problem}");
    let problem = f.act("delete", json!({ "path": " " })).unwrap_err();
    assert!(problem.contains("`delete` was given an empty `path`"), "{problem}");
    assert_eq!(f.notice(), "Nothing was named, so nothing was done.");

    // A picture that is not there, and one that is not a picture.
    let problem = f.act("variations", json!({ "of": "nothing-like-it.png" })).unwrap_err();
    assert!(problem.contains("nothing-like-it.png"), "{problem}");
    assert!(problem.contains("there is no"), "{problem}");
    assert!(problem.contains("`describe`"), "the caller is told where to look: {problem}");
    let notice = f.notice();
    assert!(notice.contains("there is no nothing-like-it.png in the gallery"), "{notice}");
    assert!(!notice.contains("`describe`"), "the person is sent to an action: {notice}");
    let problem = f.act("upscale", json!({ "path": "/tmp/studio-no-such-file.png" })).unwrap_err();
    assert!(problem.contains("is not there. Nothing was changed."), "{problem}");
    std::fs::write(f.world.dir.join("notes.txt"), "not a picture").unwrap();
    let problem = f.act("upscale", json!({ "path": f.world.dir.join("notes.txt").display().to_string() })).unwrap_err();
    assert!(problem.contains("is not a picture this OS recognises"), "{problem}");

    // `delete` is not a way to remove a file from somewhere else. This is the refusal the grade of
    // `standard` rests on.
    let outside = f.world.dir.join("outside.png");
    std::fs::write(&outside, backend::draw("elsewhere", 1, 32, 32)).unwrap();
    let problem = f.act("delete", json!({ "path": outside.display().to_string() })).unwrap_err();
    assert!(problem.contains("only deletes its own pictures"), "{problem}");
    assert!(problem.contains("Files can delete anything"), "{problem}");
    assert!(outside.exists(), "the refusal happened after the move");
    // The same file is readable through `upscale`, which changes nothing outside the gallery and
    // writes only inside it: reading a picture is not deleting one.
    f.act("upscale", json!({ "path": outside.display().to_string() })).unwrap();
    assert_eq!(f.settled(), "");
    assert!(outside.exists());
}

#[test]
fn a_picture_with_no_record_cannot_be_varied_and_the_refusal_says_what_to_do_instead() {
    let f = fake("refuse-record");
    let day = gallery::day_folder(&f.gallery(), chrono::Local::now());
    std::fs::create_dir_all(&day).unwrap();
    let stray = day.join("stray.png");
    std::fs::write(&stray, backend::draw("copied in by hand", 3, 64, 64)).unwrap();
    // A bare name is looked up in the gallery and then in today's folder, so this finds the file
    // whether or not the `refresh` below has finished re-reading, and either way the answer has to
    // be that there is nothing to vary, not a guess.
    let problem = f.act("variations", json!({ "of": "stray.png" })).unwrap_err();
    assert!(problem.contains("there is no record beside"), "{problem}");
    assert!(problem.contains("Generate from a sentence instead"), "{problem}");
    let notice = f.notice();
    assert!(notice.contains("there is no record beside"), "{notice}");
    assert!(!notice.contains("`generate"), "the person is sent to an action: {notice}");
    assert!(notice.contains("prompt box"), "{notice}");

    f.act("refresh", json!({})).unwrap();
    // And the gallery says which of its rows are unaccounted for, rather than presenting a guess.
    let listed = f.state()["gallery"]["newest"]
        .as_array()
        .unwrap()
        .iter()
        .find(|each| each["path"].as_str().unwrap().ends_with("stray.png"))
        .cloned()
        .expect("the picture copied in by hand never reached the gallery");
    assert_eq!(listed["sidecar"], json!("missing"), "{listed}");
    assert!(listed.get("seed").is_none(), "a row with no record must not invent a seed: {listed}");
}

#[test]
fn cancelling_what_is_not_there_is_a_refusal_and_never_stops_everything() {
    let f = fake("refuse-cancel");
    // Cancelling an empty queue is a refusal, not a success: a mind that reports "cancelled" when
    // nothing was running has been told something false.
    assert_eq!(f.act("cancel_all", json!({})).unwrap_err(), "nothing was queued or running");
    let problem = f.act("cancel", json!({ "job": 999 })).unwrap_err();
    assert!(problem.contains("there is no job 999"), "{problem}");
    assert_eq!(f.notice(), "there is no job 999 in the queue.");
    // An id nobody was ever issued is refused as that, with the action that really does stop
    // everything named: `-1` once read as 0 through a float cast and cancelled the whole queue.
    for not_a_job in [json!({ "job": -1 }), json!({ "job": 0 }), json!({ "job": 2.5 })] {
        let problem = f.act("cancel", not_a_job.clone()).unwrap_err();
        assert!(problem.contains("cancel_all"), "{not_a_job}: {problem}");
    }
    // The dispatch refuses a missing one, and one that is not a number, before the app is asked.
    assert!(f.act("cancel", json!({})).is_err());
    assert!(f.act("cancel", json!({ "job": "two" })).is_err());
    // And the app's own sentence for a call that reaches it without a usable id.
    f.app.run(Command::Cancel(Which::Unusable { given: false }));
    assert_eq!(f.notice(), "A job could not be named, so nothing was stopped.", "the person is not sent to an action");
}

#[test]
fn a_backend_studio_does_not_have_is_refused_with_the_ones_it_does() {
    let f = fake("refuse-backend");
    let problem = f.act("set_backend", json!({ "kind": "midjourney" })).unwrap_err();
    assert!(problem.contains("`midjourney` is not a backend Studio has"), "{problem}");
    assert!(problem.contains(config::KINDS[0]), "the refusal lists what would work: {problem}");
    let problem = f.act("set_backend", json!({ "kind": "openai-images" })).unwrap_err();
    assert!(problem.contains("`model` is required"), "{problem}");
    let problem = f.act("set_backend", json!({ "kind": "  " })).unwrap_err();
    assert!(problem.contains("`set_backend` was given an empty `kind`"), "{problem}");
    // Nothing was written, and nothing changed.
    assert!(!f.world.places.settings.as_ref().unwrap().exists());
    assert_eq!(f.state()["backend"]["kind"], json!("fake"));
    assert!(!f.app.model(|m| m.changing_backend), "a refused change left the app waiting for it");
}
