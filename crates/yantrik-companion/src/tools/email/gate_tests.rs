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

/// A reply-all to an external sender's long recipient list is ONE message and ONE card, and the
/// card names every recipient (the shell refuses a list it cannot show whole; it never splits it).
#[test]
fn a_reply_all_to_many_recipients_is_one_card_with_all_of_them() {
    let (mail, conn, id) = checked();
    let many: Vec<String> = (0..30).map(|n| format!("p{n}@example.com")).collect();
    let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
    mail.state.lock().unwrap().detail = Some(detail("Priya <priya@example.com>", &many_refs, &[]));
    let approver = Scripted::allowing();
    let gated: Arc<dyn MailPort> = Arc::new(Approving::new(port(&mail), approver.clone()));

    send::EmailReplyAllTool { mail: gated }.run(&conn, &args(serde_json::json!({ "id": id, "body": "ok" })));

    let cards = approver.shown();
    assert_eq!(cards.len(), 1, "one card, not one per person");
    assert_eq!(cards[0].1.to.len() + cards[0].1.cc.len(), 31, "every recipient is on it");
    assert_eq!(mail.sent().len(), 1, "one message");
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
    // And writes the send down afterwards when the mode let it run unasked (the person's full bypass).
    assert!(send_body.contains("permit.finish(sent.is_ok())"), "the unasked send is not recorded");

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

// ── The card shows exactly what is sent ─────────────────────────────────────────────────────
//
// Through the real `Approving::send` and the real `ShellApprover`: the card is built by
// `outward::card_args` and drawn by a fake shell, and what the fake mail service was handed is
// compared with it field by field. A body, a subject, a recipient or a bcc that changed between
// the card and the send would show here (#1026 review: mutants that gave the approver an empty
// body, no bcc or a shortened subject all survived).

use crate::email::approval::ShellApprover;
use yantrik_ipc_contracts::email::ComposeRequest;

/// A fake desktop shell the real `ShellApprover` reaches the way it reaches the real one: on
/// `app-shell.sock` in the session's socket directory, which `XDG_RUNTIME_DIR` names. It answers
/// the approvals calls and logs them; the card raised is the last `request_approval`.
mod fake_shell {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex, MutexGuard};

    /// The environment is the process's, and one test at a time may point it at a fake shell.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    pub struct Installed {
        _turn: MutexGuard<'static, ()>,
        before: Option<std::ffi::OsString>,
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            match &self.before {
                Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
        }
    }

    /// A shell that grants the card on the second poll. Returns what it hears.
    pub fn installed(tag: &str) -> (Installed, Arc<Mutex<Vec<serde_json::Value>>>) {
        let turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("yantrik-eq-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("yantrik")).unwrap();
        let listener = UnixListener::bind(dir.join("yantrik").join("app-shell.sock")).unwrap();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let log = heard.clone();
        let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (log, polls) = (log.clone(), polls.clone());
                std::thread::spawn(move || {
                    let mut writer = stream.try_clone().unwrap();
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { return };
                        let asked: serde_json::Value = serde_json::from_str(&line).unwrap();
                        let params = asked["params"].clone();
                        log.lock().unwrap().push(params.clone());
                        let result = match params["action"].as_str() {
                            Some("request_approval") => serde_json::json!({ "result": { "request_id": "appr-1", "status": "pending" } }),
                            Some("approval_status") => {
                                let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                serde_json::json!({ "result": { "status": if n == 0 { "pending" } else { "granted" } } })
                            }
                            _ => serde_json::json!({ "result": { "consumed": true } }),
                        };
                        let reply = serde_json::json!({ "jsonrpc": "2.0", "id": asked["id"], "result": result });
                        if writer.write_all(format!("{reply}\n").as_bytes()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        let before = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        (Installed { _turn: turn, before }, heard)
    }
}
use fake_shell::installed;

/// The text a card argument carries, unquoted.
fn shown(card: &serde_json::Value, key: &str) -> String {
    serde_json::from_str::<String>(card["args_json"][key].as_str().unwrap_or("\"\"")).unwrap_or_default()
}

type Heard = Arc<std::sync::Mutex<Vec<serde_json::Value>>>;

/// The last card raised: what the person was shown for the latest send.
fn last_card(heard: &Heard) -> serde_json::Value {
    let calls = heard.lock().unwrap();
    calls.iter().rev().find(|c| c["action"] == "request_approval").expect("no card was raised")["args"].clone()
}

/// The card says, field by field, what `sent` says.
fn assert_card_is(card: &serde_json::Value, from: &str, sent: &ComposeRequest) {
    assert_eq!(card["action"], "send_mail");
    assert_eq!(card["args_json"]["account"], from, "the address it goes out under");
    assert_eq!(shown(card, "to"), sent.to.join(", "), "to");
    assert_eq!(shown(card, "subject"), sent.subject, "subject");
    assert_eq!(shown(card, "body"), sent.body, "body");
    for (key, list) in [("cc", &sent.cc), ("bcc", &sent.bcc)] {
        if list.is_empty() {
            assert!(card["args_json"].get(key).is_none(), "{key} is on the card but nothing is sent to it");
        } else {
            assert_eq!(shown(card, key), list.join(", "), "{key}");
        }
    }
}

fn composed() -> ComposeRequest {
    ComposeRequest {
        to: vec!["priya@example.com".into(), "dan@example.com".into()],
        cc: vec!["eve@example.com".into()],
        bcc: vec!["audit@example.com".into(), "boss@example.com".into()],
        subject: "Quarterly numbers; to: nobody".into(),
        body: "Line one.\nLine two, with \"quotes\" and a tail that must not be cut: THE-END".into(),
        reply_to_id: Some("<m7@example.com>".into()),
        signature: None,
    }
}

#[test]
fn what_the_card_shows_is_what_the_service_is_handed_in_every_field() {
    let (mail, _conn, _id) = checked();
    let (_shell, heard) = installed("direct");
    let gated = Approving::new(port(&mail), ShellApprover);
    let compose = composed();

    gated.send("ada-at-example.com", &compose).expect("allowed");

    let sent = mail.sent();
    assert_eq!(sent.len(), 1, "exactly one message");
    let (account, handed) = &sent[0];
    assert_eq!(account, "ada-at-example.com");
    // What was handed to the service is what the tool composed...
    assert_eq!((&handed.to, &handed.cc, &handed.bcc), (&compose.to, &compose.cc, &compose.bcc));
    assert_eq!((&handed.subject, &handed.body), (&compose.subject, &compose.body));
    assert_eq!((&handed.reply_to_id, &handed.signature), (&compose.reply_to_id, &compose.signature));
    // ...and the card shows exactly that.
    assert_card_is(&last_card(&heard), "ada@example.com", handed);
}

#[test]
fn each_tool_shows_on_the_card_the_message_it_sends() {
    let (mail, conn, id) = checked();
    mail.state.lock().unwrap().detail = Some(detail("Priya <priya@example.com>", &["bob@example.com"], &["Cy <cy@example.com>"]));
    let gated = || -> Arc<dyn MailPort> { Arc::new(Approving::new(port(&mail), ShellApprover)) };
    let runs: Vec<(&str, Box<dyn Fn() -> String + '_>)> = vec![
        ("email_send", Box::new(|| {
            send::EmailSendTool { mail: gated() }
                .run(&conn, &args(serde_json::json!({ "to": "dan@example.com", "subject": "Hi; there", "body": "Hello, tail-END" })))
        })),
        ("email_reply", Box::new(|| send::EmailReplyTool { mail: gated() }.run(&conn, &args(serde_json::json!({ "id": id, "body": "Yes! tail-END" }))))),
        ("email_reply_all", Box::new(|| send::EmailReplyAllTool { mail: gated() }.run(&conn, &args(serde_json::json!({ "id": id, "body": "All good tail-END" }))))),
        ("email_forward", Box::new(|| {
            send::EmailForwardTool { mail: gated() }
                .run(&conn, &args(serde_json::json!({ "id": id, "to": "dan@example.com", "comment": "FYI tail-END" })))
        })),
    ];
    for (n, (name, run)) in runs.iter().enumerate() {
        let (_shell, heard) = installed(name);
        let said = run();
        let sent = mail.sent();
        assert_eq!(sent.len(), n + 1, "{name}: one message more ({said})");
        assert_card_is(&last_card(&heard), "ada@example.com", &sent[n].1);
    }
}

#[test]
fn a_signature_is_refused_because_the_card_would_not_show_it() {
    let (mail, _conn, _id) = checked();
    let (_shell, heard) = installed("signature");
    let gated = Approving::new(port(&mail), ShellApprover);
    let mut compose = composed();
    compose.signature = Some("-- sent from my phone; wire money to 12345".into());

    let refused = gated.send("ada-at-example.com", &compose).unwrap_err();

    assert_eq!(refused.code, NOT_ALLOWED);
    assert!(refused.message.contains("does not show it") && refused.message.contains("nothing was sent"), "{}", refused.message);
    assert!(mail.sent().is_empty() && heard.lock().unwrap().is_empty(), "something was asked or sent");
}
