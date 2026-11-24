//! A request too large for an account is adjusted, not given up (#1390). Nothing here sleeps: the
//! upstream records the waits asked of it, and time is a clock a test moves by hand.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::fall_tests::{raw, Hosts, Manual, Opts, Out, Raw, Rig, Switch};
use crate::fit::{estimate, shrink, Room};
use crate::health::Health;
use crate::hold::Holds;
use crate::log::{Call, CallLog};
use crate::policy::{Refusal, Target};
use crate::server::{Accounts, Gateway, ModelInfo};
use crate::tests::{request, Fake};
use crate::tokens::{mint, Grant, Tokens};

/// `Switch`'s accounts, with a room per account.
struct Roomy {
    inner: Switch,
    rooms: HashMap<&'static str, Room>,
}

impl Accounts for Roomy {
    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        self.inner.target(account, model)
    }
    fn private_mode(&self) -> bool {
        self.inner.private_mode()
    }
    fn private_lane(&self) -> Option<String> {
        self.inner.private_lane()
    }
    fn fall_to_lane(&self) -> bool {
        self.inner.fall_to_lane()
    }
    fn room(&self, account: &str, _model: &str) -> Room {
        self.rooms.get(account).copied().unwrap_or_default()
    }
}

fn tpm(n: u64) -> Room {
    Room { per_minute: Some(n), context: None }
}

fn rig(o: Opts, rooms: &[(&'static str, Room)]) -> Rig {
    let tokens = Arc::new(Tokens::in_memory());
    let (pi, mind, companion) = (mint().unwrap(), mint().unwrap(), mint().unwrap());
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    tokens.register(&mind, Grant { harness: "mind".into(), private_context: true }).unwrap();
    tokens.register(&companion, Grant { harness: crate::asked::COMPANION.into(), private_context: true }).unwrap();
    let fall = Arc::new(AtomicBool::new(o.fall));
    let up = Arc::new(Hosts::default());
    let clock = Arc::new(Manual::default());
    let switch = Switch {
        inner: Fake { private_mode: o.private_mode, cloud: o.cloud, lane: o.lane },
        fall: fall.clone(),
        cloud_off_from: usize::MAX,
        cloud_asks: AtomicUsize::new(0),
        cloud_tight: false,
        lane_tight: false,
    };
    let g = Gateway {
        tokens,
        accounts: Arc::new(Roomy { inner: switch, rooms: rooms.iter().copied().collect() }),
        upstream: up.clone(),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(),
        params_refused: Default::default(),
        health: Arc::new(Health::with_clock(clock.clone())),
        holds: Arc::new(Holds::with_clock(clock.clone())),
    };
    Rig { g, up, fall, clock, pi, mind, companion }
}

const SECRET: &str = "the-secret-words-of-a-page";

/// A page of `lines` lines, each 40 bytes or so.
fn page(lines: usize) -> String {
    (0..lines).map(|i| format!("{SECRET} line {i:05} of the long page\n")).collect()
}

fn conversation(page_lines: usize) -> Value {
    json!({
        "model": "cloud/gpt-oss-120b",
        "messages": [
            {"role": "system", "content": "You are a helpful mind."},
            {"role": "user", "content": "Read the page and tell me what it says."},
            {"role": "assistant", "content": null, "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "read", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": page(page_lines)},
            {"role": "user", "content": "Well?"},
        ],
    })
}

fn ask(r: &Rig, body: Value) -> Raw {
    raw(&r.g, &request("POST", "/v1/chat/completions", Some(&r.pi), body))
}

fn last(r: &Rig) -> Call {
    r.g.log.recent().pop().unwrap()
}

fn sent(r: &Rig, i: usize) -> Value {
    r.up.bodies.lock().unwrap()[i].1.clone()
}

#[test]
fn a_request_that_fits_is_sent_untouched() {
    let r = rig(Opts::default(), &[("cloud", tpm(8_000))]);
    let body = conversation(20);
    let a = ask(&r, body.clone());
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(r.up.count("cloud"), 1);
    assert_eq!(sent(&r, 0)["messages"], body["messages"]);
    assert!(last(&r).shrunk.is_none());
    assert!(a.header("x-yantrik-shrunk").is_none());
}

#[test]
fn a_request_over_the_room_is_shrunk_to_fit_and_what_must_stay_stays() {
    let r = rig(Opts::default(), &[("cloud", tpm(2_000))]);
    let body = conversation(2_000);
    assert!(estimate(&body) > 2_000);
    let a = ask(&r, body.clone());
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(r.up.count("cloud"), 1, "one send, not a 413 and a second");
    let out = sent(&r, 0);
    assert!(estimate(&out) <= 2_000, "{}", estimate(&out));
    let (was, now) = (body["messages"].as_array().unwrap(), out["messages"].as_array().unwrap());
    assert_eq!(was.len(), now.len(), "no message is dropped or reordered");
    for i in [0, 1, 2, 4] {
        assert_eq!(was[i], now[i], "message {i} is whole");
    }
    assert_eq!(now[3]["role"], "tool");
    assert_eq!(now[3]["tool_call_id"], "c1");
    let cut = now[3]["content"].as_str().unwrap();
    let marker = cut.lines().last().unwrap();
    assert!(marker.contains("Cut by the gateway") && marker.contains("bytes left out") && marker.contains("Ask again"), "{marker}");
    assert!(page(2_000).starts_with(&cut[..cut.rfind('\n').unwrap()]), "the head is the page's own, on a line end");
    let row = last(&r).shrunk.expect("the log row says it was shrunk");
    assert_eq!(row.results, 1);
    assert!(row.bytes > 0);
    assert_eq!(a.header("x-yantrik-shrunk").unwrap(), format!("1 results, {} bytes", row.bytes));
    let logged = serde_json::to_string(&last(&r)).unwrap() + &a.head;
    assert!(!logged.contains(SECRET), "counts only");
}

#[test]
fn the_newest_message_is_never_cut_even_when_it_is_a_tool_result() {
    let mut body = conversation(10);
    let big = page(3_000);
    let m = body["messages"].as_array_mut().unwrap();
    m.pop();
    m.push(json!({"role": "tool", "tool_call_id": "c1", "content": big.clone()}));
    assert_eq!(shrink(&mut body, 500), None, "only the newest is large: nothing to cut");
    assert_eq!(body["messages"][4]["content"], big);
}

#[test]
fn the_largest_results_are_cut_first() {
    let mut body = conversation(100);
    body["messages"].as_array_mut().unwrap().insert(4, json!({"role": "tool", "tool_call_id": "c2", "content": page(4_000)}));
    let small = body["messages"][3]["content"].clone();
    let limit = estimate(&body) - 10_000;
    let done = shrink(&mut body, limit).unwrap();
    assert_eq!(done.results, 1);
    assert_eq!(body["messages"][3]["content"], small, "the small one is left whole");
    assert!(body["messages"][4]["content"].as_str().unwrap().contains("Cut by the gateway"));
}

#[test]
fn a_per_minute_overflow_waits_and_does_not_shrink() {
    let r = rig(Opts::default(), &[("cloud", tpm(3_000))]);
    let body = conversation(150);
    let one = estimate(&body);
    assert!(one < 3_000 && one * 2 > 3_000, "{one}: fits an empty minute, not two");
    assert_eq!(ask(&r, body.clone()).status, 200);
    r.clock.advance(Duration::from_secs(10));
    let a = ask(&r, body.clone());
    assert_eq!(a.status, 200, "{}", a.body);
    let waits = r.up.waits.lock().unwrap().clone();
    assert_eq!(waits.len(), 1, "waited once");
    assert!(waits[0] > Duration::from_secs(49) && waits[0] <= Duration::from_secs(51), "{waits:?}");
    assert!(last(&r).shrunk.is_none(), "it waited, it did not shrink");
    assert_eq!(sent(&r, 1)["messages"], body["messages"]);
    assert!(last(&r).waited_ms >= 50_000);
}

#[test]
fn a_request_that_cannot_fit_ends_in_words_that_name_the_account_and_never_the_providers_body() {
    let r = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", tpm(500))]);
    let mut body = conversation(1);
    body["messages"][1]["content"] = json!(page(2_000));
    let a = ask(&r, body);
    assert_eq!(a.status, 413, "{}", a.body);
    let v: Value = serde_json::from_str(&a.body).unwrap();
    assert_eq!(v["error"]["code"], "upstream_413");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("cloud provider") && msg.contains("too long") && msg.contains("new chat"), "{msg}");
    assert!(!a.body.contains(SECRET));
    assert_eq!(r.up.count("cloud"), 0, "nothing was sent to be refused");
    assert_eq!(last(&r).outcome, "upstream_413");
}

#[test]
fn a_request_that_cannot_fit_moves_to_the_private_lane_when_allowed_and_says_so() {
    let r = rig(Opts::default(), &[("cloud", tpm(500))]);
    let mut body = conversation(1);
    body["messages"][1]["content"] = json!(page(2_000));
    let a = ask(&r, body);
    assert_eq!(a.status, 200, "{}", a.body);
    assert!(a.body.contains("from home"), "{}", a.body);
    assert_eq!(a.header("x-yantrik-answered-by").unwrap(), "home/qwen3:8b");
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["home"]);
    assert_eq!(last(&r).outcome, "fell_to_lane+upstream_413");
}

#[test]
fn a_move_needs_the_person_to_allow_it_and_a_lane_with_room() {
    let mut body = conversation(1);
    body["messages"][1]["content"] = json!(page(2_000));
    let off = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", tpm(500))]);
    assert_eq!(ask(&off, body.clone()).status, 413);
    assert_eq!(off.up.count("home"), 0);
    let tight = rig(Opts::default(), &[("cloud", tpm(500)), ("home", tpm(500))]);
    let a = ask(&tight, body.clone());
    assert_eq!(a.status, 413, "{}", a.body);
    assert_eq!(tight.up.count("home"), 0, "a lane that cannot take it either is not asked");
    let none = rig(Opts { lane: None, ..Opts::default() }, &[("cloud", tpm(500))]);
    assert_eq!(ask(&none, body).status, 413);
}

#[test]
fn a_local_account_is_never_moved_to_a_less_private_one() {
    // The lane is the only place a call moves to, and only a local one: a call asked of the
    // private model itself has nowhere else to go.
    let r = rig(Opts::default(), &[("home", tpm(500))]);
    let mut body = conversation(1);
    body["model"] = json!("home/qwen3:8b");
    body["messages"][1]["content"] = json!(page(2_000));
    let a = ask(&r, body);
    assert_eq!(a.status, 413, "{}", a.body);
    assert_eq!(r.up.count("cloud"), 0);
    assert!(a.body.contains("home provider"), "{}", a.body);
}

#[test]
fn an_upstream_413_is_treated_as_a_predicted_one() {
    // The gateway did not know this account's room; the provider said.
    let r = rig(Opts { fall: false, ..Opts::default() }, &[]);
    r.up.set("cloud", &[Out::Status(413), Out::Ok]);
    let a = ask(&r, conversation(2_000));
    assert_eq!(a.status, 200, "{}", a.body);
    assert!(a.body.contains("from cloud"));
    assert_eq!(r.up.count("cloud"), 2);
    assert!(estimate(&sent(&r, 1)) < estimate(&sent(&r, 0)) / 2 + 100, "the second send is about half");
    assert_eq!(last(&r).shrunk.map(|s| s.results), Some(1));
}

#[test]
fn an_upstream_413_that_stays_ends_in_the_gateways_words_after_one_more_send() {
    let r = rig(Opts { fall: false, ..Opts::default() }, &[]);
    r.up.set("cloud", &[Out::Status(413)]);
    let a = ask(&r, conversation(2_000));
    assert_eq!(a.status, 413);
    assert_eq!(r.up.count("cloud"), 2, "once more, no more");
    let v: Value = serde_json::from_str(&a.body).unwrap();
    assert_eq!(v["error"]["code"], "upstream_413");
    assert!(v["error"]["message"].as_str().unwrap().contains("cloud provider"));
    assert!(!a.body.contains("status 413"), "not the provider's own body");
}

#[test]
fn an_upstream_413_with_nothing_to_cut_ends_without_a_second_send() {
    let r = rig(Opts { fall: false, ..Opts::default() }, &[]);
    r.up.set("cloud", &[Out::Status(413)]);
    let a = ask(&r, json!({"model": "cloud/gpt-oss-120b", "messages": [{"role": "user", "content": page(500)}]}));
    assert_eq!(a.status, 413);
    assert_eq!(r.up.count("cloud"), 1);
}

#[test]
fn an_upstream_413_moves_to_the_lane_when_allowed() {
    let r = rig(Opts::default(), &[]);
    r.up.set("cloud", &[Out::Status(413)]);
    let a = ask(&r, json!({"model": "cloud/gpt-oss-120b", "messages": [{"role": "user", "content": page(500)}]}));
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(a.header("x-yantrik-answered-by").unwrap(), "home/qwen3:8b");
}

#[test]
fn a_cut_is_on_a_line_end_and_never_inside_a_character() {
    let mut body = json!({"model": "m", "messages": [
        {"role": "tool", "tool_call_id": "c", "content": "é".repeat(5_000) + "\n" + &"日本語\n".repeat(1_000)},
        {"role": "user", "content": "go"},
    ]});
    let done = shrink(&mut body, 600).unwrap();
    assert_eq!(done.results, 1);
    let cut = body["messages"][0]["content"].as_str().unwrap();
    assert!(cut.contains("Cut by the gateway"));
    assert!(estimate(&body) <= 600);
}
