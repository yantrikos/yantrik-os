//! `email_delete`, `email_move` and `email_archive` ask before the IMAP store and expunge (#1311).

use yantrik_companion_tools::asks_scenarios::{check_all, Scenario};

use super::mail;
use super::tests_support::{mail as sample, q};

fn long() -> String {
    "s".repeat(300)
}

#[test]
fn deleting_mail_asks_with_sender_subject_date_and_that_it_is_for_good() {
    check_all(&Scenario {
        action: "delete_for_good",
        grade: "sensitive",
        card: serde_json::json!({
            "tool": "email_delete", "account": "ada@example.com", "from": q("The Weekly <news@example.com>"),
            "subject": q("Offer; to: someone@else.example"), "date": "2026-11-12 09:30 UTC", "folder": q("INBOX"),
            "change": "deletes the message for good",
        }),
        call: &|variant, effect| {
            let subject = match variant {
                0 => "Offer; to: someone@else.example".to_string(),
                1 => "Another message".to_string(),
                _ => long(),
            };
            mail::delete(&sample(&subject), "ada@example.com", effect)
        },
    });
}

#[test]
fn moving_mail_asks_with_where_it_goes() {
    check_all(&Scenario {
        action: "change_item",
        grade: "standard",
        card: serde_json::json!({
            "tool": "email_move", "account": "ada@example.com", "from": q("The Weekly <news@example.com>"),
            "subject": q("Offer"), "date": "2026-11-12 09:30 UTC", "folder": q("INBOX"), "to_folder": q("Trash"),
        }),
        call: &|variant, effect| {
            let subject = match variant {
                0 => "Offer".to_string(),
                1 => "Another message".to_string(),
                // The card of a change that can be reversed cuts a value at sixty characters, so
                // a subject past that is not asked about.
                _ => "x".repeat(70),
            };
            mail::move_to("email_move", &sample(&subject), "ada@example.com", "Trash", effect)
        },
    });
}

#[test]
fn archiving_mail_names_its_own_tool_and_the_folder_it_goes_to() {
    check_all(&Scenario {
        action: "change_item",
        grade: "standard",
        card: serde_json::json!({
            "tool": "email_archive", "account": "ada@example.com", "from": q("The Weekly <news@example.com>"),
            "subject": q("Offer"), "date": "2026-11-12 09:30 UTC", "folder": q("INBOX"), "to_folder": q("[Gmail]/All Mail"),
        }),
        call: &|variant, effect| {
            let subject = match variant {
                0 => "Offer".to_string(),
                1 => "Another message".to_string(),
                _ => "x".repeat(70),
            };
            mail::move_to("email_archive", &sample(&subject), "ada@example.com", "[Gmail]/All Mail", effect)
        },
    });
}
