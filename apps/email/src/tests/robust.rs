//! What the framework's gates ask of every port: a worker that panics, a stale result, an agent
//! without authority, and a shutdown while work is in flight.

use std::time::Duration;

use serde_json::json;
use yantrik_testing::Call;

use super::{act, inbox, notice, quiet, started, state, with_inbox};
use crate::backend::fake::FakeMail;

#[test]
fn a_panicking_worker_brings_its_busy_flag_down_and_says_so() {
    let app = started(&with_inbox().panicking("email.list_messages"));
    // The look is not left running, and the window says so.
    assert!(!app.model(|m| m.syncing));
    assert_eq!(notice(&app), "The mailbox could not be read; try Sync again.");

    let app = started(&with_inbox().panicking("email.search"));
    assert!(act(&app, "search", json!({ "query": "x" })).is_err(), "the caller is answered");
    assert!(!app.ui(|ui| ui.get_is_loading()));
    assert_eq!(notice(&app), "The folder could not be read; try again.");

    // A send that panicked may or may not have gone; the sentence says exactly that, and the
    // composer can send again.
    let app = started(&with_inbox().panicking("email.send_message"));
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "s".into(), "b".into()));
    quiet(&app);
    assert!(notice(&app).starts_with("The send did not finish, so this window cannot say whether the message went."));
    assert!(app.model(|m| m.composer.sending.is_none()));
}

#[test]
fn a_listing_replaced_by_a_newer_one_is_dropped() {
    let mail = with_inbox();
    let app = started(&mail);
    app.ui(|ui| ui.invoke_folder_clicked(1));
    app.ui(|ui| ui.invoke_folder_clicked(0));
    quiet(&app);
    assert_eq!(state(&app)["folder"], "INBOX", "the last folder pressed is the one shown");
    let asked: Vec<String> = mail
        .asked_for("email.list_messages")
        .iter()
        .map(|p| p["folder"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(asked, ["INBOX", "INBOX"], "the replaced listing never reached the server");
    assert_eq!(state(&app)["listed"], inbox().as_array().unwrap().len());
}

#[test]
fn an_agent_needs_a_grant_for_the_sensitive_sign_in_and_not_for_reading() {
    let mail = with_inbox();
    let app = started(&mail);
    let refused = app
        .act_as(Call::new("begin_google_sign_in", json!({})).as_agent("mind").in_mode("ask"))
        .is_err();
    assert!(refused, "a sensitive intent needs a person's grant in ask mode");
    assert!(mail.asked_for("email.oauth_begin").is_empty());
    assert!(app.act_as(Call::new("select_folder", json!({ "folder": "Archive" })).as_agent("mind").in_mode("ask")).is_ok());
}

#[test]
fn a_shutdown_while_a_listing_is_owed_answers_it_as_a_closing() {
    let app = started(&with_inbox());
    let owed = app.act("select_folder", json!({ "folder": "Archive" })).unwrap();
    assert!(owed.is_owed());
    let began = std::time::Instant::now();
    let _clean = app.shutdown();
    assert!(began.elapsed() < Duration::from_secs(2), "shutdown must not hang on the jobs");
    assert_eq!(owed.settle().unwrap_err(), "the app is closing");
}

#[test]
fn a_service_that_never_answers_a_caller_is_refused_in_its_words() {
    let app = started(&FakeMail::unreachable());
    let e = act(&app, "select_folder", json!({ "folder": "INBOX" })).unwrap_err();
    assert_eq!(e, "no folder called \u{201c}inbox\u{201d}; no folders are listed");
}
