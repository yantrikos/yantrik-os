//! A request too large for an account is adjusted, not given up (#1390). Nothing here sleeps: the
//! upstream records the waits asked of it, and time is a clock a test moves by hand.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::fall_tests::{raw, Hosts, Manual, Opts, Out, Raw, Rig, Switch};
use crate::fit::{counted_by, estimate, shrink, Room};
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
fn a_call_whose_only_large_part_is_the_newest_tool_result_is_shrunk() {
    let mut body = conversation(10);
    let m = body["messages"].as_array_mut().unwrap();
    m.pop();
    m.push(json!({"role": "tool", "tool_call_id": "c1", "content": page(3_000)}));
    let user = body["messages"][1].clone();
    let done = shrink(&mut body, 1_000).expect("the newest tool result is cut, last");
    assert_eq!(done.results, 1);
    assert!(body["messages"][4]["content"].as_str().unwrap().contains("Cut by the gateway"));
    assert_eq!(body["messages"][1], user, "the person's message is whole");
    assert!(estimate(&body) <= 1_000);
}

#[test]
fn earlier_results_are_cut_before_the_newest_one() {
    let mut body = conversation(100);
    body["messages"].as_array_mut().unwrap().pop();
    body["messages"].as_array_mut().unwrap().push(json!({"role": "tool", "tool_call_id": "c2", "content": page(4_000)}));
    let newest = body["messages"][4]["content"].clone();
    let limit = estimate(&body) - 300;
    shrink(&mut body, limit).unwrap();
    assert_eq!(body["messages"][4]["content"], newest, "the newest tool result is kept while an earlier one can give");
    assert!(body["messages"][3]["content"].as_str().unwrap().contains("Cut by the gateway"));
}

#[test]
fn a_tool_result_sent_as_text_parts_is_cut_part_by_part() {
    let mut body = conversation(1);
    body["messages"][3]["content"] = json!([{"type": "text", "text": page(3_000)}, {"type": "text", "text": "short"}]);
    let done = shrink(&mut body, 1_000).unwrap();
    assert_eq!(done.results, 1);
    let parts = body["messages"][3]["content"].as_array().unwrap();
    assert!(parts[0]["text"].as_str().unwrap().contains("Cut by the gateway"));
    assert_eq!(parts[1]["text"], "short");
    assert_eq!(parts[0]["type"], "text");
    assert!(estimate(&body) <= 1_000);
}

#[test]
fn a_pi_shaped_call_to_an_8000_tpm_account_is_sent_not_refused() {
    let r = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", Room { per_minute: Some(8_000), context: Some(131_072) })]);
    let mut body = conversation(1);
    body["max_completion_tokens"] = json!(32_768);
    body["messages"][1]["content"] = json!(page(560));
    let one = estimate(&body);
    assert!(one > 6_000 && one < 8_000, "{one}");
    let a = ask(&r, body.clone());
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(sent(&r, 0)["messages"], body["messages"]);
    assert!(last(&r).shrunk.is_none());
    // Even one that looks over, with nothing to cut, is sent: only the account's 413 decides.
    body["messages"][1]["content"] = json!(page(1_000));
    assert!(estimate(&body) > 8_000);
    let a = ask(&r, body);
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(r.up.count("cloud"), 2);
}

#[test]
fn the_minutes_ledger_takes_the_real_prompt_tokens_in_place_of_the_estimate() {
    use crate::upstream::{Reply, Unreachable, Upstream};
    struct Usage;
    impl Upstream for Usage {
        fn chat(&self, _b: &str, _k: Option<&str>, _body: &[u8]) -> Result<Reply, Unreachable> {
            let text = json!({"choices": [{"message": {"content": "ok"}}], "usage": {"prompt_tokens": 400, "completion_tokens": 5}});
            Ok(Reply { status: 200, content_type: "application/json".into(), retry_after: None, body: Box::new(std::io::Cursor::new(text.to_string().into_bytes())) })
        }
        fn wait(&self, _d: Duration) {
            panic!("it must not wait: the real count is small");
        }
    }
    let mut r = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", tpm(3_000))]);
    r.g.upstream = Arc::new(Usage);
    let body = conversation(150);
    assert!(estimate(&body) * 2 > 3_000, "two estimates would not fit a minute");
    assert_eq!(ask(&r, body.clone()).status, 200);
    assert_eq!(ask(&r, body).status, 200);
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

fn huge_user_call() -> Value {
    let mut body = conversation(1);
    body["messages"][1]["content"] = json!(page(2_000));
    body
}

#[test]
fn a_call_nothing_can_be_cut_from_is_sent_and_the_accounts_413_ends_it_in_our_words() {
    let r = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", tpm(500))]);
    r.up.set("cloud", &[Out::Status(413)]);
    let a = ask(&r, huge_user_call());
    assert_eq!(a.status, 413, "{}", a.body);
    let v: Value = serde_json::from_str(&a.body).unwrap();
    assert_eq!(v["error"]["code"], "upstream_413");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("cloud provider") && msg.contains("too long") && msg.contains("new chat"), "{msg}");
    assert!(!a.body.contains(SECRET) && !a.body.contains("status 413"));
    assert_eq!(r.up.count("cloud"), 1, "sent once; the estimate refused nothing, and nothing was left to cut");
    assert_eq!(last(&r).outcome, "upstream_413");
}

#[test]
fn a_413_from_the_account_moves_the_call_to_the_private_lane_when_allowed_and_says_so() {
    let r = rig(Opts::default(), &[("cloud", tpm(500))]);
    r.up.set("cloud", &[Out::Status(413)]);
    let a = ask(&r, huge_user_call());
    assert_eq!(a.status, 200, "{}", a.body);
    assert!(a.body.contains("from home"), "{}", a.body);
    assert_eq!(a.header("x-yantrik-answered-by").unwrap(), "home/qwen3:8b");
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud", "home"]);
    assert_eq!(last(&r).outcome, "fell_to_lane+upstream_413");
}

#[test]
fn a_move_needs_the_person_to_allow_it_and_a_lane_with_room() {
    let off = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", tpm(500))]);
    off.up.set("cloud", &[Out::Status(413)]);
    assert_eq!(ask(&off, huge_user_call()).status, 413);
    assert_eq!(off.up.count("home"), 0);
    let tight = rig(Opts::default(), &[("cloud", tpm(500)), ("home", tpm(500))]);
    tight.up.set("cloud", &[Out::Status(413)]);
    let a = ask(&tight, huge_user_call());
    assert_eq!(a.status, 413, "{}", a.body);
    assert_eq!(tight.up.count("home"), 0, "a lane that cannot take it either is not asked");
    let none = rig(Opts { lane: None, ..Opts::default() }, &[("cloud", tpm(500))]);
    none.up.set("cloud", &[Out::Status(413)]);
    assert_eq!(ask(&none, huge_user_call()).status, 413);
}

#[test]
fn a_local_account_is_never_moved_to_a_less_private_one() {
    // The lane is the only place a call moves to, and only a local one: a call asked of the
    // private model itself has nowhere else to go.
    let r = rig(Opts::default(), &[("home", tpm(500))]);
    r.up.set("home", &[Out::Status(413)]);
    let mut body = huge_user_call();
    body["model"] = json!("home/qwen3:8b");
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

#[test]
fn the_numbers_an_account_gives_in_a_413_are_read_and_nothing_else() {
    let groq = r#"{"error":{"message":"Request too large for model `openai/gpt-oss-20b` on tokens per minute (TPM): Limit 8000, Requested 9,500, please reduce your message size","type":"tokens"}}"#;
    assert_eq!(counted_by(groq), (Some(9500), Some(8000)));
    let openai = "This model's maximum context length is 8192 tokens. However, your messages resulted in 9100 tokens.";
    assert_eq!(counted_by(openai), (Some(9100), Some(8192)));
    assert_eq!(counted_by("status 413"), (None, None), "an account that states neither leaves both out");
    assert_eq!(counted_by("Limit abc"), (None, None));
}

#[test]
fn a_413_keeps_what_the_account_said_it_counted_and_its_limit_in_the_call_row_and_no_words() {
    let r = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", tpm(500))]);
    r.up.set("cloud", &[Out::Said(413, r#"{"error":{"message":"Limit 8000, Requested 9500 SECRET-WORDS"}}"#)]);
    let a = ask(&r, huge_user_call());
    assert_eq!(a.status, 413, "{}", a.body);
    let row = last(&r);
    assert_eq!((row.counted, row.limit), (Some(9500), Some(8000)));
    let line = serde_json::to_string(&row).unwrap();
    assert!(line.contains("\"counted\":9500") && line.contains("\"limit\":8000") && !line.contains("SECRET-WORDS"), "{line}");
    assert!(!a.body.contains("SECRET-WORDS"), "the provider's words do not go back either: {}", a.body);
    let plain = rig(Opts { fall: false, ..Opts::default() }, &[("cloud", tpm(500))]);
    plain.up.set("cloud", &[Out::Status(413)]);
    ask(&plain, huge_user_call());
    let line = serde_json::to_string(&last(&plain)).unwrap();
    assert!(!line.contains("counted") && !line.contains("limit"), "absent when the account stated nothing: {line}");
}
