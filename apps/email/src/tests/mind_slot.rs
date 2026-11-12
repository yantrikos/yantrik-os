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
    again.ui(|ui| ui.invoke_open_prepared());
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

// ── Round 3 (review of 40fd3b3e): the prepared draft is its own entry, and the person's ──

fn slot_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir().join(files::MIND_DRAFT)
}

fn with_commitment(attach: &str, commitment: &str) -> serde_json::Value {
    let mut args = compose_args(attach);
    args["commitment"] = json!(commitment);
    args
}

/// The review's probe: the executor composes the Maya draft with the deck, the person presses
/// Cancel, then Compose, and writes to Bob.
#[test]
fn plain_compose_is_blank_and_the_persons_mail_to_bob_goes_while_mayas_draft_waits() {
    let file = HomeFile::new("probe");
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox().answering("email.send_message", json!({}));
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    act(&app, "compose", compose_args(&file.path())).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);

    app.ui(|ui| ui.invoke_compose_new());
    assert_eq!(app.ui(|ui| (ui.get_compose_to().to_string(), ui.get_compose_body().to_string())), (String::new(), String::new()), "blank");
    assert!(!app.ui(|ui| ui.get_composing_prepared()));
    assert_eq!(state(&app)["draft_attachments"], json!([]), "Maya's deck is not on Bob's mail");
    type_into_composer(&app, &draft("bob@example.com", "Lunch?"));
    app.ui(|ui| ui.invoke_send_email("bob@example.com".into(), "".into(), "".into(), "Plans".into(), "Lunch?".into()));
    quiet(&app);
    let sent = mail.asked_for("email.send_message");
    assert_eq!(sent.len(), 1, "Bob's mail went");
    assert!(sent[0].to_string().contains("bob@example.com") && !sent[0].to_string().contains("Q4-deck"), "{sent:?}");

    // Maya's draft is untouched, still carries its file, and is reached by its own entry.
    assert_eq!(app.ui(|ui| ui.get_prepared_label().to_string()), "Draft Yantrik prepared for maya@example.com");
    assert!(slot_file(&paths).exists(), "slot");
    assert!(!kept_file(&paths).exists(), "Bob's draft is cleared once it went");
    // The person has not touched it, so a later Mind compose is not refused with a false sentence.
    assert!(act(&app, "compose", compose_args("")).is_ok());
}

#[test]
fn the_prepared_entry_opens_it_with_its_file_shown() {
    let file = HomeFile::new("entry");
    let app = started_at(&Paths::temporary(Email::ID));
    act(&app, "compose", compose_args(&file.path())).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert!(!app.ui(|ui| ui.get_is_composing()));
    app.ui(|ui| ui.invoke_open_prepared());
    assert!(app.ui(|ui| ui.get_is_composing() && ui.get_composing_prepared()));
    assert_eq!(app.ui(|ui| ui.get_prepared_file().to_string()), "Q4-deck.ydeck");
    assert_eq!(app.ui(|ui| ui.get_compose_body().to_string()), "Here it is.");
}

#[test]
fn discard_prepared_draft_removes_the_slot_and_leaves_the_persons_draft() {
    let file = HomeFile::new("discard");
    let paths = Paths::temporary(Email::ID);
    let kept = kept_with_cc(&paths);
    let app = started_at(&paths);
    // The person's own draft is in the way, so the Mind's compose is refused; clear it, then prepare.
    state::clear_draft(&kept_file(&paths)).unwrap();
    let app = { drop(app); started_at(&paths) };
    act(&app, "compose", compose_args(&file.path())).unwrap();
    app.ui(|ui| ui.invoke_discard_prepared());
    quiet(&app);
    assert!(!slot_file(&paths).exists());
    assert!(!app.ui(|ui| ui.get_is_composing()) && app.ui(|ui| ui.get_prepared_label().to_string()).is_empty());
    assert_eq!(state(&app)["draft_attachments"], json!([]));
    let _ = kept;
}

#[test]
fn remove_attachment_then_send_sends_the_persons_own_mail_and_clears_both() {
    let file = HomeFile::new("remove");
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox().answering("email.send_message", json!({}));
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    act(&app, "compose", compose_args(&file.path())).unwrap();
    app.ui(|ui| ui.invoke_remove_attachment());
    quiet(&app);
    assert!(!slot_file(&paths).exists(), "no longer a prepared draft");
    assert_eq!(state(&app)["draft_attachments"], json!([]));
    assert!(kept_file(&paths).exists(), "what is in the composer is the person's own draft now");
    // What they write after removing the file is kept as theirs.
    type_into_composer(&app, &draft("maya@example.com", "Here it is, without the deck."));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert_eq!(state::load_draft(&kept_file(&paths)).unwrap().unwrap().body, "Here it is, without the deck.");
    app.ui(|ui| ui.invoke_compose_new());
    app.ui(|ui| ui.invoke_send_email("maya@example.com".into(), "".into(), "".into(), "Plans".into(), "Here it is, without the deck.".into()));
    quiet(&app);
    assert_eq!(mail.asked_for("email.send_message").len(), 1, "it can be sent now");
    assert!(!kept_file(&paths).exists() && !slot_file(&paths).exists());
}

#[test]
fn remove_attachment_is_refused_over_a_draft_of_the_persons_own() {
    let file = HomeFile::new("remove-refused");
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", compose_args(&file.path())).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    kept_with_cc(&paths);
    let app = { drop(app); started_at(&paths) };
    app.ui(|ui| ui.invoke_open_prepared());
    app.ui(|ui| ui.invoke_remove_attachment());
    quiet(&app);
    assert!(notice(&app).starts_with("You have an unsent message of your own"), "{}", notice(&app));
    assert!(slot_file(&paths).exists(), "nothing was overwritten or lost");
    assert_eq!(state::load_draft(&kept_file(&paths)).unwrap().unwrap().body, "half written");
}

#[test]
fn cancel_on_an_emptied_slot_discards_it_and_the_refusal_is_not_said_after() {
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", compose_args("")).unwrap();
    type_into_composer(&app, &Draft::default());
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert!(!slot_file(&paths).exists(), "nothing left to keep");
    assert!(app.ui(|ui| ui.get_prepared_label().to_string()).is_empty());
    assert!(act(&app, "compose", compose_args("")).is_ok(), "no false 'you have an unsent message'");
}

#[test]
fn a_save_still_in_flight_is_the_persons_text() {
    let app = started_at(&Paths::temporary(Email::ID));
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "closing now"));
    app.ui(|ui| ui.invoke_cancel_compose());
    // The save has not landed: `kept` is still empty.
    assert!(app.model(|m| m.composer.saving || m.composer.writing));
    let refused = app.act("compose", compose_args("")).map(|p| p.settle());
    let said = match &refused { Ok(Err(e)) | Err(e) => e.clone(), Ok(Ok(v)) => v.to_string() };
    assert!(said.starts_with("You have an unsent message of your own"), "{refused:?}");
}

#[test]
fn forgetting_a_promise_clears_its_untouched_draft_and_only_its_own() {
    let file = HomeFile::new("forget");
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", with_commitment(&file.path(), "7")).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    let raw = std::fs::read_to_string(slot_file(&paths)).unwrap();
    assert!(raw.contains("\"commitment\": \"7\""), "the id is kept in mind_draft.json: {raw}");
    act(&app, "forget_prepared", json!({ "commitment": "8" })).unwrap();
    assert!(slot_file(&paths).exists(), "another promise's forgetting leaves it");
    act(&app, "forget_prepared", json!({ "commitment": "7" })).unwrap();
    assert!(!slot_file(&paths).exists());
    assert!(app.ui(|ui| ui.get_prepared_label().to_string()).is_empty());

    // One the person changed is theirs now, and stays.
    act(&app, "compose", with_commitment(&file.path(), "9")).unwrap();
    type_into_composer(&app, &draft("maya@example.com", "my own wording"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "9" })).unwrap();
    assert!(slot_file(&paths).exists());
}

#[test]
fn discard_and_remove_are_the_persons_buttons_and_no_intent() {
    let mut table = yantrik_app::intents::Intents::<Email>::new();
    <Email as App>::intents(&mut table);
    let names: Vec<String> = table.specs().iter().map(|s| s.name.clone()).collect();
    for name in &names {
        assert!(!name.contains("discard") && !name.contains("remove") && !name.contains("open_prepared"), "{name}");
    }
    // And a Mind's window refuses them if it is ever pressed.
    for c in [crate::Command::OpenPrepared, crate::Command::DiscardPrepared, crate::Command::RemoveAttachment] {
        assert!(crate::mind_view::refuses(&c), "{c:?}");
    }
}

// ── MEDIUM-D: a draft needs no account, so no mail look ──

fn service_start(paths: &Paths, mail: &crate::backend::fake::FakeMail) -> yantrik_testing::TestApp<Email> {
    crate::backend::fake::install(paths, mail);
    yantrik_testing::TestApp::<Email>::start_service_at(paths.clone())
}

#[test]
fn an_email_started_only_to_compose_looks_at_no_mail() {
    let file = HomeFile::new("service");
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox();
    let app = service_start(&paths, &mail);
    quiet(&app);
    act(&app, "compose", compose_args(&file.path())).unwrap();
    app.advance(std::time::Duration::from_secs(60));
    quiet(&app);
    assert!(mail.asked().is_empty(), "no Look was queued and no mail service was asked: {:?}", mail.asked());
    assert_eq!(app.model(|m| m.composer.attachments.len()), 1, "the draft was made");
}

/// P3: an owed clear alone is no reason to open a mail session, however long Email stays up.
#[test]
fn an_agent_start_with_only_forget_prepared_makes_no_mail_call() {
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox();
    let app = service_start(&paths, &mail);
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "7" })).unwrap();
    app.advance(std::time::Duration::from_secs(60));
    quiet(&app);
    assert!(mail.asked().is_empty(), "no mail call: {:?}", mail.asked());
}

/// P4: there is no grace timer to lose a race to; a compose that arrives late is as quiet as one
/// that arrives at once.
#[test]
fn a_compose_after_ten_seconds_makes_no_mail_call() {
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox();
    let app = service_start(&paths, &mail);
    quiet(&app);
    app.advance(std::time::Duration::from_secs(10));
    act(&app, "compose", compose_args("")).unwrap();
    app.advance(std::time::Duration::from_secs(60));
    quiet(&app);
    assert!(mail.asked().is_empty(), "{:?}", mail.asked());
}

/// LOW-1: in an Email started for an agent nobody has the draft open, so the clear is paid at once
/// and is not refused with a reason that is not true.
#[test]
fn an_agent_start_compose_then_forget_prepared_clears_it_and_never_says_someone_has_it_open() {
    let file = HomeFile::new("low1");
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox();
    let app = service_start(&paths, &mail);
    quiet(&app);
    act(&app, "compose", with_commitment(&file.path(), "7")).unwrap();
    assert!(slot_file(&paths).exists(), "the agent's draft is kept");
    act(&app, "forget_prepared", json!({ "commitment": "7" })).unwrap();
    assert!(!slot_file(&paths).exists(), "the slot is gone");
    assert!(mail.asked().is_empty(), "{:?}", mail.asked());
}

/// P1: the person is typing in the prepared draft when the clear arrives.
#[test]
fn forget_prepared_while_the_person_types_in_it_refuses_and_their_text_survives() {
    let file = HomeFile::new("p1");
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", with_commitment(&file.path(), "7")).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    app.ui(|ui| ui.invoke_open_prepared());
    type_into_composer(&app, &draft("maya@example.com", "my own careful wording, half done"));
    let said = act(&app, "forget_prepared", json!({ "commitment": "7" })).unwrap_err();
    assert!(said.to_string().to_lowercase().contains("try again"), "kept owed by classify: {said}");
    assert!(app.model(|m| m.composer.open && m.composer.in_slot), "the window was not closed");
    assert_eq!(app.model(|m| m.composer.text.body.clone()), "my own careful wording, half done");
    assert!(slot_file(&paths).exists(), "the slot is kept");
    // And it is theirs now: a later forget, with the window closed, leaves it.
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "7" })).unwrap();
    assert!(slot_file(&paths).exists());
    assert!(std::fs::read_to_string(slot_file(&paths)).unwrap().contains("my own careful wording"));
}

#[test]
fn forget_prepared_while_the_prepared_view_is_open_but_unedited_waits_and_keeps_the_window() {
    let file = HomeFile::new("open");
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    act(&app, "compose", with_commitment(&file.path(), "7")).unwrap();
    assert!(act(&app, "forget_prepared", json!({ "commitment": "7" })).is_err());
    assert!(app.model(|m| m.composer.open && m.composer.in_slot), "the window is still open under the person");
    assert!(slot_file(&paths).exists());
    // Closed unedited, the clear is paid.
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "7" })).unwrap();
    assert!(!slot_file(&paths).exists());
}

#[test]
fn the_first_intent_that_needs_the_mailbox_looks_at_once_after_a_compose() {
    let paths = Paths::temporary(Email::ID);
    let mail = with_inbox();
    let app = service_start(&paths, &mail);
    quiet(&app);
    act(&app, "compose", compose_args("")).unwrap();
    assert!(mail.asked().is_empty());
    let _ = act(&app, "select_folder", json!({ "folder": "INBOX" }));
    assert!(!mail.asked().is_empty());
}

#[test]
fn the_persons_start_looks_at_once_as_it_always_did() {
    let mail = with_inbox();
    let (app, _) = start_at(Paths::temporary(Email::ID), &mail, FakeMind::new());
    quiet(&app);
    assert!(!mail.asked().is_empty());
}
