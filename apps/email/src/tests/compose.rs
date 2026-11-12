//! Writing: the composer, the draft that survives Cancel, sending, replies and forwards, and the
//! draft file where the framework's paths say it is.

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths};

use super::{act, notice, quiet, start_at, started, state, with_inbox};
use crate::command::Command;
use crate::files::{self, Files};
use crate::state::{self, Draft};
use crate::Email;

pub(super) fn draft(to: &str, body: &str) -> Draft {
    Draft { to: to.into(), subject: "Plans".into(), body: body.into(), ..Draft::default() }
}

pub(super) fn type_into_composer(app: &yantrik_testing::TestApp<Email>, d: &Draft) {
    let d = d.clone();
    app.ui(move |ui| {
        ui.set_compose_to(d.to.into());
        ui.set_compose_cc(d.cc.into());
        ui.set_compose_bcc(d.bcc.into());
        ui.set_compose_subject(d.subject.into());
        ui.set_compose_body(d.body.into());
        ui.invoke_edited();
    });
}

#[test]
fn cancel_keeps_the_unsent_message_and_compose_reopens_it() {
    let app = started(&with_inbox());
    let file = app.paths().data_dir().join(files::DRAFT);
    app.ui(|ui| ui.invoke_compose_new());
    assert!(app.ui(|ui| ui.get_is_composing()));
    type_into_composer(&app, &draft("ananya@example.com", "See you at noon."));
    assert_eq!(state(&app)["composing"], true);
    app.ui(|ui| ui.invoke_cancel_compose());
    assert!(!app.ui(|ui| ui.get_is_composing()));
    quiet(&app);
    assert_eq!(state::load_draft(&file).unwrap(), Some(draft("ananya@example.com", "See you at noon.")));
    assert_eq!(app.ui(|ui| ui.get_email_draft_status()), "Draft saved");
    assert_eq!(notice(&app), "The unsent message was kept as a draft; Compose reopens it.");

    type_into_composer(&app, &Draft::default());
    app.ui(|ui| ui.invoke_compose_new());
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "See you at noon.");
    assert_eq!(app.ui(|ui| ui.get_email_draft_status()), "Draft restored");

    // Cancelling an empty composer keeps nothing.
    type_into_composer(&app, &Draft::default());
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    assert!(!file.exists());
    app.ui(|ui| ui.invoke_compose_new());
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "");
}

#[test]
fn save_draft_writes_the_file_or_says_there_is_nothing_to_save() {
    let app = started(&with_inbox());
    app.ui(|ui| ui.invoke_compose_new());
    app.ui(|ui| ui.invoke_save_draft());
    assert_eq!(notice(&app), "There is nothing in the composer to save.");
    type_into_composer(&app, &draft("a@example.com", "one"));
    app.ui(|ui| ui.invoke_save_draft());
    // A second save while the first is writing waits for it, so the file ends as the last one.
    type_into_composer(&app, &draft("a@example.com", "two"));
    app.ui(|ui| ui.invoke_save_draft());
    assert_eq!(app.jobs_waiting(), 1, "one write at a time");
    quiet(&app);
    let file = app.paths().data_dir().join(files::DRAFT);
    assert_eq!(state::load_draft(&file).unwrap().unwrap().body, "two");
    assert_eq!(app.ui(|ui| ui.get_email_draft_status()), "Draft saved");
    assert!(app.ui(|ui| ui.get_is_composing()), "Save Draft leaves the composer open");
}

#[test]
fn a_draft_file_that_is_not_a_draft_is_said_when_compose_is_pressed() {
    let paths = Paths::temporary(Email::ID);
    std::fs::create_dir_all(paths.data_dir()).unwrap();
    std::fs::write(paths.data_dir().join(files::DRAFT), "not json").unwrap();
    let (app, _) = start_at(paths, &with_inbox(), FakeMind::new());
    quiet(&app);
    app.ui(|ui| ui.invoke_compose_new());
    assert!(notice(&app).starts_with("Could not read the saved draft: "), "{}", notice(&app));
}

#[test]
fn send_sends_once_closes_the_composer_and_removes_the_draft() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let app = started(&mail);
    let file = app.paths().data_dir().join(files::DRAFT);
    state::save_draft(&file, &draft("x@example.com", "old")).unwrap();
    app.ui(|ui| ui.invoke_compose_new());
    app.ui(|ui| {
        ui.invoke_send_email("a@example.com, b@example.com".into(), "".into(), "c@example.com".into(), "Plans".into(), "Noon?".into())
    });
    // A second press while the first is in flight sends nothing more.
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "Noon?".into()));
    quiet(&app);
    assert_eq!(
        mail.asked_for("email.send_message"),
        [json!({
            "account_id": "acct-1", "to": ["a@example.com", "b@example.com"], "cc": [], "bcc": ["c@example.com"],
            "subject": "Plans", "body": "Noon?",
        })]
    );
    assert!(!app.ui(|ui| ui.get_is_composing()));
    assert!(!file.exists(), "the draft was this message");
    assert_eq!(mail.asked_for("email.list_messages").len(), 2, "the folder is read again");

    mail.set("email.send_message", Err("554 rejected".into()));
    app.ui(|ui| ui.invoke_compose_new());
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "Noon?".into()));
    quiet(&app);
    assert!(app.ui(|ui| ui.get_is_composing()), "the composer stays open, holding what was typed");
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Not sent");
    assert_eq!(notice(&app), "Could not send the message: 554 rejected");
}

#[test]
fn sending_with_no_account_is_refused_in_words() {
    let app = started(&crate::backend::fake::FakeMail::no_account());
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "s".into(), "b".into()));
    assert_eq!(app.jobs_waiting(), 0);
    assert_eq!(notice(&app), "Cannot send: no email account is configured.");
}

#[test]
fn compose_fills_a_draft_and_never_sends() {
    let mail = with_inbox();
    let app = started(&mail);
    let answer = act(&app, "compose", json!({ "to": "mom@example.com", "subject": "Sunday", "body": "Lunch?" })).unwrap();
    assert_eq!(answer, json!({ "drafted": true, "note": "waiting for the user to send it" }));
    assert!(app.ui(|ui| ui.get_is_composing()));
    assert_eq!(app.ui(|ui| ui.get_compose_to()), "mom@example.com");
    assert_eq!(app.ui(|ui| ui.get_compose_subject()), "Sunday");
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "Lunch?");
    assert!(mail.asked_for("email.send_message").is_empty());
}

#[test]
fn reply_quotes_the_open_message_and_forward_carries_it() {
    let app = started(&with_inbox());
    app.run(Command::Reply { all: false });
    assert!(!app.ui(|ui| ui.get_is_composing()), "nothing open, nothing to reply to");
    act(&app, "open_message", json!({ "which": 1 })).unwrap();
    app.ui(|ui| ui.invoke_reply_all_email());
    assert_eq!(app.ui(|ui| ui.get_compose_to()), "Priya R. <priya@example.com>");
    assert_eq!(app.ui(|ui| ui.get_compose_cc()), "team@example.com");
    assert_eq!(app.ui(|ui| ui.get_compose_subject()), "Re: Lunch on Friday");
    assert_eq!(
        app.ui(|ui| ui.get_compose_body()),
        "\n\nOn 2026-10-20 10:35, Priya R. wrote:\n> The text of Lunch on Friday.\n> Second line."
    );
    app.ui(|ui| ui.invoke_forward_email());
    assert_eq!(app.ui(|ui| ui.get_compose_to()), "");
    assert_eq!(app.ui(|ui| ui.get_compose_subject()), "Fwd: Lunch on Friday");
    assert!(app.ui(|ui| ui.get_compose_body()).starts_with("\n\n---------- Forwarded message ----------\nFrom: Priya R. <Priya R. <priya@example.com>>"));
}

/// The composer's text is the window's while the person types: a command that has nothing to do
/// with it never redraws it.
#[test]
fn what_is_typed_is_never_overwritten_by_an_unrelated_redraw() {
    let app = started(&with_inbox());
    app.ui(|ui| ui.invoke_compose_new());
    app.ui(|ui| ui.set_compose_body("half a sentence".into()));
    app.ui(|ui| ui.invoke_triage_filter(1));
    app.run(Command::Look(crate::command::Look::Sync));
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "half a sentence");
}

#[test]
fn a_draft_from_before_the_framework_is_brought_across_once_and_kept() {
    let root = std::env::temp_dir().join(format!("email-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let paths = Paths::under(&root, Email::ID, yantrik_app::paths::DEFAULT_PROFILE);
    let files = Files::for_app(&paths);
    let legacy = root.join("home/.local/share/yantrik/email/draft.json");
    state::save_draft(&legacy, &draft("old@example.com", "from before")).unwrap();

    files::import_legacy_draft(&paths, &files, Some(&legacy));
    assert_eq!(state::load_draft(&files.draft).unwrap(), Some(draft("old@example.com", "from before")));
    assert!(!legacy.exists());
    let kept = root.join("home/.local/share/yantrik/email/draft.json.legacy.json");
    assert!(kept.exists(), "the old file is kept beside itself, not removed");

    // Once: a draft of this build's own is never replaced by an older one.
    state::save_draft(&legacy, &draft("older@example.com", "even older")).unwrap();
    state::save_draft(&files.draft, &draft("new@example.com", "mine")).unwrap();
    files::import_legacy_draft(&paths, &files, Some(&legacy));
    assert_eq!(state::load_draft(&files.draft).unwrap().unwrap().body, "mine");
    assert!(legacy.exists(), "left alone");

    // Another profile is a new place: nothing of the default's is brought into it.
    let other = Paths::under(&root.join("p2"), Email::ID, "work");
    let other_files = Files::for_app(&other);
    files::import_legacy_draft(&other, &other_files, Some(&legacy));
    assert!(!other_files.draft.exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// Review M1 (#920): a send in flight is shown, a second press says so, and what is typed while
/// the first one is out is kept, never closed away with it.
#[test]
fn a_send_in_flight_is_shown_and_an_edit_made_meanwhile_is_kept() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let app = started(&mail);
    let file = app.paths().data_dir().join(files::DRAFT);
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "draft v1"));
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "draft v1".into()));
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Sending\u{2026}");
    assert!(app.ui(|ui| ui.get_is_sending()), "Send says a send is in flight");

    type_into_composer(&app, &draft("a@example.com", "draft v2 with the fix"));
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "draft v2 with the fix".into()));
    assert_eq!(notice(&app), "A message is already being sent; wait for it before sending again.");
    quiet(&app);

    let bodies: Vec<String> =
        mail.asked_for("email.send_message").iter().map(|p| p["body"].as_str().unwrap_or_default().to_string()).collect();
    assert_eq!(bodies, ["draft v1"], "one send, of what was there when Send was pressed");
    assert!(!app.ui(|ui| ui.get_is_sending()));
    assert!(app.ui(|ui| ui.get_is_composing()), "the composer stays open with the edit");
    assert_eq!(app.ui(|ui| ui.get_compose_body()), "draft v2 with the fix");
    assert_eq!(state::load_draft(&file).unwrap().map(|d| d.body), Some("draft v2 with the fix".to_string()));
    assert_eq!(
        notice(&app),
        "The message was sent as it was when Send was pressed. What was changed since is still in the composer, kept as a draft."
    );
}

/// Review L1 (#920): a send that failed or never came back leaves its text where Compose finds
/// it.
#[test]
fn a_send_that_does_not_come_back_leaves_its_text_in_compose() {
    for mail in [
        with_inbox().failing("email.send_message", "554 rejected"),
        with_inbox().panicking("email.send_message"),
    ] {
        let app = started(&mail);
        let file = app.paths().data_dir().join(files::DRAFT);
        app.ui(|ui| ui.invoke_compose_new());
        type_into_composer(&app, &draft("a@example.com", "the only copy"));
        app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "the only copy".into()));
        quiet(&app);
        assert_eq!(state::load_draft(&file).unwrap().map(|d| d.body), Some("the only copy".to_string()));
        type_into_composer(&app, &Draft::default());
        app.ui(|ui| ui.invoke_compose_new());
        assert_eq!(app.ui(|ui| ui.get_compose_body()), "the only copy");
    }
}

/// Review L1 (#920): a send still waiting for a worker when the window closes is gone with the
/// process, so its text is offered again at the next start, with what is and is not known.
#[test]
fn a_send_still_queued_when_the_window_closes_is_offered_again_at_the_next_start() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "queued when it closed"));
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "queued when it closed".into()));
    let _ = app.shutdown();
    drop(app);
    assert!(mail.asked_for("email.send_message").is_empty(), "the send never ran");

    let (again, _) = start_at(paths, &mail, FakeMind::new());
    quiet(&again);
    assert_eq!(
        notice(&again),
        "A message was being sent when Email closed, and whether it went is not known. It is kept as a draft: look in Sent before sending it again."
    );
    again.ui(|ui| ui.invoke_compose_new());
    assert_eq!(again.ui(|ui| ui.get_compose_body()), "queued when it closed");
}

/// The draft is private mail: written whole, and readable only by its owner.
#[cfg(unix)]
#[test]
fn the_draft_is_written_whole_and_only_for_its_owner() {
    use std::os::unix::fs::PermissionsExt as _;
    let app = started(&with_inbox());
    let file = app.paths().data_dir().join(files::DRAFT);
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "private"));
    app.ui(|ui| ui.invoke_save_draft());
    quiet(&app);
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!file.with_extension("json.part").exists(), "nothing is left beside it");
}

/// Sends `body` from a fresh composer, as the window does.
fn send_from_composer(app: &yantrik_testing::TestApp<Email>, body: &str) {
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(app, &draft("a@example.com", body));
    let body = body.to_string();
    app.ui(move |ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), body.into()));
}

/// Review L5 (#920): a draft whose message may already have gone says so every time Compose
/// opens it, across restarts, until it is sent or discarded; a listing in between does not take
/// the warning away.
#[test]
fn a_draft_whose_send_may_have_gone_says_so_each_time_compose_opens_it() {
    let unsure = "Whether this message was already sent is not known. Look in Sent before sending it again.";
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    send_from_composer(&app, "maybe went");
    let _ = app.shutdown();
    drop(app);

    let (again, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&again);
    again.ui(|ui| ui.invoke_folder_clicked(1));
    quiet(&again);
    again.ui(|ui| ui.invoke_compose_new());
    assert_eq!(again.ui(|ui| ui.get_email_draft_status()), "Not known whether this was sent");
    assert_eq!(notice(&again), unsure);
    again.ui(|ui| ui.invoke_cancel_compose());
    quiet(&again);
    let _ = again.shutdown();
    drop(again);

    let (third, _) = start_at(paths, &mail, FakeMind::new());
    quiet(&third);
    third.ui(|ui| ui.invoke_compose_new());
    assert_eq!(third.ui(|ui| ui.get_compose_body()), "maybe went");
    assert_eq!(third.ui(|ui| ui.get_email_draft_status()), "Not known whether this was sent", "still, after a restart");
    // Sent at last: the warning goes with the draft.
    third.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "maybe went".into()));
    quiet(&third);
    third.ui(|ui| ui.invoke_compose_new());
    assert_eq!(third.ui(|ui| ui.get_compose_body()), "");
    assert_eq!(third.ui(|ui| ui.get_email_draft_status()), "");
}

/// Review L5 (#920): a first look that does not land still offers the unsent message, and says
/// what is not known about it.
#[test]
fn an_unsent_message_is_offered_even_when_the_first_look_does_not_land() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    send_from_composer(&app, "maybe went");
    let _ = app.shutdown();
    drop(app);
    let (again, _) = start_at(paths, &with_inbox().panicking("email.list_messages"), FakeMind::new());
    quiet(&again);
    assert!(notice(&again).ends_with("look in Sent before sending it again."), "{}", notice(&again));
}

/// Review L6 (#920): Cancel with an edit while a send is out never calls the kept edit "the
/// unsent message" over a message that went, nor says it is still in a composer that is closed.
#[test]
fn cancel_with_an_edit_during_a_send_says_what_went_and_where_the_edit_is() {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let app = started(&mail);
    let file = app.paths().data_dir().join(files::DRAFT);
    send_from_composer(&app, "v1");
    type_into_composer(&app, &draft("a@example.com", "v2 edit"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    let bodies: Vec<String> =
        mail.asked_for("email.send_message").iter().map(|p| p["body"].as_str().unwrap_or_default().to_string()).collect();
    assert_eq!(bodies, ["v1"]);
    assert!(!app.ui(|ui| ui.get_is_composing()));
    assert_eq!(state::load_draft(&file).unwrap().map(|d| d.body), Some("v2 edit".to_string()));
    assert_eq!(
        notice(&app),
        "The message was sent as it was when Send was pressed. What was changed since is kept as a draft; Compose reopens it."
    );
}

/// Review L7 (#920): when the draft file cannot be written, the lane copy of a send that never
/// came back stays, so the text is still there at the next start.
#[test]
fn the_lane_copy_stays_until_the_draft_file_holding_its_text_has_landed() {
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), &with_inbox().panicking("email.send_message"), FakeMind::new());
    quiet(&app);
    // A folder stands where the draft file goes, so it cannot be written.
    std::fs::create_dir_all(paths.data_dir().join(files::DRAFT).join("in-the-way")).unwrap();
    send_from_composer(&app, "the only copy");
    quiet(&app);
    assert!(notice(&app).starts_with("Could not keep the unsent message: "), "{}", notice(&app));
    let _ = app.shutdown();
    drop(app);

    let (again, _) = start_at(paths, &with_inbox(), FakeMind::new());
    quiet(&again);
    again.ui(|ui| ui.invoke_compose_new());
    assert_eq!(again.ui(|ui| ui.get_compose_body()), "the only copy");
}
