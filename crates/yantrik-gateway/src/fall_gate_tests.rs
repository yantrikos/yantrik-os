//! What must never fall: a setting that is off, no lane, a refusal by policy, a call that was
//! changed or kept while it was sent, anything but the person's own lane. And the health record's
//! own rules. See `fall_tests` for the rig.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::fall_tests::{ask, ask_as, last, rig, Manual, Opts, Out};
use crate::health::{Admit, Health, FIRST_INTERVAL, MAX_INTERVAL};
use crate::policy::Sharing;

#[test]
fn with_the_setting_off_nothing_falls_and_an_account_that_is_down_fails_as_it_always_did() {
    let r = rig(Opts { fall: false, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    let a = ask(&r);
    assert_eq!((a.status, a.body.contains("unreachable")), (502, true), "{}", a.body);
    r.up.set("cloud", &[Out::Status(503)]);
    assert_eq!(ask(&r).status, 503);
    assert_eq!(r.up.count("home"), 0);
    assert!(!r.g.health.is_down("cloud"), "nothing is remembered when nothing fell");
    // And on again, a moment later: honoured at the next call.
    r.fall.store(true, Ordering::SeqCst);
    assert!(ask(&r).body.contains("from home"));
}

#[test]
fn with_no_lane_nothing_falls() {
    let r = rig(Opts { lane: None, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    assert_eq!(ask(&r).status, 502);
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud"]);
    assert!(!r.g.health.is_down("cloud"));
}

#[test]
fn a_call_never_falls_anywhere_but_the_lane_the_person_named_and_never_off_this_network() {
    // The "lane" is another cloud account: nothing is sent to it.
    let r = rig(Opts { lane: Some("elsewhere/m"), ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    assert_eq!(ask(&r).status, 502);
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud"]);
    // The lane is the account that is down: no second try at the same place.
    let r = rig(Opts::default());
    r.up.set("home", &[Out::Down]);
    assert_eq!(ask_as(&r, &r.pi, "home/qwen3:8b", false).status, 502);
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["home"]);
}

#[test]
fn a_policy_refusal_is_never_turned_into_a_fall() {
    // Sharing off for a call carrying private context.
    let r = rig(Opts { cloud: Sharing::Off, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    let a = ask_as(&r, &r.mind, "cloud/gpt-oss-120b", false);
    assert_eq!((a.status, a.body.contains("private_context_not_shared")), (403, true), "{}", a.body);
    assert!(r.up.sent.lock().unwrap().is_empty(), "nothing was sent anywhere");
    // A tier switched off between the first routing and the fall.
    let r = rig(Opts { cloud_off_from: 1, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    let a = ask(&r);
    assert_eq!((a.status, a.body.contains("tier_off")), (403, true), "{}", a.body);
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud"], "the lane was not sent it");
    assert_eq!(last(&r).outcome, "tier_off");
}

#[test]
fn the_setting_is_read_again_before_the_second_send() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    ask(&r);
    // The account is remembered as down; the person switches the setting off; the next call is
    // sent to the account as it was before any fall.
    r.fall.store(false, Ordering::SeqCst);
    r.up.set("cloud", &[Out::Ok]);
    let a = ask(&r);
    assert!(a.body.contains("from cloud"), "{}", a.body);
    assert_eq!(r.up.count("home"), 1);
}

#[test]
fn private_mode_behaves_as_before_with_the_setting_on() {
    let r = rig(Opts { private_mode: true, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    let a = ask(&r);
    assert_eq!((a.status, a.body.contains("private_mode")), (403, true), "{}", a.body);
    assert!(r.up.sent.lock().unwrap().is_empty(), "nothing is sent to the cloud, so nothing falls");
    // The lane itself is local, and answers.
    let b = ask_as(&r, &r.pi, "home/qwen3:8b", false);
    assert!(b.status == 200 && b.body.contains("from home"), "{}", b.body);
    // The local lane down in Private mode: refused as before, one send, no other target.
    r.up.set("home", &[Out::Down]);
    assert_eq!(ask_as(&r, &r.pi, "home/qwen3:8b", false).status, 502);
    assert_eq!(r.up.count("cloud") + r.up.count("elsewhere"), 0);
}

#[test]
fn a_call_the_companion_started_by_itself_to_a_tight_account_is_refused_and_never_fallen() {
    let r = rig(Opts { cloud_tight: true, ..Opts::default() });
    r.up.set("cloud", &[Out::Down]);
    let a = ask_as(&r, &r.companion, "cloud/gpt-oss-120b", true);
    assert_eq!(a.status, 429, "{}", a.body);
    assert!(a.body.contains(yantrik_ml::asked::KEPT_CODE), "{}", a.body);
    assert!(r.up.sent.lock().unwrap().is_empty());
    assert_eq!(last(&r).asked.as_deref(), Some("self"));
}

#[test]
fn a_fall_is_sent_in_the_lanes_own_words_not_the_accounts() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    let req = crate::tests::request(
        "POST",
        "/v1/chat/completions",
        Some(&r.pi),
        serde_json::json!({"model": "cloud/gpt-oss-120b", "effort": "high", "messages": [{"role": "user", "content": "hi"}]}),
    );
    assert_eq!(crate::fall_tests::raw(&r.g, &req).status, 200);
    let bodies = r.up.bodies.lock().unwrap();
    let find = |h: &str| bodies.iter().find(|(host, _)| *host == h).map(|(_, b)| b.clone()).unwrap();
    let (cloud, home) = (find("cloud"), find("home"));
    assert_eq!(cloud["reasoning_effort"], "high", "{cloud}");
    assert!(home.get("reasoning_effort").is_none() && home.get("effort").is_none(), "the cloud's words were not sent to the lane: {home}");
    assert_eq!(home["messages"], cloud["messages"], "the same conversation");
}

#[test]
fn health_lets_one_try_through_per_interval_backs_off_to_a_ceiling_and_forgets_on_recovery() {
    let clock = Arc::new(Manual::default());
    let h = Health::with_clock(clock.clone());
    assert_eq!(h.admit("a"), Admit::Send, "an account never seen is sent to");
    h.failed("a");
    assert_eq!(h.admit("a"), Admit::Skip);
    clock.advance(FIRST_INTERVAL);
    assert_eq!(h.admit("a"), Admit::Send);
    assert_eq!(h.admit("a"), Admit::Skip, "the try is taken; the next waits a whole interval");
    let mut waited = FIRST_INTERVAL;
    for _ in 0..10 {
        h.failed("a");
        waited = (waited * 2).min(MAX_INTERVAL);
        clock.advance(waited - Duration::from_secs(1));
        assert_eq!(h.admit("a"), Admit::Skip);
        clock.advance(Duration::from_secs(1));
        assert_eq!(h.admit("a"), Admit::Send);
    }
    assert_eq!(waited, MAX_INTERVAL);
    h.recovered("a");
    assert!(!h.is_down("a"));
    assert_eq!(h.admit("a"), Admit::Send);
}

#[test]
fn health_is_bounded_whatever_accounts_fail() {
    let clock = Arc::new(Manual::default());
    let h = Health::with_clock(clock.clone());
    for i in 0..500 {
        clock.advance(Duration::from_millis(1));
        h.failed(&format!("acct-{i}"));
    }
    let down = (0..500).filter(|i| h.is_down(&format!("acct-{i}"))).count();
    assert_eq!(down, 64);
    assert!(h.is_down("acct-499"), "the newest is kept");
    assert!(!h.is_down("acct-0"), "the oldest went");
}
