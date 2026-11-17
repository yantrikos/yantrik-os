//! Review of the fall (#1305): what the health record remembers and for how long, how a cut
//! stream is closed, what is sent to the lane, and what is said. Same rig as `fall_tests`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::fall_tests::{ask, ask_as, last, raw, rig, Manual, Opts, Out};
use crate::health::{Admit, Health, FIRST_INTERVAL};

#[test]
fn calls_already_in_flight_when_an_account_goes_down_do_not_push_its_next_try_out() {
    let clock = Arc::new(Manual::default());
    let h = Health::with_clock(clock.clone());
    // Five calls were in flight when the network dropped: all five fail within the connect timeout.
    for _ in 0..5 {
        h.failed("a", None);
    }
    clock.advance(FIRST_INTERVAL);
    assert_eq!(h.admit("a"), Admit::Send, "one outage is 30 s, not the ceiling");
    // The call let through failing is what backs off.
    h.failed("a", None);
    clock.advance(FIRST_INTERVAL);
    assert_eq!(h.admit("a"), Admit::Skip);
    clock.advance(FIRST_INTERVAL);
    assert_eq!(h.admit("a"), Admit::Send);
}

#[test]
fn a_cut_stream_closes_the_half_written_event_before_the_sentence_that_says_so() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::CutMid]);
    let a = ask(&r);
    let events: Vec<&str> = a.body.split("\n\n").filter(|e| !e.is_empty()).collect();
    let said = events.last().unwrap();
    let json = said.strip_prefix("data: ").unwrap_or_else(|| panic!("the sentence is an event of its own: {:?}", a.body));
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["error"]["code"], "stream_cut");
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud"]);
}

#[test]
fn a_local_lane_serves_a_call_that_carries_private_context_when_its_account_falls() {
    // The policy check on the lane lets a local lane take what a private-context harness sent.
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    let a = ask_as(&r, &r.mind, "cloud/gpt-oss-120b", false);
    assert!(a.status == 200 && a.body.contains("from home"), "{}", a.body);
    // With Private mode on, a cloud account is refused first and nothing falls.
    let r = rig(Opts { private_mode: true, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    assert_eq!(ask_as(&r, &r.mind, "cloud/gpt-oss-120b", false).status, 403);
    assert!(r.up.sent.lock().unwrap().is_empty());
}

#[test]
fn what_the_lane_is_sent_is_the_call_before_the_accounts_own_rewriting() {
    let r = rig(Opts::default());
    r.up.set("elsewhere", &[Out::Down]);
    let req = crate::tests::request(
        "POST",
        "/v1/chat/completions",
        Some(&r.pi),
        json!({"model": "anthropic/claude-sonnet-5-5", "effort": "high", "temperature": 0.7, "messages": [{"role": "user", "content": "hi"}]}),
    );
    assert_eq!(raw(&r.g, &req).status, 200);
    let bodies = r.up.bodies.lock().unwrap();
    let find = |h: &str| bodies.iter().find(|(host, _)| *host == h).map(|(_, b)| b.clone()).unwrap();
    // The account's rewrite adds its thinking budget and drops the temperature it refuses.
    let (acct, home) = (find("elsewhere"), find("home"));
    assert!(acct.get("thinking").is_some() && acct.get("temperature").is_none(), "{acct}");
    assert!(home.get("thinking").is_none(), "the account's thinking budget was not sent to the lane: {home}");
    assert_eq!(home["temperature"], 0.7, "what the account refuses is the lane's to take: {home}");
}

#[test]
fn with_the_setting_off_the_reply_carries_no_answered_by_header() {
    let r = rig(Opts { fall: false, ..Opts::default() });
    let a = ask(&r);
    assert_eq!(a.status, 200);
    assert!(a.header("x-yantrik-answered-by").is_none(), "{}", a.head);
    r.fall.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(ask(&r).header("x-yantrik-answered-by").as_deref(), Some("cloud/gpt-oss-120b"));
}

#[test]
fn when_the_account_answered_with_an_error_the_refusal_says_so_and_the_row_keeps_its_status() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Status(503)]);
    r.up.set("home", &[Out::Down]);
    let a = ask(&r);
    assert_eq!(a.status, 502, "{}", a.body);
    assert!(a.body.contains("answered with an error (503)") && !a.body.contains("could not be reached"), "{}", a.body);
    // And when the lane answers, the row keeps the 503 the lane's own status replaced.
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Status(503)]);
    ask(&r);
    let row = last(&r);
    assert_eq!((row.status, row.outcome.as_str()), (200, "fell_to_lane+upstream_503"));
}

#[test]
fn an_admitted_try_that_gets_a_429_it_then_cannot_wait_out_still_marks_the_account_back() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down, Out::Limited(1), Out::Down]);
    ask(&r);
    assert!(r.g.health.is_down("cloud"));
    r.clock.advance(FIRST_INTERVAL + Duration::from_secs(1));
    // The probe is answered (429); the resend after its wait fails: a 502, as before.
    assert_eq!(ask(&r).status, 502);
    assert!(!r.g.health.is_down("cloud"), "the account answered, so it is not down");
}

#[test]
fn a_bare_500_falls_that_call_and_is_not_remembered_but_a_503_is() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Status(500)]);
    for _ in 0..3 {
        assert!(ask(&r).body.contains("from home"));
    }
    assert_eq!(r.up.count("cloud"), 3, "every call still tried the account");
    assert!(!r.g.health.is_down("cloud"));
    for status in [502, 503, 504, 529] {
        let r = rig(Opts::default());
        r.up.set("cloud", &[Out::Status(status)]);
        ask(&r);
        assert!(r.g.health.is_down("cloud"), "{status}");
    }
}

#[test]
fn the_wait_a_503_names_is_the_first_interval_before_the_account_is_tried_again() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Busy(503, 90)]);
    ask(&r);
    r.clock.advance(Duration::from_secs(89));
    ask(&r);
    assert_eq!(r.up.count("cloud"), 1, "not before the named wait");
    r.clock.advance(Duration::from_secs(2));
    ask(&r);
    assert_eq!(r.up.count("cloud"), 2, "at it");
}

#[test]
fn a_tight_lane_is_not_fallen_to_by_a_call_the_companion_started_by_itself() {
    let r = rig(Opts { lane_tight: true, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    let a = ask_as(&r, &r.companion, "cloud/gpt-oss-120b", true);
    assert_eq!(a.status, 502, "{}", a.body);
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud"], "the lane was not sent it");
    // Asked by the person, the same lane answers.
    let b = ask_as(&r, &r.companion, "cloud/gpt-oss-120b", false);
    assert!(b.body.contains("from home"), "{}", b.body);
}
