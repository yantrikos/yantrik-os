//! Where an agent may have a file saved, and what it may retry (#443, #942). The person is not
//! asked: only a call that carries an agent's token is held to the rule, and it is held to it in
//! the job that adds the download, off the UI thread.

use serde_json::json;

use super::{body, home, open, server::File};

#[test]
fn an_agent_saves_into_an_ordinary_folder_of_the_home() {
    let f = open(vec![("/ok.bin", File::plain(body(4_000)))]);
    let dir = home().join("Downloads");
    let answer = f.agent("add", json!({ "url": f.server.url("/ok.bin"), "save_dir": dir })).unwrap();
    let id = answer["id"].as_i64().unwrap();
    assert_eq!(answer["path"], json!(dir.join("ok.bin").to_string_lossy()));
    f.until("it completes", |f| f.row(id)["status"] == "completed");
    assert!(dir.join("ok.bin").exists());
}

#[test]
fn an_agent_never_picks_a_protected_destination() {
    let f = open(vec![("/evil.desktop", File::plain(body(100)))]);
    let autostart = home().join(".config/autostart");
    std::fs::create_dir_all(&autostart).unwrap();
    for dir in [autostart.to_string_lossy().into_owned(), "/etc".into(), "/tmp".into(), home().join("not/there/yet").to_string_lossy().into_owned()] {
        let refused = f.agent("add", json!({ "url": f.server.url("/evil.desktop"), "save_dir": dir })).unwrap_err();
        assert!(refused.starts_with("an agent may not save a download there:"), "{dir}: {refused}");
    }
    assert_eq!(f.state()["total"], 0, "nothing was queued, so nothing was written");
    assert!(!autostart.join("evil.desktop").exists());
}

#[test]
fn an_agent_with_no_folder_named_is_held_to_the_default_folder_too() {
    let f = open(vec![("/d.bin", File::plain(body(100)))]);
    // The default folder here is outside the tests' home, as a person's may be outside theirs.
    let refused = f.agent("add", json!({ "url": f.server.url("/d.bin") })).unwrap_err();
    assert!(refused.starts_with("an agent may not save a download there:"), "{refused}");
    // The same call as the person is not asked.
    assert!(f.act("add", json!({ "url": f.server.url("/d.bin") })).is_ok());
}

#[test]
fn a_refusal_names_no_path_the_agent_did_not_give() {
    let f = open(vec![("/e.bin", File::plain(body(100)))]);
    let refused = f.agent("add", json!({ "url": f.server.url("/e.bin"), "save_dir": "/etc" })).unwrap_err();
    assert!(!refused.contains(&f.paths.data_dir().to_string_lossy().into_owned()), "{refused}");
}

#[test]
fn an_agent_may_not_delete_a_finished_download_to_fetch_it_again() {
    let f = open(vec![("/r.bin", File::plain(body(4_000)))]);
    let id = f.download("/r.bin");
    let refused = f.agent("retry", json!({ "id": id })).unwrap_err();
    assert_eq!(refused, format!("download {id} finished; an agent may not delete it to fetch it again"));
    assert!(f.default_dir().join("r.bin").exists());
    // The person may.
    assert!(f.act("retry", json!({ "id": id })).is_ok());
}
