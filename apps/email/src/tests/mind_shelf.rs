//! Each promise keeps its own prepared draft (#1184, gate 8): a newer promise's draft does not
//! replace an earlier promise's while that card still says it is in Email.

use serde_json::json;
use yantrik_app::{App, Paths};

use yantrik_app::mind::fake::FakeMind;

use super::{act, quiet, start_at, state, with_inbox};
use crate::command::Command;
use crate::files;
use crate::Email;

fn started(paths: &Paths) -> yantrik_testing::TestApp<Email> {
    let (app, _) = start_at(paths.clone(), &with_inbox(), FakeMind::new());
    quiet(&app);
    app
}

fn home_file(name: &str, tag: &str) -> std::path::PathBuf {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    let dir = home.join(format!(".yantrik-email-shelf-test-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, b"x").unwrap();
    file
}

fn compose(app: &yantrik_testing::TestApp<Email>, to: &str, file: &std::path::Path, commitment: &str) {
    let args = json!({ "to": to, "subject": "report", "body": "Here.", "attach": file.to_string_lossy(), "commitment": commitment });
    act(app, "compose", args).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(app);
}

fn shown_to(app: &yantrik_testing::TestApp<Email>) -> String {
    app.ui(|ui| ui.get_compose_to().to_string())
}

#[test]
fn two_promises_each_keep_their_draft_and_each_opens_its_own() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "two");
    let b = home_file("beta.ydeck", "two");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "ravi@example.com", &b, "25");
    let slot = paths.data_dir().join(files::MIND_DRAFT);
    assert!(slot.exists(), "the newest is in the slot");
    assert!(paths.data_dir().join("mind_draft.24.json").exists(), "the earlier promise's draft is kept");

    app.run(Command::OpenPreparedFor("24".into()));
    quiet(&app);
    assert_eq!(shown_to(&app), "maya@example.com");
    assert_eq!(state(&app)["draft_attachments"], json!([a.canonicalize().unwrap().to_string_lossy()]));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    app.run(Command::OpenPreparedFor("25".into()));
    quiet(&app);
    assert_eq!(shown_to(&app), "ravi@example.com");
    assert_eq!(state(&app)["draft_attachments"], json!([b.canonicalize().unwrap().to_string_lossy()]));
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn forgetting_one_promise_leaves_the_other_promises_draft() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "forget");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "ravi@example.com", &a, "25");
    let shelf = paths.data_dir().join("mind_draft.24.json");
    act(&app, "forget_prepared", json!({ "commitment": "25" })).unwrap();
    assert!(shelf.exists() && !paths.data_dir().join(files::MIND_DRAFT).exists(), "only 25 went");
    act(&app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    assert!(!shelf.exists(), "the waiting draft clears when its promise is forgotten");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn a_second_draft_for_the_same_promise_replaces_the_first() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "same");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "maya2@example.com", &a, "24");
    assert!(!paths.data_dir().join("mind_draft.24.json").exists());
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn an_id_that_could_name_a_path_is_not_shelved() {
    let slot = std::path::Path::new("/d/mind_draft.json");
    assert_eq!(crate::state::shelf_path(slot, "../x"), None);
    assert_eq!(crate::state::shelf_path(slot, ""), None);
    assert_eq!(crate::state::shelf_path(slot, "24").unwrap(), std::path::Path::new("/d/mind_draft.24.json"));
}
