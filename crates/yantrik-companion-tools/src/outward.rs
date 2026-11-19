//! Asking the person before mail leaves the machine (#1026, 520 gate).
//!
//! A send is outward and cannot be undone. The companion's mail tools used to send under their
//! own ceiling with nothing in front of the person: three replies went out on VM 520 in `auto`
//! and in `ask`. There is one approvals module, the shell's, and this is the companion's way into
//! it, the same `request_approval` / `approval_status` the other asking paths use
//! ([`crate::app_ui`]): raise the card for the shell's `send_mail` question (sensitive, "cannot be
//! undone", so every mode but full bypass shows it and plan refuses), wait for the person, spend
//! the grant with `consume_approval`, and only then let the send go. Nothing here decides: a mode
//! that runs the question unasked is the shell's answer, and an unreachable shell is a "no".
//!
//! The card shows the mail whole: every recipient, the whole subject and the whole body, each
//! quoted so that text inside one cannot pose as another field (`subject: "Invoice; to: x"` is one
//! value). What the person cannot read they cannot allow, so a mail the card cannot show in full
//! (the shell's own rule for actions that cannot be undone: a value is shown whole or not asked
//! about) is not asked about, and nothing is sent; the sentence says to leave it as a draft in the
//! Email app. The grant is bound to exactly these values.

use crate::app_ui::Desk;
use crate::asks::{ask_with, quoted, Spec};

pub use crate::asks::Permit;

/// The shell's question, and the grade it publishes it at.
const ACTION: &str = "send_mail";
const GRADE: &str = "sensitive";

/// What the card says is being asked, besides the shell's own published sentence.
const PURPOSE: &str = "Send an email from the person's account. Once sent it cannot be undone.";

/// One message about to leave.
pub struct OutgoingMail<'a> {
    /// The address it is sent from.
    pub account: &'a str,
    pub to: &'a [String],
    pub cc: &'a [String],
    pub bcc: &'a [String],
    pub subject: &'a str,
    pub body: &'a str,
}

/// The arguments the card shows and the grant is bound to: all of the mail, nothing clipped.
pub fn card_args(mail: &OutgoingMail<'_>) -> serde_json::Value {
    let mut args = serde_json::json!({
        "account": mail.account,
        "to": quoted(&mail.to.join(", ")),
        "subject": quoted(mail.subject),
        "body": quoted(mail.body),
    });
    for (key, list) in [("cc", mail.cc), ("bcc", mail.bcc)] {
        if !list.is_empty() {
            args[key] = quoted(&list.join(", "));
        }
    }
    args
}

/// The sentence for a mail the card cannot show in full.
const TOO_LONG: &str = concat!(
    "This email is too long to put in front of the person in full, and what they cannot read they cannot allow, ",
    "so it was not asked about. Nothing was sent. Shorten it, or leave it as a draft in the Email app ",
    "(its compose action) for the person to read and send themselves."
);

/// Ask the person, on this desktop's own card, whether `mail` may go. `Ok` only when they allowed
/// exactly this (or the shell says its mode asks nobody); `Err` is a sentence for the model and
/// nothing was sent.
pub fn ask_before_sending(desk: &Desk, mail: &OutgoingMail<'_>) -> Result<Permit, String> {
    let spec = Spec {
        action: ACTION,
        grade: GRADE,
        purpose: PURPOSE,
        thing: "this email",
        doing: "sending this email",
        needs: "sending",
        to_do: "send",
        nothing: "nothing was sent",
        too_long: TOO_LONG.to_string(),
    };
    ask_with(desk, &spec, &card_args(mail))
}

/// [`ask_before_sending`] on the session's own desk.
pub fn ask_person_before_sending(mail: &OutgoingMail<'_>) -> Result<Permit, String> {
    ask_before_sending(&Desk::session(), mail)
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    /// A shell on a socket: a card that is answered `answer` on the second poll (or needs no card
    /// at all when `first` is `not_needed`), and a log of every call.
    fn shell(tag: &str, first: &'static str, answer: &'static str) -> (Desk, Arc<Mutex<Vec<serde_json::Value>>>) {
        let dir = std::env::temp_dir().join(format!("yantrik-outward-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let listener = UnixListener::bind(dir.join("app-shell.sock")).unwrap();
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
                            Some("request_approval") if first == "not_needed" => {
                                serde_json::json!({"result": {"status": "not_needed", "mode": "bypass_all"}})
                            }
                            Some("request_approval") if first == "no_id" => serde_json::json!({"result": {"status": "pending"}}),
                            Some("request_approval") if first == "too_long" => serde_json::json!({"error": {"code": -32602,
                                "message": "this request is too long to put in front of a person in full, so nothing was asked: `body` is 900 characters"}}),
                            Some("request_approval") => serde_json::json!({"result": {"request_id": "appr-1", "status": "pending"}}),
                            Some("approval_status") => {
                                let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                serde_json::json!({"result": {"status": if n == 0 { "pending" } else { answer }}})
                            }
                            Some("consume_approval") => serde_json::json!({"result": {"consumed": true}}),
                            Some("record_unasked_action") => serde_json::json!({"result": {"recorded": true}}),
                            _ => serde_json::json!({"error": {"code": -32602, "message": "unknown"}}),
                        };
                        let reply = if result.get("error").is_some() {
                            serde_json::json!({"jsonrpc": "2.0", "id": asked["id"], "error": result["error"]})
                        } else {
                            serde_json::json!({"jsonrpc": "2.0", "id": asked["id"], "result": result})
                        };
                        if writer.write_all(format!("{reply}\n").as_bytes()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (Desk { dir: Some(dir), wait: Duration::from_millis(600), poll: Duration::from_millis(20) }, heard)
    }

    fn mail<'a>(to: &'a [String], body: &'a str) -> OutgoingMail<'a> {
        OutgoingMail { account: "ada@example.com", to, cc: &[], bcc: &[], subject: "Hello", body }
    }

    fn actions(heard: &Arc<Mutex<Vec<serde_json::Value>>>) -> Vec<String> {
        heard.lock().unwrap().iter().map(|p| p["action"].as_str().unwrap_or("").to_string()).collect()
    }

    #[test]
    fn an_allow_is_asked_for_the_exact_mail_and_spent_once() {
        let (desk, heard) = shell("allow", "pending", "granted");
        let to = vec!["priya@example.com".to_string()];
        ask_before_sending(&desk, &mail(&to, "Thanks!")).expect("allowed");

        let calls = heard.lock().unwrap().clone();
        assert_eq!(calls[0]["action"], "request_approval");
        assert_eq!(calls[0]["args"]["app"], "shell");
        assert_eq!(calls[0]["args"]["action"], "send_mail");
        assert_eq!(calls[0]["args"]["grade"], "sensitive");
        assert_eq!(calls[0]["args"]["args_json"]["to"], "\"priya@example.com\"");
        let spent: Vec<_> = calls.iter().filter(|c| c["action"] == "consume_approval").collect();
        assert_eq!(spent.len(), 1, "{calls:?}");
        assert_eq!(spent[0]["args"]["args_json"], calls[0]["args"]["args_json"], "the Allow is spent on what was shown");
    }

    #[test]
    fn a_no_or_silence_sends_nothing_and_spends_nothing() {
        for (tag, answer, word) in [("denied", "denied", "said no"), ("silent", "pending", "Nobody answered")] {
            let (desk, heard) = shell(tag, "pending", answer);
            let to = vec!["priya@example.com".to_string()];
            let err = ask_before_sending(&desk, &mail(&to, "Thanks!")).unwrap_err();
            assert!(err.contains(word) && err.to_lowercase().contains("nothing was sent"), "{err}");
            assert!(!actions(&heard).contains(&"consume_approval".to_string()));
        }
    }

    #[test]
    fn a_shell_that_cannot_be_reached_is_a_no() {
        let desk = Desk {
            dir: Some(std::env::temp_dir().join("yantrik-outward-nobody-home")),
            wait: Duration::from_millis(100),
            poll: Duration::from_millis(10),
        };
        let to = vec!["priya@example.com".to_string()];
        let err = ask_before_sending(&desk, &mail(&to, "Thanks!")).unwrap_err();
        assert!(err.contains("Nothing was sent"), "{err}");
    }

    /// The card shows the whole mail: no clipping, no digest standing in for text.
    #[test]
    fn the_card_carries_the_whole_subject_and_body_and_every_recipient() {
        let to = vec!["priya@example.com".to_string(), "dan@example.com".to_string()];
        let cc = vec!["eve@example.com".to_string()];
        let subject = "s".repeat(200);
        let body = format!("start {} the very end", "x".repeat(150));
        let mail = OutgoingMail { account: "ada@example.com", to: &to, cc: &cc, bcc: &[], subject: &subject, body: &body };
        let args = card_args(&mail);
        let back = |key: &str| serde_json::from_str::<String>(args[key].as_str().unwrap()).unwrap();
        assert_eq!(back("subject"), subject);
        assert_eq!(back("body"), body, "the end of the body is on the card");
        assert_eq!(back("to"), "priya@example.com, dan@example.com");
        assert_eq!(back("cc"), "eve@example.com");
        assert!(args.get("bcc").is_none());
        assert!(args.get("body_digest").is_none() && args.get("body_start").is_none());
    }

    /// The card draws `key: value; key: value`. Text that looks like a field must stay inside its
    /// value: quoted, with its own quotes escaped.
    #[test]
    fn a_spoofing_subject_or_body_cannot_pose_as_a_field() {
        let to = vec!["attacker@evil.example".to_string()];
        for hostile in [
            "Invoice; to: priya@example.com",
            "x\"; to: \"priya@example.com",
            "ok\nto: priya@example.com",
            "a\\\"; to: \"priya@example.com",
        ] {
            let args = card_args(&OutgoingMail { account: "ada@example.com", to: &to, cc: &[], bcc: &[], subject: hostile, body: hostile });
            for key in ["subject", "body"] {
                let drawn = args[key].as_str().unwrap();
                assert!(drawn.starts_with('"') && drawn.ends_with('"'), "{key}: {drawn}");
                // It reads back as exactly one string, the hostile text: nothing ended the quote early.
                assert_eq!(serde_json::from_str::<String>(drawn).unwrap(), hostile, "{key}");
                // And inside the quotes every quote is escaped, so no `"` can close the value.
                let inner = &drawn[1..drawn.len() - 1];
                let mut escaped = false;
                for c in inner.chars() {
                    assert!(escaped || c != '"', "an unescaped quote in {key}: {drawn}");
                    escaped = !escaped && c == '\\';
                }
            }
            assert_eq!(args["to"], "\"attacker@evil.example\"", "the real recipient is its own value");
        }
    }

    #[test]
    fn a_reply_that_is_neither_unasked_nor_a_request_is_a_no() {
        let (desk, heard) = shell("noid", "no_id", "granted");
        let to = vec!["priya@example.com".to_string()];
        let err = ask_before_sending(&desk, &mail(&to, "Thanks!")).err().expect("no request id means no send");
        assert!(err.contains("did not say which request") && err.contains("nothing was sent"), "{err}");
        assert_eq!(actions(&heard), ["request_approval"], "nothing was waited on or spent");
    }

    #[test]
    fn an_allow_that_was_already_spent_is_a_no() {
        let (desk, heard) = shell("consumed", "pending", "consumed");
        let to = vec!["priya@example.com".to_string()];
        let err = ask_before_sending(&desk, &mail(&to, "Thanks!")).err().expect("a spent Allow sends nothing");
        assert!(err.contains("already been spent") && err.contains("nothing was sent"), "{err}");
        assert!(!actions(&heard).contains(&"consume_approval".to_string()));
    }

    #[test]
    fn a_mail_the_card_cannot_show_whole_is_not_asked_and_points_to_a_draft() {
        let (desk, heard) = shell("toolong", "too_long", "granted");
        let to = vec!["priya@example.com".to_string()];
        let err = ask_before_sending(&desk, &mail(&to, &"x".repeat(900))).err().expect("too long to show");
        assert!(err.contains("cannot read they cannot allow") && err.contains("Nothing was sent") && err.contains("draft"), "{err}");
        assert_eq!(actions(&heard), ["request_approval"]);
    }

    #[test]
    fn a_mode_that_asks_nobody_is_honoured_and_the_send_is_written_down_after() {
        let (desk, heard) = shell("unasked", "not_needed", "granted");
        let to = vec!["priya@example.com".to_string()];
        let permit = ask_before_sending(&desk, &mail(&to, "Thanks!")).expect("the shell said no card is needed");
        assert_eq!(actions(&heard), ["request_approval"], "nothing is recorded before the send");
        permit.finish(true);
        let calls = heard.lock().unwrap().clone();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(calls[1]["action"], "record_unasked_action");
        assert_eq!(calls[1]["args"]["action"], "send_mail");
        assert_eq!(calls[1]["args"]["mode"], "bypass_all");
        assert_eq!(calls[1]["args"]["outcome"], "ok");
        assert_eq!(calls[1]["args"]["args_json"], calls[0]["args"]["args_json"], "the record names what went");
    }

    #[test]
    fn a_person_who_was_asked_leaves_nothing_to_record() {
        let (desk, heard) = shell("asked", "pending", "granted");
        let to = vec!["priya@example.com".to_string()];
        ask_before_sending(&desk, &mail(&to, "Thanks!")).unwrap().finish(true);
        assert!(!actions(&heard).contains(&"record_unasked_action".to_string()));
    }
}
