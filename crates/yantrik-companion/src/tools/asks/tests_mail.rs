//! `email_delete`, `email_move` and `email_archive` ask before the IMAP store and expunge (#1311),
//! holding no session while the card waits and checking, after the Allow, that the message is the
//! one that was shown.

use std::cell::Cell;

use yantrik_companion_tools::asks_scenarios::{check_all, Scenario};
use yantrik_companion_tools::fake_shell::{Answer, FakeShell, First};

use super::mail::{self, Door, Seen};
use super::tests_support::{mail as sample, q};
use crate::email::db::CachedEmail;

/// A mailbox that counts what it is asked, and says what the message is "now".
struct Mailbox {
    now: Option<Seen>,
    reachable: bool,
    opened: Cell<usize>,
    closed: Cell<usize>,
    deleted: Cell<usize>,
    moved: Cell<usize>,
}

impl Mailbox {
    /// A mailbox where the shown message is still there, as it was.
    fn holding(mail: &CachedEmail) -> Mailbox {
        Mailbox::with(Some(Seen { subject: mail.subject.clone(), message_id: mail.message_id.clone(), uid_validity: Some(7) }))
    }

    fn with(now: Option<Seen>) -> Mailbox {
        Mailbox { now, reachable: true, opened: Cell::new(0), closed: Cell::new(0), deleted: Cell::new(0), moved: Cell::new(0) }
    }
}

impl Door for Mailbox {
    type S = ();
    fn open(&self) -> Result<(), String> {
        self.opened.set(self.opened.get() + 1);
        if self.reachable { Ok(()) } else { Err("connection refused".into()) }
    }
    fn seen(&self, _: &mut (), _: &str, _: u32) -> Result<Option<Seen>, String> {
        Ok(self.now.clone())
    }
    fn delete(&self, _: &mut (), _: &str, _: u32) -> Result<(), String> {
        self.deleted.set(self.deleted.get() + 1);
        Ok(())
    }
    fn move_to(&self, _: &mut (), _: &str, _: u32, _: &str) -> Result<(), String> {
        self.moved.set(self.moved.get() + 1);
        Ok(())
    }
    fn close(&self, _: ()) {
        self.closed.set(self.closed.get() + 1);
    }
}

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
            let item = sample(&subject);
            let door = Mailbox::holding(&item);
            let done = mail::delete(&item, "ada@example.com", Some(7), &door);
            match &done {
                Ok(()) => assert_eq!((door.opened.get(), door.deleted.get(), door.closed.get()), (1, 1, 1), "one new session, one delete, closed"),
                Err(_) => assert_eq!((door.opened.get(), door.deleted.get()), (0, 0), "nothing was opened or changed"),
            }
            done.and_then(|()| effect())
        },
    });
}

#[test]
fn moving_mail_asks_with_where_it_goes_even_for_a_long_subject() {
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
                _ => "x".repeat(300),
            };
            let item = sample(&subject);
            let door = Mailbox::holding(&item);
            let done = mail::move_to("email_move", &item, "ada@example.com", "Trash", Some(7), &door);
            assert_eq!(door.moved.get(), usize::from(done.is_ok()));
            done.and_then(|()| effect())
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
                _ => "x".repeat(300),
            };
            let item = sample(&subject);
            let door = Mailbox::holding(&item);
            let done = mail::move_to("email_archive", &item, "ada@example.com", "[Gmail]/All Mail", Some(7), &door);
            assert_eq!(door.moved.get(), usize::from(done.is_ok()));
            done.and_then(|()| effect())
        },
    });
}

/// A mail subject of sixty-one characters is moved: the question is drawn whole.
#[test]
fn a_subject_of_sixty_one_characters_is_asked_about_and_moved() {
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let item = sample(&"s".repeat(61));
    let door = Mailbox::holding(&item);
    mail::move_to("email_move", &item, "ada@example.com", "Archive", Some(7), &door).expect("asked, allowed and moved");
    assert_eq!((shell.cards().len(), door.moved.get()), (1, 1));
}

/// The Allow ends in the change or in a plain sentence, never in a held session that timed out:
/// the message is looked at again, after the Allow, on a session opened for it.
#[test]
fn a_message_that_changed_or_went_while_the_person_was_asked_is_said_plainly_and_left_alone() {
    let item = sample("Offer");
    let cases = [
        (Some(Seen { subject: "Something else".into(), message_id: String::new(), uid_validity: Some(7) }), "no longer the one that was shown"),
        (Some(Seen { subject: "Offer".into(), message_id: "<other@example.com>".into(), uid_validity: Some(7) }), "no longer the one that was shown"),
        (None, "no longer in INBOX"),
    ];
    let mut with_id = item.clone();
    with_id.message_id = "<a@example.com>".into();
    for (now, said) in cases {
        let shell = FakeShell::session(First::Pending, Answer::Granted);
        let door = Mailbox::with(now);
        let err = mail::delete(&with_id, "ada@example.com", Some(7), &door).unwrap_err();
        assert!(err.contains(said) && err.contains("Nothing was changed"), "{err}");
        assert_eq!((door.deleted.get(), door.moved.get(), door.closed.get()), (0, 0, 1), "nothing done, session closed");
        assert_eq!(shell.cards().len(), 1, "the person was asked, and said yes");
    }
}

#[test]
fn a_mailbox_that_cannot_be_reached_after_the_allow_changes_nothing_and_says_so() {
    let _shell = FakeShell::session(First::Pending, Answer::Granted);
    let item = sample("Offer");
    let mut door = Mailbox::holding(&item);
    door.reachable = false;
    let err = mail::move_to("email_move", &item, "ada@example.com", "Trash", Some(7), &door).unwrap_err();
    assert!(err.contains("could not be reached again") && err.contains("Nothing was changed"), "{err}");
    assert_eq!(door.moved.get(), 0);
}

#[test]
fn nothing_is_opened_before_the_person_has_answered() {
    let _shell = FakeShell::session(First::Pending, Answer::Denied);
    let item = sample("Offer");
    let door = Mailbox::holding(&item);
    mail::delete(&item, "ada@example.com", Some(7), &door).unwrap_err();
    assert_eq!(door.opened.get(), 0, "no session is held, or even opened, while the card waits");
}

/// L1 (#1313): a folder that was renumbered while the card waited is not the folder that was
/// looked at, whatever the subject says: a uid is only good under its UIDVALIDITY.
#[test]
fn a_folder_renumbered_while_the_person_was_asked_is_left_alone() {
    let item = sample("Offer");
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let door = Mailbox::with(Some(Seen { subject: "Offer".into(), message_id: String::new(), uid_validity: Some(8) }));
    let err = mail::delete(&item, "ada@example.com", Some(7), &door).unwrap_err();
    assert!(err.contains("renumbered") && err.contains("Nothing was changed"), "{err}");
    assert_eq!((door.deleted.get(), door.moved.get(), door.closed.get()), (0, 0, 1));
    assert_eq!(shell.cards().len(), 1);
}

/// A folder whose UIDVALIDITY the lookup could not read is not asked about at all.
#[test]
fn a_folder_with_no_readable_identity_is_not_asked_about() {
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let item = sample("Offer");
    let door = Mailbox::holding(&item);
    let err = mail::move_to("email_move", &item, "ada@example.com", "Trash", None, &door).unwrap_err();
    assert!(err.contains("could not be read") && err.contains("Nothing was changed"), "{err}");
    assert_eq!((shell.cards().len(), door.opened.get(), door.moved.get()), (0, 0, 0));
}
