//! A message unsent at the last close, and the draft's mark (review of 7c8d82af on #920): the
//! person's choice about the draft is the last word whenever the first look lands, the mark is
//! written as the draft is, a failure is never offered as unknown, and new text from `compose` is
//! not said to be of unknown outcome.

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths};

use super::compose::{draft, type_into_composer};
use super::{act, notice, quiet, start_at, with_inbox};
use crate::backend::fake::FakeMail;
use crate::files;
use crate::state::{self, Draft};
use crate::Email;

const NOT_KNOWN: &str = "whether it went is not known";

fn send_from_composer(app: &yantrik_testing::TestApp<Email>, body: &str) {
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(app, &draft("a@example.com", body));
    let body = body.to_string();
    app.ui(move |ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), body.into()));
}

/// A first session that closes with a send of `body` still waiting for a worker.
fn closed_with_a_send_queued(mail: &FakeMail, body: &str) -> Paths {
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), mail, FakeMind::new());
    quiet(&app);
    send_from_composer(&app, body);
    let _ = app.shutdown();
    drop(app);
    paths
}

fn draft_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir().join(files::DRAFT)
}

/// M4: an edit kept before the first look lands is the draft, then and after a restart.
#[test]
fn an_edit_kept_before_the_first_look_lands_is_the_draft() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = closed_with_a_send_queued(&mail, "maybe went");
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    // The first look is still out.
    app.ui(|ui| ui.invoke_compose_new());
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "maybe went");
    type_into_composer(&app, &draft("a@example.com", "my careful edit"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert_eq!(state::load_draft(&draft_file(&paths)).unwrap().map(|d| d.body), Some("my careful edit".to_string()));
    assert!(!notice(&app).contains(NOT_KNOWN), "{}", notice(&app));
    let _ = app.shutdown();
    drop(app);
    let (again, _) = start_at(paths, &mail, FakeMind::new());
    quiet(&again);
    again.ui(|ui| ui.invoke_compose_new());
    assert_eq!(again.ui(|ui| ui.get_compose_body()), "my careful edit");
}

/// M4: a draft discarded before the first look lands stays discarded.
#[test]
fn a_draft_discarded_before_the_first_look_lands_stays_discarded() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = closed_with_a_send_queued(&mail, "maybe went");
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &Draft::default());
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert_eq!(state::load_draft(&draft_file(&paths)).unwrap(), None);
    assert!(!files::is_unsure(&draft_file(&paths)));
    assert!(!notice(&app).contains(NOT_KNOWN), "{}", notice(&app));
    app.ui(|ui| ui.invoke_compose_new());
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "");
}

/// M4: when the first look does not land, an edit kept before it is still the draft.
#[test]
fn an_edit_kept_before_a_first_look_that_fails_is_the_draft() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = closed_with_a_send_queued(&mail, "maybe went");
    let (app, _) = start_at(paths.clone(), &with_inbox().panicking("email.list_messages"), FakeMind::new());
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "my careful edit"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert_eq!(state::load_draft(&draft_file(&paths)).unwrap().map(|d| d.body), Some("my careful edit".to_string()));
}

/// L8: the mark is written as the draft is: whole, 0600, and never through a link standing
/// where it goes.
#[cfg(unix)]
#[test]
fn the_mark_beside_the_draft_is_private_and_never_written_through_a_link() {
    use std::os::unix::fs::PermissionsExt as _;
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = closed_with_a_send_queued(&mail, "maybe went");
    let mark = files::unsure_mark(&draft_file(&paths));
    let precious = paths.data_dir().join("precious.txt");
    std::fs::create_dir_all(paths.data_dir()).unwrap();
    std::fs::write(&precious, "precious").unwrap();
    std::os::unix::fs::symlink(&precious, &mark).unwrap();
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    assert_eq!(std::fs::read_to_string(&precious).unwrap(), "precious", "the link was not followed");
    let meta = std::fs::symlink_metadata(&mark).unwrap();
    assert!(meta.file_type().is_file(), "the mark is a file of its own");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
}

/// L9: a send that failed, from a composer emptied meanwhile, keeps what was sent as an ordinary
/// draft: the failure is known, so it is never offered as of unknown outcome.
#[test]
fn a_failed_send_from_an_emptied_composer_is_kept_and_not_offered_as_unknown() {
    let mail = with_inbox().failing("email.send_message", "554 rejected");
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    send_from_composer(&app, "v1");
    type_into_composer(&app, &Draft::default());
    quiet(&app);
    assert_eq!(state::load_draft(&draft_file(&paths)).unwrap().map(|d| d.body), Some("v1".to_string()));
    let _ = app.shutdown();
    drop(app);
    let (again, _) = start_at(paths, &mail, FakeMind::new());
    quiet(&again);
    assert!(!notice(&again).contains(NOT_KNOWN), "{}", notice(&again));
    again.ui(|ui| ui.invoke_compose_new());
    assert_eq!(again.ui(|ui| ui.get_compose_body()), "v1");
    assert_eq!(again.ui(|ui| ui.get_email_draft_status()), "Draft restored");
}

/// L10: `compose` replaces the text, so what it opens is not the message whose send may have
/// gone, and Cancel does not mark it as one.
#[test]
fn compose_over_a_draft_of_unknown_outcome_is_new_text() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = closed_with_a_send_queued(&mail, "maybe went");
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    act(&app, "compose", json!({ "to": "mom@example.com", "subject": "Sunday", "body": "Lunch?" })).unwrap();
    assert_eq!(app.ui(|ui| ui.get_email_draft_status()), "Draft restored");
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert_eq!(state::load_draft(&draft_file(&paths)).unwrap().map(|d| d.body), Some("Lunch?".to_string()));
    assert!(!files::is_unsure(&draft_file(&paths)), "new text is not marked");
}

/// The writer builds `.part` from the path's own bytes, so a folder whose name is not UTF-8 is
/// written to like any other.
#[cfg(unix)]
#[test]
fn a_draft_is_written_into_a_folder_whose_name_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt as _;
    let dir = std::env::temp_dir()
        .join(format!("email-osstr-{}", std::process::id()))
        .join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(files::DRAFT);
    state::save_draft(&file, &draft("a@example.com", "bytes")).unwrap();
    assert_eq!(state::load_draft(&file).unwrap().map(|d| d.body), Some("bytes".to_string()));
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}
