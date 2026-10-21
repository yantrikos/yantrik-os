//! The mailbox: the first look, Sync, folders, search and the triage tabs, by button and by
//! intent, each answered from what the service said.

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;

use super::{act, inbox, listed, message, notice, quiet, start_with, started, state, with_inbox};
use crate::backend::fake::{FakeMail, UNREACHABLE};
use crate::command::{Command, Look};

#[test]
fn the_first_look_happens_in_a_job_and_the_window_waits_on_the_mailbox_not_the_form() {
    let mail = with_inbox();
    let (app, _) = start_with(&mail, FakeMind::new());
    // Nothing asked on the UI thread: the look is a job, and until it lands the window shows the
    // mailbox loading, never the setup form's claim that there is no account.
    assert!(mail.asked().is_empty());
    assert!(app.ui(|ui| ui.get_has_account() && ui.get_is_loading()));
    quiet(&app);
    let asked: Vec<String> = mail.asked().into_iter().map(|(m, _)| m).collect();
    assert_eq!(asked, ["email.accounts", "email.list_folders", "email.list_messages"]);
    assert_eq!(mail.asked_for("email.list_messages")[0], json!({ "account_id": "acct-1", "folder": "INBOX", "page": 1 }));
    assert!(app.ui(|ui| ui.get_has_account() && !ui.get_is_loading()));
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Synced", "the first look says only what it found");
    assert_eq!(app.ui(|ui| ui.get_account_name()), "you@example.com");
    assert_eq!(listed(&app).len(), 3);
    assert_eq!(app.ui(|ui| (ui.get_email_folder_unread(), ui.get_email_folder_total())), (2, 3));
}

#[test]
fn the_three_states_are_three_screens() {
    let down = started(&FakeMail::unreachable());
    assert!(down.ui(|ui| !ui.get_has_account()));
    assert_eq!(down.ui(|ui| ui.get_service_state()), "unreachable");
    assert_eq!(down.ui(|ui| ui.get_email_sync_status()), "Service unreachable");
    assert_eq!(notice(&down), format!("The mail service could not be reached: {UNREACHABLE}"));
    assert!(!down.ui(|ui| ui.get_google_available()));

    let empty = started(&FakeMail::no_account());
    assert!(empty.ui(|ui| !ui.get_has_account()));
    assert_eq!(empty.ui(|ui| ui.get_service_state()), "up");
    assert_eq!(empty.ui(|ui| ui.get_email_sync_status()), "No account");
    assert!(empty.ui(|ui| ui.get_setup_note()).contains("stored in clear text in /home/p/.config/yantrik/email.json"));
    assert!(empty.ui(|ui| ui.get_google_available()));

    let broken = started(&FakeMail::ready().failing("email.list_messages", "No Response: SELECT failed"));
    assert!(broken.ui(|ui| ui.get_has_account()));
    assert_eq!(broken.ui(|ui| ui.get_email_sync_status()), "Mailbox did not open");
    assert_eq!(notice(&broken), "The account is configured and the mailbox did not open: No Response: SELECT failed");
}

#[test]
fn sync_counts_what_came_and_does_not_stack() {
    let app = started(&with_inbox());
    app.run(Command::Look(Look::Sync));
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Syncing\u{2026}");
    app.run(Command::Look(Look::Sync));
    assert_eq!(app.jobs_waiting(), 1, "a second press while syncing starts nothing");
    quiet(&app);
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Synced \u{00b7} 3 in INBOX");
}

#[test]
fn a_folder_pressed_is_listed_in_a_job_and_a_failure_is_said() {
    let mail = with_inbox();
    let app = started(&mail);
    app.ui(|ui| ui.invoke_folder_clicked(1));
    assert!(app.ui(|ui| ui.get_is_loading()));
    assert_eq!(app.ui(|ui| ui.get_selected_folder_index()), 1);
    quiet(&app);
    assert!(!app.ui(|ui| ui.get_is_loading()));
    assert_eq!(state(&app)["folder"], "Archive");
    assert_eq!(mail.asked_for("email.list_messages").last().unwrap()["folder"], "Archive");
    // A folder that is not there is a press on nothing.
    app.ui(|ui| ui.invoke_folder_clicked(9));
    assert_eq!(app.jobs_waiting(), 0);

    mail.set("email.list_messages", Err("No Response: SELECT failed".into()));
    app.ui(|ui| ui.invoke_folder_clicked(0));
    quiet(&app);
    assert_eq!(state(&app)["folder"], "Archive", "the folder on screen is still the one that opened");
    assert_eq!(notice(&app), "Could not open INBOX: No Response: SELECT failed");
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Mailbox did not open");
}

#[test]
fn select_folder_answers_with_the_folder_its_messages_and_its_counts() {
    let app = started(&with_inbox());
    let answer = act(&app, "select_folder", json!({ "folder": "archive" })).unwrap();
    assert_eq!(answer, json!({ "folder": "Archive", "messages": 3, "counts": { "unread": 0, "total": 10 } }));
    let e = act(&app, "select_folder", json!({ "folder": "Spam" })).unwrap_err();
    assert_eq!(e, "no folder called \u{201c}spam\u{201d}; there is: INBOX, Archive, Receipts");
    let e = act(&app, "select_folder", json!({ "folder": "  " })).unwrap_err();
    assert!(e.contains("folder"), "{e}");
    // The service refusing the folder is the caller's answer, in the service's words.
    let mail = FakeMail::ready().answering("email.list_messages", inbox());
    let app = started(&mail);
    mail.set("email.list_messages", Err("No Response: SELECT failed".into()));
    let e = act(&app, "select_folder", json!({ "folder": "INBOX" })).unwrap_err();
    assert_eq!(e, "No Response: SELECT failed");
    assert_eq!(notice(&app), "Could not open INBOX: No Response: SELECT failed");
}

#[test]
fn search_runs_in_the_folder_on_screen_and_an_empty_one_shows_the_folder_again() {
    let mail = with_inbox().answering("email.search", json!([message("102", "INBOX", "Invoice R0093", "Billing <b@example.net>", true, true)]));
    let app = started(&mail);
    let answer = act(&app, "search", json!({ "query": "invoice" })).unwrap();
    assert_eq!(answer, json!({ "matched": 1, "subjects": ["Invoice R0093"] }));
    assert_eq!(mail.asked_for("email.search")[0], json!({ "account_id": "acct-1", "folder": "INBOX", "query": "invoice" }));
    assert_eq!(app.ui(|ui| ui.get_email_search_query()), "invoice", "the box shows what was searched");
    assert!(app.ui(|ui| ui.get_email_search_active()));
    assert_eq!(app.ui(|ui| ui.get_email_search_count()), 1);
    assert_eq!(state(&app)["search_query"], "invoice");

    // The box submitted empty: the folder comes back, and the search is off.
    app.ui(|ui| ui.invoke_search_emails("".into()));
    quiet(&app);
    assert!(!app.ui(|ui| ui.get_email_search_active()));
    assert_eq!(listed(&app).len(), 3);

    mail.set("email.search", Err("BAD SEARCH".into()));
    let e = act(&app, "search", json!({ "query": "x" })).unwrap_err();
    assert_eq!(e, "Could not search for \u{201c}x\u{201d}: BAD SEARCH");
    assert_eq!(notice(&app), e, "a search that could not run is said, not shown as no matches");
}

#[test]
fn search_with_no_account_is_refused_in_words() {
    let app = started(&FakeMail::no_account());
    let e = act(&app, "search", json!({ "query": "x" })).unwrap_err();
    assert_eq!(e, "Cannot search: no email account is configured.");
    assert_eq!(notice(&app), e);
}

#[test]
fn the_triage_tabs_filter_what_is_in_hand_without_asking_the_server() {
    let mail = with_inbox();
    let app = started(&mail);
    let asked = mail.asked().len();
    app.ui(|ui| ui.invoke_triage_filter(1));
    assert_eq!(listed(&app), [(0, "Lunch on Friday".to_string()), (2, "Kernel review notes".to_string())]);
    assert_eq!(state(&app)["triage"], "Unread");
    assert_eq!(state(&app)["listed"], 2);
    // The header is the folder's, not the tab's.
    assert_eq!(app.ui(|ui| (ui.get_email_folder_unread(), ui.get_email_folder_total())), (2, 3));
    app.ui(|ui| ui.invoke_triage_filter(2));
    assert_eq!(listed(&app), [(1, "Invoice R0093".to_string())]);
    app.ui(|ui| ui.invoke_triage_filter(7));
    assert_eq!(notice(&app), "There is no triage tab 7.");
    assert_eq!(mail.asked().len(), asked);
}
