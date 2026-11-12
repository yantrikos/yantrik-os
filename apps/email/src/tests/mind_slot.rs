//! A Mind's draft lives in its own slot (#1184, review of 377b29c5, HIGH-1 and LOW-12): it never
//! touches the person's kept or open draft, starts empty, keeps its file and its refusal to send
//! across a restart, and only attaches a file that is the person's to give.

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths};

use super::compose::{draft, type_into_composer};
use super::{act, notice, quiet, start_at, state, with_inbox};
use crate::files;
use crate::state::{self, Draft};
use crate::Email;

/// A file under the person's home, removed when this goes out of scope.
struct HomeFile(std::path::PathBuf);

impl HomeFile {
    fn new(name: &str) -> HomeFile {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
        let dir = home.join(format!(".yantrik-email-slot-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Q4-deck.ydeck");
        std::fs::write(&file, b"deck").unwrap();
        HomeFile(file)
    }

    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for HomeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn compose_args(attach: &str) -> serde_json::Value {
    json!({ "to": "maya@example.com", "subject": "Q4 deck", "body": "Here it is.", "attach": attach })
}

fn started_at(paths: &Paths) -> yantrik_testing::TestApp<Email> {
    let (app, _) = start_at(paths.clone(), &with_inbox(), FakeMind::new());
    quiet(&app);
    app
}

fn kept_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir().join(files::DRAFT)
}

fn kept_with_cc(paths: &Paths) -> Draft {
    let kept = Draft { to: "a@example.com".into(), cc: "hr@corp.example".into(), bcc: "b@corp.example".into(), subject: "Mine".into(), body: "half written".into() };
    state::save_draft(&kept_file(paths), &kept).unwrap();
    kept
}

#[test]
fn a_draft_with_a_file_attached_keeps_it_and_is_never_sent() {
    let file = HomeFile::new("sent");
    let mail = with_inbox();
    let (app, _) = start_at(Paths::temporary(Email::ID), &mail, FakeMind::new());
    quiet(&app);
    act(&app, "compose", compose_args(&file.path())).unwrap();
    assert_eq!(state(&app)["draft_attachments"], json!([std::fs::canonicalize(&file.0).unwrap().to_string_lossy()]));
    app.ui(|ui| ui.invoke_send_email("maya@example.com".into(), "".into(), "".into(), "Q4 deck".into(), "Here it is.".into()));
    assert_eq!(app.jobs_waiting(), 0, "nothing was queued to send");
    assert!(notice(&app).starts_with("Not sent: this draft has a file attached"), "{}", notice(&app));
    assert!(mail.asked_for("email.send_message").is_empty());
}

#[test]
fn compose_over_a_kept_draft_is_refused_in_plain_words_and_leaves_it_intact() {
    let paths = Paths::temporary(Email::ID);
    let kept = kept_with_cc(&paths);
    let before = std::fs::read(kept_file(&paths)).unwrap();
    let app = started_at(&paths);
    let refused = act(&app, "compose", compose_args("")).unwrap_err();
    assert_eq!(refused, "You have an unsent message of your own, and a draft from Yantrik would not replace it. Send or discard yours first.");
    quiet(&app);
    assert_eq!(std::fs::read(kept_file(&paths)).unwrap(), before, "the kept draft file is as it was");
    assert_eq!(app.model(|m| m.composer.kept.clone()), Ok(Some(kept)));
    assert!(!app.ui(|ui| ui.get_is_composing()), "the composer did not change under the person");
    assert!(!paths.data_dir().join(files::MIND_DRAFT).exists());
}

#[test]
fn compose_while_the_person_is_typing_is_refused_and_their_text_stays() {
    let app = started_at(&Paths::temporary(Email::ID));
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "typing now"));
    assert!(act(&app, "compose", compose_args("")).is_err());
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "typing now");
}

#[test]
fn a_minds_draft_starts_empty_with_no_cc_or_bcc_and_is_not_the_persons_draft() {
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", compose_args("")).unwrap();
    let text = app.model(|m| m.composer.text.clone());
    assert_eq!((text.cc.as_str(), text.bcc.as_str()), ("", ""));
    assert_eq!(text.to, "maya@example.com");
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert!(!kept_file(&paths).exists(), "Cancel did not make it the person's draft");
    assert!(paths.data_dir().join(files::MIND_DRAFT).exists(), "it is kept in its own slot");
}

#[test]
fn the_file_and_the_refusal_to_send_survive_a_restart() {
    let file = HomeFile::new("restart");
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", compose_args(&file.path())).unwrap();
    app.event(yantrik_app::app::Event::ExitRequested(yantrik_app::app::ExitReason::Logout));
    let _ = app.shutdown();
    drop(app);
    assert!(!kept_file(&paths).exists(), "the end of the app did not hand it to the person's draft");

    let again = started_at(&paths);
    again.ui(|ui| ui.invoke_compose_new());
    assert_eq!(again.ui(|ui| ui.get_compose_body()), "Here it is.");
    assert_eq!(state(&again)["draft_attachments"].as_array().map(Vec::len), Some(1), "the file is still carried");
    again.ui(|ui| ui.invoke_send_email("maya@example.com".into(), "".into(), "".into(), "Q4 deck".into(), "Here it is.".into()));
    assert_eq!(again.jobs_waiting(), 0);
    assert!(notice(&again).starts_with("Not sent: this draft has a file attached"), "{}", notice(&again));
}

#[test]
fn compose_opens_the_persons_own_draft_first_when_both_exist() {
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", compose_args("")).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    let _ = app.shutdown();
    drop(app);
    kept_with_cc(&paths);
    let again = started_at(&paths);
    again.ui(|ui| ui.invoke_compose_new());
    assert_eq!(again.ui(|ui| ui.get_compose_body()), "half written", "the person's draft, not the Mind's");
}

#[test]
fn a_draft_the_person_changed_is_theirs_and_is_not_replaced() {
    let app = started_at(&Paths::temporary(Email::ID));
    act(&app, "compose", compose_args("")).unwrap();
    type_into_composer(&app, &draft("maya@example.com", "my own wording"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert!(act(&app, "compose", compose_args("")).is_err());
}

#[test]
fn a_file_that_is_missing_outside_home_or_protected_is_not_attached() {
    let app = started_at(&Paths::temporary(Email::ID));
    let home = std::env::var("HOME").unwrap();
    let cases = [
        ("/home/nobody-here/Q4.ydeck".to_string(), "is not there"),
        ("/etc/hostname".to_string(), "Only files in your home folder"),
        (format!("{home}/.ssh/id_ed25519"), ""),
        ("Q4.ydeck".to_string(), "full path"),
    ];
    for (path, says) in cases {
        let refused = act(&app, "compose", compose_args(&path)).unwrap_err();
        assert!(refused.contains(says), "{path}: {refused}");
        assert_eq!(state(&app)["draft_attachments"], json!([]), "{path}");
    }
    let protected = HomeFile::new("protected");
    let key = protected.0.parent().unwrap().join(".ssh");
    std::fs::create_dir_all(&key).unwrap();
    std::fs::write(key.join("id_ed25519"), b"key").unwrap();
    let refused = act(&app, "compose", compose_args(&key.join("id_ed25519").to_string_lossy())).unwrap_err();
    assert!(refused.contains("protected place"), "{refused}");
}

#[test]
fn a_sent_minds_draft_empties_its_slot_and_not_the_persons() {
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox().answering("email.send_message", json!({}));
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    act(&app, "compose", compose_args("")).unwrap();
    app.ui(|ui| ui.invoke_send_email("maya@example.com".into(), "".into(), "".into(), "Q4 deck".into(), "Here it is.".into()));
    quiet(&app);
    assert_eq!(mail.asked_for("email.send_message").len(), 1);
    assert!(!paths.data_dir().join(files::MIND_DRAFT).exists());
    assert!(!kept_file(&paths).exists());
}
