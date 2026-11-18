//! The review of #958: an agent's cancel never deletes a finished file (M1), an agent's fetch never
//! reaches the machine's own services, also through a redirect (M2).

use std::path::PathBuf;

use serde_json::json;

use super::{body, home, open, server::File, server::Server, Fixture};

fn downloads() -> PathBuf {
    home().join("Downloads")
}

/// An agent's add into the tests' home, answered at once or refused.
fn agent_adds(f: &Fixture, url: &str) -> Result<i64, String> {
    f.agent("add", json!({ "url": url, "save_dir": downloads() })).map(|a| a["id"].as_i64().unwrap())
}

fn failed_with(f: &Fixture, id: i64) -> String {
    f.until("it fails", |f| f.row(id)["status"] == "failed");
    f.row(id)["error"].as_str().unwrap_or_default().to_string()
}

#[test]
fn an_agents_cancel_of_a_finished_download_is_refused_and_the_file_stays() {
    let f = open(vec![("/m1a.bin", File::plain(body(4_000)))]);
    let id = f.download("/m1a.bin");
    let refused = f.agent("cancel", json!({ "id": id })).unwrap_err();
    assert!(refused.contains("an agent may not remove it"), "{refused}");
    assert_eq!(std::fs::read(f.default_dir().join("m1a.bin")).unwrap(), body(4_000));
    assert_eq!(f.row(id)["status"], "completed", "the row is as it was");
}

#[test]
fn the_persons_cancel_of_a_finished_download_takes_the_row_and_keeps_the_file() {
    let f = open(vec![("/m1b.bin", File::plain(body(4_000)))]);
    let id = f.download("/m1b.bin");
    let answer = f.act("cancel", json!({ "id": id })).unwrap();
    assert_eq!(answer["status"], "removed");
    assert_eq!(f.row(id), serde_json::Value::Null, "the row is gone");
    assert_eq!(std::fs::read(f.default_dir().join("m1b.bin")).unwrap(), body(4_000), "the file is not");
}

#[test]
fn a_cancel_of_a_running_download_removes_only_its_partial_file() {
    let f = open(vec![("/m1c.bin", File::held_at(body(300_000), 70_000))]);
    let id = f.act("add", json!({ "url": f.server.url("/m1c.bin") })).unwrap()["id"].as_i64().unwrap();
    let path = f.default_dir().join("m1c.bin");
    f.until("some is on disk", |_| std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false));
    f.act("cancel", json!({ "id": id })).unwrap();
    f.server.release();
    f.until("it ends", |f| f.row(id)["status"] == "failed");
    assert_eq!(f.row(id)["error"], "cancelled");
    assert!(!path.exists(), "the partial file is gone");
}

#[test]
fn an_agent_may_not_add_an_address_of_this_machine() {
    let secret = Server::start_unlisted(vec![("/history", File::plain(b"SECRET-LOCAL-SERVICE-BODY".to_vec()))]);
    let f = open(vec![]);
    for url in [secret.url("/history"), format!("http://localhost:{}/history", secret.port), format!("http://[::1]:{}/x", secret.port)] {
        let refused = agent_adds(&f, &url).unwrap_err();
        assert!(refused.contains("an agent may not fetch"), "{url}: {refused}");
    }
    assert_eq!(f.state()["total"], 0, "nothing was queued");
    assert!(secret.ranges.lock().unwrap().is_empty(), "nothing connected");
    assert!(!downloads().join("history").exists());
}

#[test]
fn an_agent_may_not_be_sent_to_this_machine_by_a_redirect() {
    let secret = Server::start_unlisted(vec![("/history", File::plain(b"SECRET-LOCAL-SERVICE-BODY".to_vec()))]);
    let f = open(vec![("/m2-innocent.txt", File::redirect(&secret.url("/history")))]);
    let id = agent_adds(&f, &f.server.url("/m2-innocent.txt")).unwrap();
    let error = failed_with(&f, id);
    assert!(error.contains("an agent may not fetch"), "{error}");
    assert!(secret.ranges.lock().unwrap().is_empty(), "the second hop never connected");
    assert!(!downloads().join("m2-innocent.txt").exists());
}

#[test]
fn an_agents_redirect_must_stay_http_and_must_end() {
    let f = open(vec![
        ("/m2-file", File::redirect("file:///etc/passwd")),
        ("/m2-loop", File::redirect("/m2-loop")),
    ]);
    let id = agent_adds(&f, &f.server.url("/m2-file")).unwrap();
    let error = failed_with(&f, id);
    assert!(error.contains("only http and https"), "{error}");
    let id = agent_adds(&f, &f.server.url("/m2-loop")).unwrap();
    let error = failed_with(&f, id);
    assert!(error.contains("redirects"), "{error}");
}

#[test]
fn an_agent_still_follows_a_redirect_that_stays_on_the_public_side() {
    let f = open(vec![("/m2-a.bin", File::redirect("/m2-b.bin")), ("/m2-b.bin", File::plain(body(3_000)))]);
    let id = agent_adds(&f, &f.server.url("/m2-a.bin")).unwrap();
    f.until("it completes", |f| f.row(id)["status"] == "completed");
    assert_eq!(std::fs::read(downloads().join("m2-a.bin")).unwrap(), body(3_000));
}

#[test]
fn the_persons_add_is_unchanged_and_may_follow_a_redirect_to_the_local_network() {
    let local = Server::start_unlisted(vec![("/page.bin", File::plain(body(2_000)))]);
    let f = open(vec![("/p-go.bin", File::redirect(&local.url("/page.bin")))]);
    let id = f.act("add", json!({ "url": f.server.url("/p-go.bin") })).unwrap()["id"].as_i64().unwrap();
    f.until("it completes", |f| f.row(id)["status"] == "completed");
    assert_eq!(std::fs::read(f.default_dir().join("p-go.bin")).unwrap(), body(2_000));
    let own = f.act("add", json!({ "url": local.url("/page.bin") })).unwrap();
    assert!(own["id"].as_i64().is_some(), "an address of this machine is the person's to fetch");
}
