//! An asked call that meets a 429 holds the account for the companion's own calls (#1386).
//! Nothing here sleeps: time is moved by hand and the upstream records the waits asked of it.

use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::health::Clock;
use crate::hold::Holds;
use crate::http::Request;
use crate::log::CallLog;
use crate::policy::Sharing;
use crate::server::Gateway;
use crate::tests::{answer, request, Fake};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

#[derive(Default)]
struct Hand(AtomicU64);

impl Clock for Hand {
    fn now(&self) -> Duration {
        Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}

/// Answers 429 (naming `named` seconds) to every call while `limited` is set, else 200. Counts sends.
struct Provider {
    limited: Mutex<bool>,
    named: Option<u64>,
    sent: Mutex<usize>,
}

impl Upstream for Provider {
    fn chat(&self, _base_url: &str, _key: Option<&str>, _body: &[u8]) -> Result<Reply, Unreachable> {
        *self.sent.lock().unwrap() += 1;
        let (status, body) = if *self.limited.lock().unwrap() {
            (429, json!({"error": {"message": "rate limit"}}))
        } else {
            (200, json!({"choices": [{"message": {"content": "hello"}}]}))
        };
        Ok(Reply { status, content_type: "application/json".into(), retry_after: self.named.filter(|_| status == 429).map(Duration::from_secs), body: Box::new(Cursor::new(body.to_string().into_bytes())) })
    }
    fn wait(&self, _d: Duration) {}
}

struct Rig {
    g: Gateway,
    up: Arc<Provider>,
    clock: Arc<Hand>,
    companion: String,
    pi: String,
}

fn rig(named: Option<u64>) -> Rig {
    let tokens = Arc::new(Tokens::in_memory());
    let (companion, pi) = (mint().unwrap(), mint().unwrap());
    tokens.register(&companion, Grant { harness: crate::asked::COMPANION.into(), private_context: true }).unwrap();
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    let up = Arc::new(Provider { limited: Mutex::new(false), named, sent: Mutex::new(0) });
    let clock = Arc::new(Hand::default());
    let g = Gateway {
        tokens,
        accounts: Arc::new(Fake { private_mode: false, cloud: Sharing::Share, lane: None }),
        upstream: up.clone(),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(),
        params_refused: Default::default(),
        health: Default::default(),
        holds: Arc::new(Holds::with_clock(clock.clone())),
    };
    Rig { g, up, clock, companion, pi }
}

fn chat(token: &str, account: &str, mark: bool) -> Request {
    let mut req = request("POST", "/v1/chat/completions", Some(token), json!({"model": format!("{account}/gpt-oss-120b"), "messages": [{"role": "user", "content": "hi"}]}));
    if mark {
        req.headers.push((yantrik_ml::asked::HEADER.to_string(), "self".to_string()));
    }
    req
}

fn refused_as_kept(status: u16, body: &str) -> bool {
    status == 429 && serde_json::from_str::<Value>(body).is_ok_and(|v| v["error"]["code"] == "kept_for_asked")
}

#[test]
fn an_asked_429_holds_the_companions_own_calls_to_that_account() {
    let r = rig(Some(20));
    *r.up.limited.lock().unwrap() = true;
    let (status, _) = answer(&r.g, &chat(&r.pi, "cloud", false));
    assert_eq!(status, 429);
    *r.up.limited.lock().unwrap() = false;
    let before = *r.up.sent.lock().unwrap();
    let (status, body) = answer(&r.g, &chat(&r.companion, "cloud", true));
    assert!(refused_as_kept(status, &body), "{status} {body}");
    assert!(body.contains("kept for what you ask"), "{body}");
    assert_eq!(*r.up.sent.lock().unwrap(), before, "nothing was sent to the provider");
}

#[test]
fn the_hold_is_for_that_account_only() {
    let r = rig(Some(20));
    *r.up.limited.lock().unwrap() = true;
    answer(&r.g, &chat(&r.pi, "cloud", false));
    *r.up.limited.lock().unwrap() = false;
    let (status, body) = answer(&r.g, &chat(&r.companion, "elsewhere", true));
    assert_eq!(status, 200, "{body}");
}

#[test]
fn the_hold_ends_after_the_named_window_and_after_sixty_seconds_when_none_is_named() {
    for (named, window) in [(Some(20), 20), (None, 60)] {
        let r = rig(named);
        *r.up.limited.lock().unwrap() = true;
        answer(&r.g, &chat(&r.pi, "cloud", false));
        *r.up.limited.lock().unwrap() = false;
        r.clock.0.store(window - 1, Ordering::SeqCst);
        let (status, body) = answer(&r.g, &chat(&r.companion, "cloud", true));
        assert!(refused_as_kept(status, &body), "still held a second before {window} s: {status}");
        r.clock.0.store(window + 1, Ordering::SeqCst);
        let (status, body) = answer(&r.g, &chat(&r.companion, "cloud", true));
        assert_eq!(status, 200, "{body}");
    }
}

#[test]
fn asked_calls_are_never_held_by_this_rule() {
    let r = rig(Some(20));
    *r.up.limited.lock().unwrap() = true;
    answer(&r.g, &chat(&r.pi, "cloud", false));
    *r.up.limited.lock().unwrap() = false;
    for token in [&r.pi, &r.companion] {
        let (status, body) = answer(&r.g, &chat(token, "cloud", false));
        assert_eq!(status, 200, "{body}");
    }
}

#[test]
fn a_companion_call_that_meets_a_429_holds_nothing() {
    let r = rig(Some(20));
    *r.up.limited.lock().unwrap() = true;
    let (status, _) = answer(&r.g, &chat(&r.companion, "cloud", true));
    assert_eq!(status, 429);
    assert!(r.g.holds.remaining("cloud").is_none());
}

#[test]
fn a_429_that_goes_back_is_the_gateways_own_sentence_and_nothing_else_of_the_upstream() {
    let r = rig(Some(20));
    *r.up.limited.lock().unwrap() = true;
    let (status, body) = answer(&r.g, &chat(&r.pi, "cloud", false));
    assert_eq!(status, 429);
    let v: Value = serde_json::from_str(&body).unwrap();
    let m = v["error"]["message"].as_str().unwrap();
    assert!(m.contains("is rate-limited right now (too many requests). It said: rate limit"), "{m}");
    assert_eq!(v["error"].as_object().unwrap().len(), 3, "{body}");
    assert_eq!(v.as_object().unwrap().len(), 1, "{body}");
}

#[test]
fn the_upstream_reason_is_cut_and_its_other_fields_are_dropped() {
    let long = "x".repeat(500);
    let reply = Reply { status: 429, content_type: "text/plain".into(), retry_after: None, body: Box::new(Cursor::new(json!({"error": {"message": long, "secret": "k"}, "extra": 1}).to_string().into_bytes())) };
    let out = crate::wait::in_our_words(reply, "Groq (free)");
    let mut s = String::new();
    std::io::Read::read_to_string(&mut { out.body }, &mut s).unwrap();
    assert!(!s.contains("secret") && !s.contains("extra"), "{s}");
    assert!(s.contains(&"x".repeat(200)) && !s.contains(&"x".repeat(201)), "{s}");
    assert!(s.contains("Groq (free) is rate-limited"), "{s}");
}
