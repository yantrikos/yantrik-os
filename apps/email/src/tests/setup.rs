//! Account setup and the Google sign-in. The password is followed through every path: it goes to
//! the service in the call that needs it, and nowhere else the app can show, keep or log.

use serde_json::json;

use super::{act, notice, quiet, started, state};
use crate::backend::fake::FakeMail;
use crate::command::{Command, SetupForm};
use crate::update::google::POLL_EVERY;

/// The password used below. Nothing this app shows, keeps or describes may contain it.
const SENTINEL: &str = "hunter2-SENTINEL-do-not-print";

fn form() -> SetupForm {
    SetupForm {
        email: "you@example.com".into(),
        password: SENTINEL.into(),
        display_name: "You".into(),
        provider: "gmail".into(),
        imap_port: "993".into(),
        smtp_port: "587".into(),
        ..SetupForm::default()
    }
}

/// Everything the app can say or keep where a person or a mind could read it.
fn everything_said(app: &yantrik_testing::TestApp<crate::Email>) -> String {
    let model = app.model(|m| {
        format!("{} | {} | {} | {:?} | {}", m.notice, m.setup.status, m.sync_status, m.composer, m.answer)
    });
    format!("{model} | {} | {}", app.describe(), app.ui(|ui| ui.get_setup_status()))
}

#[test]
fn the_password_reaches_the_service_and_nothing_else() {
    let summary = json!({
        "id": "acct-1", "email": "you@example.com", "imap_server": "imap.gmail.com", "imap_port": 993,
        "smtp_server": "smtp.gmail.com", "smtp_port": 587,
    });
    let mail = FakeMail::no_account()
        .answering("email.test_account", json!({ "imap_ok": true, "imap": "IMAP ok", "smtp_ok": false, "smtp": "SMTP refused" }))
        .answering("email.save_account", summary);
    let app = started(&mail);
    app.ui(|ui| ui.set_setup_password(SENTINEL.into()));
    app.run(Command::TestConnection(form()));
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "Signing in\u{2026}");
    assert!(app.ui(|ui| ui.get_setup_testing()));
    // Pressed again while the test is in flight: nothing more is asked.
    app.run(Command::SaveAccount(form()));
    quiet(&app);
    assert_eq!(mail.asked_for("email.test_account").len(), 1);
    assert!(mail.asked_for("email.save_account").is_empty());
    assert_eq!(mail.asked_for("email.test_account")[0]["password"], SENTINEL, "the service is given it");
    assert_eq!(mail.asked_for("email.test_account")[0]["imap_server"], "imap.gmail.com");
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "Mail can be read but not sent. SMTP refused");
    assert!(!app.ui(|ui| ui.get_setup_ok()));
    assert!(!everything_said(&app).contains(SENTINEL));

    app.run(Command::SaveAccount(form()));
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "Signed in as you@example.com");
    assert_eq!(app.ui(|ui| ui.get_setup_password()), "", "it leaves the window once it is not needed");
    assert!(!app.ui(|ui| ui.get_setup_open()));
    assert_eq!(mail.asked_for("email.accounts").len(), 2, "a sync follows");
    assert!(!everything_said(&app).contains(SENTINEL));
    // A form's `Debug`, the path a log line or a panic would take, does not carry it either.
    assert!(!format!("{:?}", Command::SaveAccount(form())).contains(SENTINEL));
}

#[test]
fn a_form_that_cannot_work_is_refused_on_the_form_without_quoting_the_password() {
    let mail = FakeMail::no_account().failing("email.save_account", "authentication failed for <redacted>");
    let app = started(&mail);
    app.run(Command::SaveAccount(SetupForm { email: "nobody".into(), ..form() }));
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "\u{201c}nobody\u{201d} is not an email address \u{2014} it has no @ in it.");
    assert!(mail.asked_for("email.save_account").is_empty());
    assert_eq!(notice(&app), "", "said on the form, where the person is looking");

    app.ui(|ui| ui.set_setup_password(SENTINEL.into()));
    app.run(Command::SaveAccount(form()));
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "authentication failed for <redacted>");
    assert_eq!(app.ui(|ui| ui.get_setup_password()), SENTINEL, "the form stays up, holding what was typed");
    assert!(!everything_said(&app).contains(SENTINEL));
}

#[test]
fn cancel_and_add_account_clear_the_form() {
    let app = started(&FakeMail::no_account());
    app.ui(|ui| {
        ui.set_setup_email("you@example.com".into());
        ui.set_setup_password(SENTINEL.into());
        ui.set_setup_provider("gmail".into());
    });
    app.ui(|ui| ui.invoke_cancel_setup());
    assert_eq!(app.ui(|ui| ui.get_setup_password()), "");
    assert_eq!(app.ui(|ui| ui.get_setup_email()), "you@example.com", "Cancel clears the password only");
    app.ui(|ui| ui.invoke_add_account());
    assert!(app.ui(|ui| ui.get_setup_open()));
    assert_eq!(app.ui(|ui| (ui.get_setup_email(), ui.get_setup_provider())), ("".into(), "".into()));
}

fn google() -> FakeMail {
    FakeMail::no_account()
        .answering("email.oauth_begin", json!({ "flow_id": "f1", "auth_url": "https://accounts.example/consent", "expires_in_secs": 3 }))
        .answering("email.oauth_status", json!({ "state": "waiting" }))
        .answering("email.oauth_cancel", json!({}))
}

#[test]
fn the_google_sign_in_opens_the_browser_polls_and_lands() {
    let mail = google();
    let app = started(&mail);
    let answer = act(&app, "begin_google_sign_in", json!({})).unwrap();
    assert_eq!(answer["opened"], true);
    assert_eq!(mail.opened(), ["https://accounts.example/consent"]);
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "Waiting for Google\u{2026}");
    assert!(app.ui(|ui| ui.get_google_waiting()));
    app.advance(POLL_EVERY);
    quiet(&app);
    assert_eq!(mail.asked_for("email.oauth_status"), [json!({ "flow_id": "f1" })]);
    // A second start while one is waiting is refused, to a caller in words.
    let e = act(&app, "begin_google_sign_in", json!({})).unwrap_err();
    assert_eq!(e, "A sign-in is already in progress.");

    mail.set(
        "email.oauth_status",
        Ok(json!({ "state": "done", "account": {
            "id": "g", "email": "someone@gmail.com", "imap_server": "imap.gmail.com", "imap_port": 993,
            "smtp_server": "smtp.gmail.com", "smtp_port": 587, "uses_oauth": true,
        } })),
    );
    app.advance(POLL_EVERY);
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "Signed in as someone@gmail.com");
    assert!(!app.ui(|ui| ui.get_google_waiting()));
    assert_eq!(app.timers_armed(), 0, "the poll stops once it has an answer");
    assert_eq!(mail.asked_for("email.accounts").len(), 2, "a sync follows");
}

#[test]
fn a_sign_in_not_finished_in_time_or_cancelled_stops_and_says_so() {
    let mail = google();
    let app = started(&mail);
    app.ui(|ui| ui.invoke_sign_in_with_google());
    quiet(&app);
    for _ in 0..30 {
        app.advance(POLL_EVERY);
        quiet(&app);
    }
    assert_eq!(
        notice(&app),
        "The Google sign-in was not finished in time, so this window stopped waiting for it. Nothing was saved."
    );
    assert_eq!(app.timers_armed(), 0);
    let polls = mail.asked_for("email.oauth_status").len();
    assert_eq!(polls as u64, 3 + 20 - 1, "a little past the service's own deadline");

    app.ui(|ui| ui.invoke_sign_in_with_google());
    quiet(&app);
    app.ui(|ui| ui.invoke_cancel_google_sign_in());
    quiet(&app);
    assert_eq!(mail.asked_for("email.oauth_cancel"), [json!({ "flow_id": "f1" })]);
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "The Google sign-in was cancelled.");
    // An answer for the flow that was cancelled is dropped.
    app.run(Command::GoogleStatus { flow: "f1".into(), outcome: crate::state::GoogleOutcome::SignedIn("late@gmail.com".into()) });
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "The Google sign-in was cancelled.");
}

#[test]
fn no_browser_puts_the_address_on_the_screen_and_no_client_is_said() {
    let app = started(&google().no_browser("xdg-open exited 3"));
    app.ui(|ui| ui.invoke_sign_in_with_google());
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_google_url()), "https://accounts.example/consent");
    assert_eq!(notice(&app), crate::state::browser_failed_note("xdg-open exited 3"));

    let unavailable = FakeMail::default().answering(
        "email.accounts",
        json!({ "accounts": [], "config_path": "/c", "secrets_are_plaintext": true,
                "google_sign_in": { "available": false, "note": "This build has no Google client id." } }),
    );
    let app = started(&unavailable);
    app.ui(|ui| ui.invoke_sign_in_with_google());
    assert_eq!(app.ui(|ui| ui.get_setup_status()), "This build has no Google client id.");
    assert_eq!(notice(&app), "This build has no Google client id.");
    assert_eq!(act(&app, "begin_google_sign_in", json!({})).unwrap_err(), "This build has no Google client id.");
    assert_eq!(state(&app)["has_account"], false);
}

/// Review L3 (#920): Cancel pressed before the sign-in's start has landed ends the flow that
/// start opened, instead of leaving it polling with its listener open.
#[test]
fn cancel_before_the_google_begin_lands_ends_the_flow_it_started() {
    let mail = google();
    let app = started(&mail);
    app.ui(|ui| ui.invoke_sign_in_with_google());
    app.ui(|ui| ui.invoke_cancel_google_sign_in());
    quiet(&app);
    assert_eq!(mail.asked_for("email.oauth_cancel"), [json!({ "flow_id": "f1" })]);
    assert!(!app.ui(|ui| ui.get_google_waiting()));
    assert!(app.model(|m| m.google.flow.is_none()));
    assert_eq!(app.timers_armed(), 0, "nothing polls");
}
