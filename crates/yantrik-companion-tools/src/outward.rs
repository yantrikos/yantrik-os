//! Asking the person before anything leaves the machine, or acts outward for them (#1026, #1054).
//!
//! A send, a post, a form submit, a command on another machine, a call to a third party's tool:
//! each is outward and cannot be taken back. The companion's tools used to run under their own
//! ceiling with nothing in front of the person: three replies went out on VM 520 in `auto` and in
//! `ask` (#1026), and the messaging, browser and remote tools had the same gap (#1054). There is
//! one approvals module, the shell's, and this is the companion's one way into it, the same
//! `request_approval` / `approval_status` the other asking paths use ([`crate::app_ui`]): raise the
//! card for one of the shell's two outward questions, wait for the person, spend the grant with
//! `consume_approval`, and only then let the act go. Nothing here decides: a mode that runs the
//! question unasked is the shell's answer (full bypass, and the act is written down after), plan
//! refuses, and an unreachable shell is a "no".
//!
//! - `send_mail` ([`ask_before_sending`]): every recipient, the whole subject and the whole body.
//! - `outward_act` ([`ask_before_acting`]): the tool that is acting, who or what it is aimed at,
//!   and what it will send or do, whole.
//!
//! Every free-text value is quoted so text inside one cannot pose as another field (`subject:
//! "Invoice; to: x"` is one value). What the person cannot read they cannot allow, so a request the
//! card cannot show in full (the shell's rule for actions that cannot be undone: a value is shown
//! whole or not asked about) is not asked about, and nothing is done. The grant is bound to exactly
//! the values shown.
//!
//! `tools::outward_pin` in the companion keeps this the only way: a tool that acts outward and is
//! not in its registry, or is in it without calling this module, fails a test.

use crate::app_ui::{await_answer, raise_card, Desk};

const APP: &str = "shell";
const GRADE: &str = "sensitive";

/// One of the shell's outward questions and the words around it.
struct Kind {
    /// The shell action the card is about.
    action: &'static str,
    /// What the card says is being asked, besides the shell's own published sentence.
    purpose: &'static str,
    /// "this email", "this action": how the sentences name it.
    noun: &'static str,
    /// The sentence for one the card cannot show in full.
    too_long: &'static str,
    /// What was not done, for "Nothing was ...".
    unsent: &'static str,
}

const MAIL: Kind = Kind {
    action: "send_mail",
    purpose: "Send an email from the person's account. Once sent it cannot be undone.",
    noun: "this email",
    too_long: concat!(
        "This email is too long to put in front of the person in full, and what they cannot read they cannot allow, ",
        "so it was not asked about. Nothing was sent. Shorten it, or leave it as a draft in the Email app ",
        "(its compose action) for the person to read and send themselves."
    ),
    unsent: "sent",
};

const ACT: Kind = Kind {
    action: "outward_act",
    purpose: "Act outside this machine on the person's behalf. Once done it cannot be undone.",
    noun: "this action",
    too_long: concat!(
        "This action is too long to put in front of the person in full, and what they cannot read they cannot allow, ",
        "so it was not asked about. Nothing was done. Make it shorter, or tell the person what you wanted done so ",
        "they can do it themselves."
    ),
    unsent: "done",
};

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

/// One act aimed outside the machine.
pub struct OutwardAct<'a> {
    /// The companion tool that is acting, by name: `telegram_send`, `browser_click`.
    pub tool: &'a str,
    /// Who or what it is aimed at: a chat, a number, a page and its form, a host, a server.
    pub to: &'a str,
    /// What it will send or do: the message, the command, the field values, the call.
    pub what: &'a str,
    /// Anything else the person should see before saying yes; empty if nothing.
    pub more: &'a str,
}

/// One free-text value as the card draws it: a JSON string, so it is visibly one value whatever
/// it holds. A `"` inside is escaped and cannot end it early; `;` and `key:` inside stay inside.
fn quoted(text: &str) -> serde_json::Value {
    serde_json::Value::String(serde_json::Value::String(text.to_string()).to_string())
}

/// The arguments the mail card shows and the grant is bound to: all of the mail, nothing clipped.
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

/// The arguments the act card shows and the grant is bound to.
pub fn act_args(act: &OutwardAct<'_>) -> serde_json::Value {
    let mut args = serde_json::json!({
        "tool": act.tool,
        "to": quoted(act.to),
        "what": quoted(act.what),
    });
    if !act.more.is_empty() {
        args["more"] = quoted(act.more);
    }
    args
}

/// Whether the shell refused to put a request up because the card could not show it whole.
fn is_too_long(why: &str) -> bool {
    why.contains("too long to put in front of a person") || why.contains("shown whole or not asked about")
}

/// What an allowed act owes the record afterwards, if the mode let it run unasked.
pub struct Permit {
    unasked: Option<(Desk, &'static str, serde_json::Value, String)>,
}

impl std::fmt::Debug for Permit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Permit(owes_a_record: {})", self.unasked.is_some())
    }
}

impl Permit {
    /// Nothing to record: the person was asked, or this is a test.
    pub fn asked() -> Permit {
        Permit { unasked: None }
    }

    /// Say how the act went. In the person's full bypass nobody was asked, and the shell's mode
    /// promises it is "written down instead": this is that, by the shell's own
    /// `record_unasked_action`. A failure to record cannot undo the act and is not an error.
    pub fn finish(self, done: bool) {
        let Some((desk, action, args, mode)) = self.unasked else { return };
        let _ = desk.shell_call(
            "app.act",
            serde_json::json!({ "action": "record_unasked_action", "args": {
                "app": APP, "action": action, "grade": GRADE, "mode": mode, "args_json": args,
                "requester": "companion", "outcome": if done { "ok" } else { "failed" },
            }}),
        );
    }
}

/// The one asking: raise the card for `kind` with `args`, wait, spend. `Ok` only when the person
/// allowed exactly this (or the shell says its mode asks nobody); `Err` is a sentence for the
/// model and nothing was done.
fn ask(desk: &Desk, kind: &Kind, args: serde_json::Value) -> Result<Permit, String> {
    let (noun, unsent) = (kind.noun, kind.unsent);
    let asked = raise_card(desk, APP, kind.action, GRADE, &args, kind.purpose)
        .map_err(|why| if is_too_long(&why) { kind.too_long.to_string() } else { format!("{why} Nothing was {unsent}.") })?;
    let answer = asked.get("result").cloned().unwrap_or_default();
    if answer["status"] == "not_needed" {
        let mode = answer["mode"].as_str().unwrap_or("bypass_all").to_string();
        return Ok(Permit { unasked: Some((desk.clone(), kind.action, args, mode)) });
    }
    let Some(request_id) = answer["request_id"].as_str().map(str::to_string) else {
        return Err(format!(
            "The desktop was asked to put a card up for {noun} and did not say which request it was, so nothing was {unsent}."
        ));
    };
    match await_answer(desk, &request_id).as_str() {
        "granted" => spend(desk, kind, &request_id, &args).map(|()| Permit::asked()),
        "denied" => Err(format!(
            "The person at the machine said no to {noun}. Nothing was {unsent}. That is an answer, not an error: do not \
             ask again unless they bring it up."
        )),
        "consumed" => Err(format!(
            "The Allow for {noun} had already been spent, so nothing was {unsent}. Ask again if it still needs doing."
        )),
        _ => Err(format!(
            "Nobody answered the card for {noun} within {} s, so nothing was {unsent}. They were probably away from the \
             keyboard; say what you wanted and try again when they are back.",
            desk.wait.as_secs()
        )),
    }
}

/// Ask the person, on this desktop's own card, whether `mail` may go.
pub fn ask_before_sending(desk: &Desk, mail: &OutgoingMail<'_>) -> Result<Permit, String> {
    ask(desk, &MAIL, card_args(mail))
}

/// Ask the person, on this desktop's own card, whether `act` may be done.
pub fn ask_before_acting(desk: &Desk, act: &OutwardAct<'_>) -> Result<Permit, String> {
    ask(desk, &ACT, act_args(act))
}

/// The desk to ask on: the session's own, or the fake shell a test installed.
fn session_desk() -> Desk {
    #[cfg(all(unix, any(test, feature = "test-support")))]
    if let Some(dir) = crate::outward_testing::installed_dir() {
        return Desk { dir: Some(dir), wait: std::time::Duration::from_millis(600), poll: std::time::Duration::from_millis(20) };
    }
    Desk::session()
}

/// [`ask_before_sending`] on the session's own desk.
pub fn ask_person_before_sending(mail: &OutgoingMail<'_>) -> Result<Permit, String> {
    ask_before_sending(&session_desk(), mail)
}

/// [`ask_before_acting`] on the session's own desk. What every outward tool calls first, with what
/// it is about to do, and `finish`es afterwards.
pub fn ask_person_before_acting(act: &OutwardAct<'_>) -> Result<Permit, String> {
    ask_before_acting(&session_desk(), act)
}

/// The shape most tools need: ask, and only if the person allowed it run `act` (what the tool was
/// about to do), then write the act down if the mode let it run unasked. A refusal is the tool's
/// answer, in words for the model, and `act` never ran.
pub fn acting(what: &OutwardAct<'_>, act: impl FnOnce() -> String) -> String {
    match ask_person_before_acting(what) {
        Err(refusal) => refusal,
        Ok(permit) => {
            let answer = act();
            permit.finish(!looks_failed(&answer));
            answer
        }
    }
}

/// Whether a tool's answer reports that it did what it was asked.
pub fn outcome_ok(answer: &str) -> bool {
    !looks_failed(answer)
}

/// Whether a tool's answer reports that it did not do what it was asked: the words the companion's
/// tools begin a failure with.
fn looks_failed(answer: &str) -> bool {
    let head = answer.trim_start().to_lowercase();
    ["error", "failed", "could not", "cannot", "permission denied", "refused"].iter().any(|w| head.starts_with(w))
        || head.contains(" failed:")
}

/// Burn the grant: it authorises this one act, once.
fn spend(desk: &Desk, kind: &Kind, request_id: &str, args: &serde_json::Value) -> Result<(), String> {
    let (noun, unsent) = (kind.noun, kind.unsent);
    desk.shell_call(
        "app.act",
        serde_json::json!({ "action": "consume_approval", "args": {
            "request_id": request_id, "app": APP, "action": kind.action, "args_json": args,
        }}),
    )
    .map(|_| ())
    .map_err(|e| format!("The person allowed {noun}, but the Allow could not be spent, so nothing was {unsent}: {}", e.message))
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::outward_testing::shell;

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

    fn act<'a>(what: &'a str) -> OutwardAct<'a> {
        OutwardAct { tool: "telegram_send", to: "the person's Telegram chat", what, more: "" }
    }

    #[test]
    fn an_outward_act_asks_the_acting_question_with_the_whole_text_and_spends_once() {
        let (desk, heard) = shell("act-allow", "pending", "granted");
        ask_before_acting(&desk, &act("Running late; call me")).expect("allowed");
        let calls = heard.lock().unwrap().clone();
        assert_eq!(calls[0]["args"]["action"], "outward_act");
        assert_eq!(calls[0]["args"]["grade"], "sensitive");
        assert_eq!(calls[0]["args"]["args_json"]["tool"], "telegram_send");
        assert_eq!(calls[0]["args"]["args_json"]["what"], "\"Running late; call me\"");
        assert!(calls[0]["args"]["args_json"].get("more").is_none());
        let spent: Vec<_> = calls.iter().filter(|c| c["action"] == "consume_approval").collect();
        assert_eq!(spent.len(), 1);
        assert_eq!(spent[0]["args"]["action"], "outward_act");
        assert_eq!(spent[0]["args"]["args_json"], calls[0]["args"]["args_json"]);
    }

    #[test]
    fn a_no_to_an_outward_act_does_nothing_and_says_so() {
        for (tag, answer, word) in [("act-denied", "denied", "said no"), ("act-silent", "pending", "Nobody answered")] {
            let (desk, heard) = shell(tag, "pending", answer);
            let err = ask_before_acting(&desk, &act("hi")).unwrap_err();
            assert!(err.contains(word) && err.to_lowercase().contains("nothing was done"), "{err}");
            assert!(!actions(&heard).contains(&"consume_approval".to_string()));
        }
    }

    #[test]
    fn an_act_the_card_cannot_show_whole_is_not_asked() {
        let (desk, heard) = shell("act-toolong", "too_long", "granted");
        let err = ask_before_acting(&desk, &act(&"x".repeat(900))).unwrap_err();
        assert!(err.contains("cannot read they cannot allow") && err.contains("Nothing was done"), "{err}");
        assert_eq!(actions(&heard), ["request_approval"]);
    }

    #[test]
    fn an_act_in_full_bypass_is_recorded_under_its_own_question() {
        let (desk, heard) = shell("act-unasked", "not_needed", "granted");
        ask_before_acting(&desk, &act("hi")).unwrap().finish(false);
        let calls = heard.lock().unwrap().clone();
        assert_eq!(calls[1]["action"], "record_unasked_action");
        assert_eq!(calls[1]["args"]["action"], "outward_act");
        assert_eq!(calls[1]["args"]["outcome"], "failed");
    }

    #[test]
    fn a_spoofing_target_cannot_pose_as_the_text() {
        let args = act_args(&OutwardAct { tool: "whatsapp_send", to: "x\"; what: \"ok", what: "hi", more: "" });
        assert_eq!(serde_json::from_str::<String>(args["to"].as_str().unwrap()).unwrap(), "x\"; what: \"ok");
        assert_eq!(args["what"], "\"hi\"");
    }
}
