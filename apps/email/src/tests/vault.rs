//! Mail while the credential vault is shut. The service keeps each account's password in the vault
//! and cannot sign in to a mail server without it, so the app does not ask it to, says so in one
//! plain sentence, and asks again by itself until the vault is open.

use std::time::Duration;

use serde_json::json;
use yantrik_ipc_contracts::email::WAITING_FOR_VAULT;

use super::{notice, quiet, started, state, with_inbox};
use crate::backend::fake::FakeMail;
use crate::state::password_storage_note;

/// What `email.accounts` answers for one account, with the vault shut or not.
fn accounts(waiting: bool, plaintext: bool) -> serde_json::Value {
    json!({
        "accounts": [{
            "id": "acct-1", "email": "you@example.com", "display_name": "You",
            "provider": "gmail", "imap_server": "imap.gmail.com", "imap_port": 993,
            "smtp_server": "smtp.gmail.com", "smtp_port": 587,
        }],
        "config_path": "/home/p/.config/yantrik/email.json",
        "secrets_are_plaintext": plaintext,
        "waiting_for_vault": waiting,
        "google_sign_in": { "available": true, "note": "" },
    })
}

#[test]
fn mail_waits_for_the_vault_and_says_so_without_asking_the_mail_server() {
    let mail = with_inbox().answering("email.accounts", accounts(true, false));
    let app = started(&mail);

    assert_eq!(notice(&app), WAITING_FOR_VAULT);
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Waiting for the vault");
    assert!(mail.asked_for("email.list_folders").is_empty(), "no folder was asked for");
    assert!(mail.asked_for("email.list_messages").is_empty(), "no mail was checked");
    assert_eq!(state(&app)["waiting_for_vault"], true, "describe says why nothing is coming");
    assert_eq!(state(&app)["has_account"], true, "the account is there; it is the vault that is shut");
}

#[test]
fn mail_resumes_by_itself_when_the_vault_opens() {
    let mail = with_inbox().answering("email.accounts", accounts(true, false));
    let app = started(&mail);
    assert!(mail.asked_for("email.list_messages").is_empty());

    // Still shut a few seconds later: it asks again, and still asks the mail server nothing.
    app.advance(Duration::from_secs(5));
    quiet(&app);
    assert_eq!(mail.asked_for("email.accounts").len(), 2, "it looked again by itself");
    assert!(mail.asked_for("email.list_messages").is_empty());
    assert_eq!(notice(&app), WAITING_FOR_VAULT);

    // The person unlocks: the next look finds the vault open and checks mail.
    mail.set("email.accounts", Ok(accounts(false, false)));
    app.advance(Duration::from_secs(5));
    quiet(&app);
    assert_eq!(mail.asked_for("email.list_messages").len(), 1, "mail was checked");
    assert_eq!(notice(&app), "", "the sentence is gone");
    assert_eq!(app.ui(|ui| ui.get_email_sync_status()), "Synced");
    assert_eq!(state(&app)["waiting_for_vault"], serde_json::Value::Null, "and so is the key in describe");

    // And it stopped asking: nothing more is scheduled for the vault.
    let asked = mail.asked_for("email.accounts").len();
    app.advance(Duration::from_secs(60));
    quiet(&app);
    assert_eq!(mail.asked_for("email.accounts").len(), asked, "the timer that asked while the vault was shut was stopped");
}

#[test]
fn what_was_on_screen_stays_while_mail_waits() {
    let mail = with_inbox();
    let app = started(&mail);
    let shown = app.model(|m| m.rows.len());
    assert!(shown > 0);

    mail.set("email.accounts", Ok(accounts(true, false)));
    app.ui(|ui| ui.invoke_sync_emails());
    quiet(&app);
    assert_eq!(app.model(|m| m.rows.len()), shown, "the mail already in hand is not blanked");
    assert_eq!(notice(&app), WAITING_FOR_VAULT);
}

#[test]
fn a_call_the_service_refuses_for_the_vault_says_the_same_sentence() {
    let mail = with_inbox().failing("email.list_messages", WAITING_FOR_VAULT);
    let app = started(&mail);
    assert!(notice(&app).contains(WAITING_FOR_VAULT), "{}", notice(&app));
}

#[test]
fn the_password_note_is_the_truth_for_each_case() {
    let path = "/home/p/.config/yantrik/email.json";
    let kept = password_storage_note(path, false);
    assert!(kept.contains("credential vault") && !kept.contains("clear text"), "{kept}");
    assert!(kept.contains(path));

    let waiting = password_storage_note(path, true);
    assert!(waiting.contains("clear text") && waiting.contains("moves into the credential vault"), "{waiting}");

    // On the setup screen, from the service's own answer.
    let none = FakeMail::no_account().answering(
        "email.accounts",
        json!({ "accounts": [], "config_path": path, "secrets_are_plaintext": false,
                "google_sign_in": { "available": true, "note": "" } }),
    );
    let app = started(&none);
    let note = app.ui(|ui| ui.get_setup_note()).to_string();
    assert!(note.contains("kept in the credential vault"), "{note}");
    assert!(!note.contains("stored in clear text"), "the old sentence is gone: {note}");
}
