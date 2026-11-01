//! No mail leaves without the person's say (#1026, 520 gate: three replies went out with no card,
//! in `auto` and in `ask`). The shell decides whether a card is raised and in which mode
//! (`control_mail`, which pins auto, ask and plan); these pin that every tool that sends asks it,
//! that a no sends nothing, that a yes sends exactly once, and that a new way to send cannot be
//! added without being asked about.

use std::sync::Arc;

use super::tests::{args, checked, detail, port, world};
use super::send;
use crate::email::approval::{Approving, Scripted, NOT_ALLOWED};
use crate::email::fake::FakeMail;
use crate::email::port::MailPort;
use crate::tools::Tool;

/// The four tools that send, each with the arguments that make it send once, over a fake mail
/// service behind the approver `approver`. Returns what the tool said.
fn run_each(mail: &Arc<FakeMail>, approver: &Arc<Scripted>, conn: &rusqlite::Connection, id: i64) -> Vec<(&'static str, String)> {
    let gated = || -> Arc<dyn MailPort> { Arc::new(Approving::new(port(mail), approver.clone())) };
    vec![
        (
            "email_send",
            send::EmailSendTool { mail: gated() }
                .run(conn, &args(serde_json::json!({ "to": "dan@example.com", "subject": "Hi", "body": "Hello" }))),
        ),
        ("email_reply", send::EmailReplyTool { mail: gated() }.run(conn, &args(serde_json::json!({ "id": id, "body": "Yes!" })))),
        (
            "email_reply_all",
            send::EmailReplyAllTool { mail: gated() }.run(conn, &args(serde_json::json!({ "id": id, "body": "All good" }))),
        ),
        (
            "email_forward",
            send::EmailForwardTool { mail: gated() }
                .run(conn, &args(serde_json::json!({ "id": id, "to": "dan@example.com", "comment": "FYI" }))),
        ),
    ]
}

#[test]
fn send_reply_reply_all_and_forward_each_raise_a_card_and_a_no_sends_nothing() {
    let (mail, conn, id) = checked();
    mail.state.lock().unwrap().detail = Some(detail("Priya <priya@example.com>", &["bob@example.com"], &[]));
    let approver = Scripted::refusing("The person at the machine said no to sending this email. Nothing was sent.");

    let said = run_each(&mail, &approver, &conn, id);

    assert_eq!(approver.shown().len(), 4, "one card per tool");
    assert!(mail.sent().is_empty(), "the fake SMTP got {} message(s) after a no", mail.sent().len());
    for (tool, said) in said {
        assert!(said.contains("failed") && said.contains("Nothing was sent"), "{tool}: {said}");
    }
    // The card carries what would have left, whole where it matters: the address it goes out under
    // and every recipient.
    let cards = approver.shown();
    assert!(cards.iter().all(|(from, _)| from == "ada@example.com"));
    assert_eq!(cards[0].1.to, vec!["dan@example.com"]);
    assert_eq!(cards[1].1.to, vec!["priya@example.com"]);
}

#[test]
fn an_allow_sends_each_exactly_once() {
    let (mail, conn, id) = checked();
    mail.state.lock().unwrap().detail = Some(detail("Priya <priya@example.com>", &["bob@example.com"], &[]));
    let approver = Scripted::allowing();

    run_each(&mail, &approver, &conn, id);

    assert_eq!(approver.shown().len(), 4);
    assert_eq!(mail.sent().len(), 4, "one message per tool, once each");
}

#[test]
fn a_refusal_carries_its_own_code_so_a_no_is_not_mistaken_for_a_failure() {
    let (mail, _conn, _id) = checked();
    let gated = Approving::new(port(&mail), Scripted::refusing("no"));
    let compose = yantrik_ipc_contracts::email::ComposeRequest {
        to: vec!["a@example.com".into()],
        cc: vec![],
        bcc: vec![],
        subject: "s".into(),
        body: "b".into(),
        reply_to_id: None,
        signature: None,
    };
    let refused = gated.send("ada-at-example.com", &compose).unwrap_err();
    assert_eq!(refused.code, NOT_ALLOWED);
    assert!(mail.sent().is_empty());
}

/// A new outward mail tool must be gated. Every port method is either known to stay on the
/// person's side or is the one send, and the port the tools are registered with is the approving
/// one. A method added to `MailPort` that is in neither list fails this until someone decides, in
/// this file, which it is: the gate cannot be forgotten by adding a tool next to the others.
#[test]
fn a_new_way_to_send_cannot_be_added_without_being_asked_about() {
    const STAYS_ON_THE_PERSONS_SIDE: [&str; 10] = [
        "accounts", "list_folders", "list_messages", "search", "get_message", "mark_read", "mark_starred", "move_message",
        "delete_message", "import_account",
    ];
    const OUTWARD: [&str; 1] = ["send"];

    let port_source = include_str!("../../email/port.rs");
    let declared = port_source.split("pub trait MailPort").nth(1).and_then(|t| t.split("\n}\n").next()).expect("the trait");
    let methods: Vec<&str> = declared
        .lines()
        .filter_map(|l| l.trim().strip_prefix("fn "))
        .map(|rest| rest.split(|c: char| c == '(' || c == '<').next().unwrap_or(""))
        .collect();
    assert!(methods.len() >= 11, "the scan stopped reading the trait: {methods:?}");
    for method in &methods {
        assert!(
            STAYS_ON_THE_PERSONS_SIDE.contains(method) || OUTWARD.contains(method),
            "MailPort::{method} is new. If it transmits mail or contacts anyone outside this machine it must ask the \
             person like `send` does (email/approval.rs); then list it here."
        );
    }

    // The wrapper asks before it delegates `send`, and no other method asks.
    let approval = include_str!("../../email/approval.rs");
    let wrapper = approval.split("impl<A: Approver> MailPort for Approving").nth(1).expect("the wrapper");
    let send_body = wrapper.split("fn send(").nth(1).and_then(|s| s.split("\n    }\n").next()).expect("send");
    let asked = send_body.find("allow_send(").expect("send asks");
    let delegated = send_body.find("self.inner.send(").expect("send delegates");
    assert!(asked < delegated, "send delegates before it asks");

    // Only send.rs puts mail on the wire, and the live port is registered behind the approver.
    for (file, text) in [
        ("read.rs", include_str!("read.rs")),
        ("change.rs", include_str!("change.rs")),
        ("support.rs", include_str!("support.rs")),
    ] {
        assert!(!text.contains(".send("), "{file} sends mail; outward mail belongs in send.rs, behind the approver");
    }
    let register = include_str!("mod.rs").split("pub fn register(").nth(1).and_then(|s| s.split("\n}\n").next()).expect("register");
    assert!(register.contains("Approving::new(Arc::new(ServiceMail::live()), ShellApprover)"), "{register}");
    assert!(!register.contains("register_with(reg, Arc::new(ServiceMail::live()))"), "the bare port is registered");
}
