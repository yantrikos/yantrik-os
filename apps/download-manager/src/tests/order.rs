//! Answers per invocation: each call is told what its own call found, whatever else is asked
//! beside it and whatever the person does meanwhile.

use serde_json::json;

use super::{body, open, server::File};

#[test]
fn two_calls_asked_together_are_each_answered_with_their_own_download() {
    let f = open(vec![("/one.bin", File::plain(body(8_000))), ("/two.bin", File::plain(body(9_000)))]);
    let first = f.app.act("add", json!({ "url": f.server.url("/one.bin") })).unwrap();
    let second = f.app.act("add", json!({ "url": f.server.url("/two.bin") })).unwrap();
    // The second is read first, as a slow caller might.
    let second = f.settled(second).unwrap();
    let first = f.settled(first).unwrap();
    assert_eq!(first["filename"], "one.bin");
    assert_eq!(second["filename"], "two.bin");
    assert_ne!(first["id"], second["id"]);
}

#[test]
fn a_refusal_is_the_refused_calls_alone() {
    let f = open(vec![("/ok.bin", File::plain(body(8_000)))]);
    let bad = f.app.act("add", json!({ "url": "file:///etc/passwd" })).unwrap();
    let good = f.app.act("add", json!({ "url": f.server.url("/ok.bin") })).unwrap();
    let good = f.settled(good).unwrap();
    let bad = f.settled(bad);
    assert_eq!(good["filename"], "ok.bin", "the good call is not told the other's refusal");
    assert_eq!(bad.unwrap_err(), "`file:///etc/passwd` is not an http(s) URL");
    // The window shows the last outcome to land, which is the refused call's.
    assert_eq!(f.state()["total"], 1);
}

#[test]
fn what_the_person_does_meanwhile_is_not_the_answer() {
    let f = open(vec![("/m.bin", File::plain(body(8_000))), ("/n.bin", File::plain(body(7_000)))]);
    let id = f.download("/m.bin");
    let waiting = f.app.act("verify", json!({ "id": id })).unwrap();
    // Before the verify lands: another download is added and the list is filtered and cleared.
    f.app.ui({
        let url = f.server.url("/n.bin");
        move |ui| {
            ui.invoke_dl_add(url.into(), "".into(), "".into());
            ui.invoke_dl_filter(3);
            ui.invoke_dl_search("zzz".into());
        }
    });
    let answer = f.settled(waiting).unwrap();
    assert_eq!(answer["id"], json!(id), "the verify is answered with the download it was asked about");
    assert_eq!(answer["filename"], "m.bin");
}

#[test]
fn a_call_asked_once_the_app_is_closing_to_start_something_is_refused() {
    let f = open(vec![("/z.bin", File::held_at(body(300_000), 20_000))]);
    f.act("add", json!({ "url": f.server.url("/z.bin") })).unwrap();
    f.until("it is moving", |f| f.state()["active"] == 1);
    f.app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    let refused = f.act("add", json!({ "url": f.server.url("/z.bin") })).unwrap_err();
    assert_eq!(refused, crate::update::CLOSING);
    f.server.release();
    f.until("the app ends", |f| f.app.exit_reason().is_some());
}
