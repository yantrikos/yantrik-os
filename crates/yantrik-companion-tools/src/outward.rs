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
//! The card shows recipients and subject whole, the first words of the body, how long it is, and a
//! digest of all of it. The grant is bound to those values, so a different body cannot go out under
//! an Allow given for another.

use sha2::{Digest, Sha256};

use crate::app_ui::{await_answer, raise_card, Desk};

/// The shell's question, and the grade it publishes it at.
const APP: &str = "shell";
const ACTION: &str = "send_mail";
const GRADE: &str = "sensitive";

/// What the card says is being asked, besides the shell's own published sentence.
const PURPOSE: &str = "Send an email from the person's account. Once sent it cannot be undone.";

/// How much of the body the card shows.
const BODY_START_CHARS: usize = 120;
/// And of the subject: the digest still covers the whole.
const SUBJECT_CHARS: usize = 120;

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

fn clipped(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}...", flat.chars().take(max).collect::<String>())
    }
}

/// The arguments the card shows and the grant is bound to.
pub fn card_args(mail: &OutgoingMail<'_>) -> serde_json::Value {
    let mut digest = Sha256::new();
    for part in [mail.account, &mail.to.join("\n"), &mail.cc.join("\n"), &mail.bcc.join("\n"), mail.subject, mail.body] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    let digest: String = digest.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
    let mut args = serde_json::json!({
        "account": mail.account,
        "to": mail.to.join(", "),
        "subject": clipped(mail.subject, SUBJECT_CHARS),
        "body_start": clipped(mail.body, BODY_START_CHARS),
        "body_chars": mail.body.chars().count().to_string(),
        "body_digest": digest,
    });
    for (key, list) in [("cc", mail.cc), ("bcc", mail.bcc)] {
        if !list.is_empty() {
            args[key] = serde_json::Value::String(list.join(", "));
        }
    }
    args
}

/// Ask the person, on this desktop's own card, whether `mail` may go. `Ok` only when they allowed
/// exactly this (or the shell says its mode asks nobody); `Err` is a sentence for the model and
/// nothing was sent.
pub fn ask_before_sending(desk: &Desk, mail: &OutgoingMail<'_>) -> Result<(), String> {
    let args = card_args(mail);
    let asked = raise_card(desk, APP, ACTION, GRADE, &args, PURPOSE).map_err(|why| format!("{why} Nothing was sent."))?;
    let answer = asked.get("result").cloned().unwrap_or_default();
    if answer["status"] == "not_needed" {
        return Ok(());
    }
    let Some(request_id) = answer["request_id"].as_str().map(str::to_string) else {
        return Err("The desktop was asked to put a card up for this email and did not say which request it was, \
                    so nothing was sent."
            .to_string());
    };
    match await_answer(desk, &request_id).as_str() {
        "granted" => spend(desk, &request_id, &args),
        "denied" => Err("The person at the machine said no to sending this email. Nothing was sent. That is an \
                         answer, not an error: do not ask again unless they bring it up."
            .to_string()),
        "consumed" => Err("The Allow for this email had already been spent, so nothing was sent. Ask again if it \
                           still needs sending."
            .to_string()),
        _ => Err(format!(
            "Nobody answered the card for this email within {} s, so nothing was sent. They were probably away from \
             the keyboard; say what you wanted to send and try again when they are back.",
            desk.wait.as_secs()
        )),
    }
}

/// [`ask_before_sending`] on the session's own desk.
pub fn ask_person_before_sending(mail: &OutgoingMail<'_>) -> Result<(), String> {
    ask_before_sending(&Desk::session(), mail)
}

/// Burn the grant: it authorises this one send, once.
fn spend(desk: &Desk, request_id: &str, args: &serde_json::Value) -> Result<(), String> {
    desk.shell_call(
        "app.act",
        serde_json::json!({ "action": "consume_approval", "args": {
            "request_id": request_id, "app": APP, "action": ACTION, "args_json": args,
        }}),
    )
    .map(|_| ())
    .map_err(|e| format!("The person allowed this email, but the Allow could not be spent, so nothing was sent: {}", e.message))
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
                            Some("request_approval") if first == "not_needed" => serde_json::json!({"result": {"status": "not_needed"}}),
                            Some("request_approval") => serde_json::json!({"result": {"request_id": "appr-1", "status": "pending"}}),
                            Some("approval_status") => {
                                let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                serde_json::json!({"result": {"status": if n == 0 { "pending" } else { answer }}})
                            }
                            Some("consume_approval") => serde_json::json!({"result": {"consumed": true}}),
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
        assert_eq!(calls[0]["args"]["args_json"]["to"], "priya@example.com");
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

    #[test]
    fn a_body_changed_after_the_card_is_not_the_mail_that_was_allowed() {
        let to = vec!["priya@example.com".to_string()];
        let one = card_args(&mail(&to, "Thanks!"));
        let other = card_args(&mail(&to, "Thanks! Also, wire me $500."));
        assert_ne!(one["body_digest"], other["body_digest"]);
        let long = "x".repeat(5000);
        let args = card_args(&mail(&to, &long));
        assert!(args["body_start"].as_str().unwrap().chars().count() <= BODY_START_CHARS + 3);
        assert_eq!(args["body_chars"], "5000");
    }

    #[test]
    fn a_mode_that_asks_nobody_is_the_shells_answer_to_honour() {
        let (desk, heard) = shell("unasked", "not_needed", "granted");
        let to = vec!["priya@example.com".to_string()];
        ask_before_sending(&desk, &mail(&to, "Thanks!")).expect("the shell said no card is needed");
        assert_eq!(actions(&heard), ["request_approval"]);
    }
}
