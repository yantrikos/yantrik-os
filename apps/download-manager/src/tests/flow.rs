//! A download from the add to the file on disk, the checksum, and each action's answer, through
//! the window's callbacks and through the surface.

use serde_json::json;

use super::{body, open, server::File, sha256_of};

#[test]
fn an_add_is_answered_with_its_own_download_and_the_file_arrives_whole() {
    let data = body(300_000);
    let f = open(vec![("/disk.iso", File::plain(data.clone()))]);
    let answer = f.act("add", json!({ "url": f.server.url("/disk.iso") })).unwrap();
    assert_eq!(answer["filename"], "disk.iso");
    assert_eq!(answer["path"], json!(f.default_dir().join("disk.iso").to_string_lossy()));
    // Answered when the call landed: queued or already moving, never finished in the same breath.
    assert!(["queued", "downloading"].contains(&answer["status"].as_str().unwrap()), "{answer}");
    let id = answer["id"].as_i64().unwrap();
    f.until("it completes and is hashed", |f| f.row(id)["status"] == "completed" && f.row(id).get("sha256").is_some());
    assert_eq!(std::fs::read(f.default_dir().join("disk.iso")).unwrap(), data);
    assert_eq!(f.row(id)["sha256"], json!(sha256_of(&data)));
}

#[test]
fn an_expected_checksum_is_checked_when_the_file_finishes() {
    let data = body(50_000);
    let f = open(vec![("/a.bin", File::plain(data.clone())), ("/b.bin", File::plain(data.clone()))]);
    let good = f.act("add", json!({ "url": f.server.url("/a.bin"), "sha256": sha256_of(&data).to_uppercase() })).unwrap()["id"].as_i64().unwrap();
    let bad = f.act("add", json!({ "url": f.server.url("/b.bin"), "sha256": "00".repeat(32) })).unwrap()["id"].as_i64().unwrap();
    f.until("both finish and are checked", |f| f.row(good)["checksum"] == "pass" && f.row(bad)["checksum"] == "fail");
    assert_eq!(f.summary(), "Downloads \u{2014} b.bin downloaded but its checksum does not match");
}

#[test]
fn verify_hashes_again_and_answers_with_the_row() {
    let data = body(40_000);
    let f = open(vec![("/v.bin", File::plain(data.clone()))]);
    let id = f.download("/v.bin");
    let answer = f.act("verify", json!({ "id": id, "sha256": sha256_of(&data) })).unwrap();
    assert_eq!(answer["id"], json!(id));
    f.until("the check lands", |f| f.row(id)["checksum"] == "pass");
    let refused = f.act("verify", json!({ "id": 999 })).unwrap_err();
    assert_eq!(refused, "no download 999");
    assert_eq!(f.banner(), "no download 999", "a refusal is said on the window too");
}

#[test]
fn the_windows_add_closes_its_dialog_only_when_it_took() {
    let f = open(vec![("/w.bin", File::plain(body(10_000)))]);
    f.app.ui(|ui| {
        ui.set_add_url_open(true);
        ui.invoke_dl_add("ftp://nope".into(), "".into(), "".into());
    });
    f.quiet();
    assert!(f.app.ui(|ui| ui.get_add_url_open()), "a rejected URL stays on screen to be fixed");
    assert_eq!(f.banner(), "`ftp://nope` is not an http(s) URL");
    let url = f.server.url("/w.bin");
    f.app.ui(move |ui| {
        ui.set_add_url_text(url.clone().into());
        ui.invoke_dl_add(url.into(), "".into(), "".into());
    });
    f.quiet();
    assert!(!f.app.ui(|ui| ui.get_add_url_open()), "and closes on success");
    assert_eq!(f.app.ui(|ui| ui.get_add_url_text().to_string()), "");
    assert_eq!(f.banner(), "");
}

#[test]
fn an_agents_add_leaves_the_persons_dialog_alone() {
    let f = open(vec![("/x.bin", File::plain(body(1_000)))]);
    f.app.ui(|ui| ui.set_add_url_open(true));
    // The default folder is outside the tests' home, so an agent may not save there; the person's
    // own call of the same add took.
    f.act("add", json!({ "url": f.server.url("/x.bin") })).unwrap();
    assert!(f.app.ui(|ui| ui.get_add_url_open()), "a call through the surface is none of the dialog's business");
}

#[test]
fn pause_then_resume_continues_from_the_bytes_on_disk() {
    let data = body(600_000);
    let f = open(vec![("/big.bin", File::held_at(data.clone(), 100_000))]);
    let id = f.act("add", json!({ "url": f.server.url("/big.bin") })).unwrap()["id"].as_i64().unwrap();
    // Bytes on disk, not just the status word: a pause set before the worker has written
    // anything is legitimate, but this test is about resuming a part-written file.
    f.until("the first part is in", |f| {
        f.row(id)["status"] == "downloading"
            && std::fs::metadata(f.default_dir().join("big.bin")).map(|m| m.len() > 0).unwrap_or(false)
    });
    f.act("pause", json!({ "id": id })).unwrap();
    // The worker is blocked on the server and sees the flag at its next chunk.
    f.server.release();
    f.until("it is paused", |f| f.row(id)["status"] == "paused");
    let have = std::fs::metadata(f.default_dir().join("big.bin")).unwrap().len() as usize;
    assert!(have > 0 && have < data.len(), "{have} bytes of {}", data.len());
    assert_eq!(std::fs::read(f.default_dir().join("big.bin")).unwrap(), data[..have], "what arrived is a prefix of the body");
    f.act("resume", json!({ "id": id })).unwrap();
    f.until("it completes", |f| f.row(id)["status"] == "completed");
    assert_eq!(std::fs::read(f.default_dir().join("big.bin")).unwrap(), data, "resumed, not restarted or stitched wrongly");
    assert_eq!(f.server.ranges.lock().unwrap().last().cloned().flatten(), Some(have.to_string()));
}

#[test]
fn cancel_deletes_the_partial_file_and_says_so() {
    let f = open(vec![("/c.bin", File::held_at(body(400_000), 50_000))]);
    let id = f.act("add", json!({ "url": f.server.url("/c.bin") })).unwrap()["id"].as_i64().unwrap();
    f.until("the first part is in", |f| f.row(id)["status"] == "downloading");
    f.act("cancel", json!({ "id": id })).unwrap();
    f.server.release();
    f.until("it is gone", |f| f.row(id)["status"] == "failed");
    assert_eq!(f.row(id)["error"], "cancelled");
    assert!(!f.default_dir().join("c.bin").exists(), "the partial file went with it");
}

#[test]
fn pause_all_resume_all_and_clear_completed_answer_with_what_they_did() {
    let f = open(vec![("/one.bin", File::plain(body(20_000))), ("/two.bin", File::held_at(body(300_000), 30_000))]);
    f.download("/one.bin");
    let two = f.act("add", json!({ "url": f.server.url("/two.bin") })).unwrap()["id"].as_i64().unwrap();
    f.until("the second is moving", |f| f.row(two)["status"] == "downloading");
    f.act("pause_all", json!({})).unwrap();
    f.server.release();
    f.until("it is paused", |f| f.row(two)["status"] == "paused");
    assert_eq!(f.act("pause_all", json!({})).unwrap(), json!({ "paused": 1 }));
    let resumed = f.act("resume_all", json!({})).unwrap();
    assert!(resumed["active"].as_i64().unwrap() <= 1);
    f.until("it completes", |f| f.row(two)["status"] == "completed");
    assert_eq!(f.act("clear_completed", json!({})).unwrap(), json!({ "cleared": 2 }));
    assert_eq!(f.state()["total"], 0);
    assert!(f.default_dir().join("one.bin").exists(), "the files stay");
}

#[test]
fn open_folder_of_a_download_that_is_not_there_is_refused_in_the_old_words() {
    let f = open(vec![]);
    assert_eq!(f.act("open_folder", json!({ "id": 7 })).unwrap_err(), "no download 7");
    assert_eq!(f.act("pause", json!({ "id": 7 })).unwrap_err(), "no download 7");
}

#[test]
fn a_download_that_failed_while_nobody_looked_is_said_on_the_banner() {
    let f = open(vec![]);
    f.app.ui({
        let url = f.server.url("/nothing.bin");
        move |ui| ui.invoke_dl_add(url.into(), "".into(), "".into())
    });
    f.until("the failure is on the banner", |f| f.banner().contains("server said HTTP 404"));
    assert_eq!(f.banner(), "nothing.bin \u{2014} server said HTTP 404");
}

#[test]
fn the_windows_filters_search_and_selection_follow_their_callbacks() {
    use slint::Model as _;
    let f = open(vec![("/alpha.bin", File::plain(body(5_000))), ("/beta.bin", File::plain(body(6_000)))]);
    f.download("/alpha.bin");
    f.download("/beta.bin");
    let rows = |f: &super::Fixture| f.app.ui(|ui| ui.get_downloads().row_count());
    assert_eq!(rows(&f), 2);
    f.app.ui(|ui| ui.invoke_dl_search("alp".into()));
    assert_eq!(rows(&f), 1);
    f.app.ui(|ui| ui.invoke_dl_search("".into()));
    f.app.ui(|ui| ui.invoke_dl_filter(1));
    assert_eq!(rows(&f), 0, "nothing is active");
    f.app.ui(|ui| ui.invoke_dl_filter(2));
    assert_eq!(rows(&f), 2);
    f.app.ui(|ui| ui.invoke_dl_select_all(true));
    assert!(f.app.ui(|ui| ui.get_downloads().row_data(0).unwrap().is_selected));
    assert!(f.app.ui(|ui| ui.get_dl_all_selected()));
}
