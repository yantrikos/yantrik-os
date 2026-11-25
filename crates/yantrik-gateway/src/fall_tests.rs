//! The fall to the private lane: what makes a call fall, and what it says when it does. Nothing
//! here sleeps: the upstream counts the waits asked of it, and time for the health record is a
//! clock a test moves by hand. The refusals that must never fall are in `fall_gate_tests`.

use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::health::{Clock, Health};
use crate::log::{Call, CallLog};
use crate::policy::{Refusal, Sharing, Target};
use crate::server::{Accounts, Gateway, ModelInfo};
use crate::tests::{request, Fake};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

/// What one send to a host is answered with.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Out {
    Down,
    Status(u16),
    Ok,
    /// A stream that starts and is then cut.
    Cut,
    /// A 429 naming a wait of this many seconds.
    Limited(u64),
    /// A stream cut in the middle of an event, with no line end.
    CutMid,
    /// This status, naming a wait of this many seconds.
    Busy(u16, u64),
    /// This status, with this body.
    Said(u16, &'static str),
}

/// Answers each host from its script (the last step repeats), recording who was sent to.
#[derive(Default)]
pub(crate) struct Hosts {
    script: Mutex<HashMap<&'static str, VecDeque<Out>>>,
    pub(crate) sent: Mutex<Vec<&'static str>>,
    pub(crate) waits: Mutex<Vec<Duration>>,
    pub(crate) bodies: Mutex<Vec<(&'static str, serde_json::Value)>>,
}

fn host_of(base_url: &str) -> &'static str {
    if base_url.contains("cloud.example") {
        "cloud"
    } else if base_url.contains("typed.example") {
        "typed"
    } else if base_url.contains("127.0.0.1") {
        "home"
    } else {
        "elsewhere"
    }
}

struct Cutting(bool, &'static [u8]);

impl Read for Cutting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0 {
            return Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset"));
        }
        self.0 = true;
        let part = self.1;
        buf[..part.len()].copy_from_slice(part);
        Ok(part.len())
    }
}

impl Hosts {
    pub(crate) fn set(&self, host: &'static str, steps: &[Out]) {
        self.script.lock().unwrap().insert(host, steps.iter().copied().collect());
    }
    pub(crate) fn count(&self, host: &str) -> usize {
        self.sent.lock().unwrap().iter().filter(|h| **h == host).count()
    }
}

impl Upstream for Hosts {
    fn chat(&self, base_url: &str, _key: Option<&str>, body: &[u8]) -> Result<Reply, Unreachable> {
        let host = host_of(base_url);
        self.sent.lock().unwrap().push(host);
        self.bodies.lock().unwrap().push((host, serde_json::from_slice(body).unwrap_or_default()));
        let step = {
            let mut script = self.script.lock().unwrap();
            match script.get_mut(host) {
                Some(q) if q.len() > 1 => q.pop_front().unwrap(),
                Some(q) => q.front().copied().unwrap_or(Out::Ok),
                None => Out::Ok,
            }
        };
        let json = |status: u16, text: String, retry: Option<u64>| Reply {
            status,
            content_type: "application/json".into(),
            retry_after: retry.map(Duration::from_secs),
            body: Box::new(Cursor::new(text.into_bytes())),
        };
        match step {
            Out::Down => Err(Unreachable { code: "unreachable", message: format!("Could not reach {host}.") }),
            Out::Ok => Ok(json(200, json!({"choices": [{"message": {"content": format!("from {host}")}}]}).to_string(), None)),
            Out::Status(n) => Ok(json(n, json!({"error": {"message": format!("status {n}")}}).to_string(), None)),
            Out::Limited(s) => Ok(json(429, json!({"error": {"message": "slow down"}}).to_string(), Some(s))),
            Out::Cut => Ok(Reply { status: 200, content_type: "text/event-stream".into(), retry_after: None, body: Box::new(Cutting(false, b"data: {\"choices\":[{\"delta\":{\"content\":\"par\"}}]}\n\n")) }),
            Out::CutMid => Ok(Reply { status: 200, content_type: "text/event-stream".into(), retry_after: None, body: Box::new(Cutting(false, b"data: {\"choices\":[{\"delta\":{\"content\":\"par")) }),
            Out::Said(n, body) => Ok(json(n, body.to_string(), None)),
            Out::Busy(n, s) => Ok(json(n, json!({"error": {"message": "overloaded"}}).to_string(), Some(s))),
        }
    }
    fn wait(&self, d: Duration) {
        self.waits.lock().unwrap().push(d);
    }
}

/// A clock a test moves by hand.
#[derive(Default)]
pub(crate) struct Manual(AtomicU64);

impl Manual {
    pub(crate) fn advance(&self, d: Duration) {
        self.0.fetch_add(d.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for Manual {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

/// `Fake`'s accounts, with the person's setting, and what can change under a call.
pub(crate) struct Switch {
    pub(crate) inner: Fake,
    pub(crate) fall: Arc<AtomicBool>,
    /// `cloud` is looked up this many times before it is switched off (`usize::MAX`: never).
    pub(crate) cloud_off_from: usize,
    pub(crate) cloud_asks: AtomicUsize,
    pub(crate) cloud_tight: bool,
    pub(crate) lane_tight: bool,
}

impl Accounts for Switch {
    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        if account == "cloud" && self.cloud_asks.fetch_add(1, Ordering::SeqCst) >= self.cloud_off_from {
            return Err(Refusal::new(403, "tier_off", "It isn't offered any more. Nothing was sent to it."));
        }
        let mut t = self.inner.target(account, model)?;
        t.tight = (account == "cloud" && self.cloud_tight) || (account == "home" && self.lane_tight);
        Ok(t)
    }
    fn private_mode(&self) -> bool {
        self.inner.private_mode()
    }
    fn private_lane(&self) -> Option<String> {
        self.inner.private_lane()
    }
    fn fall_to_lane(&self) -> bool {
        self.fall.load(Ordering::SeqCst)
    }
}

pub(crate) struct Rig {
    pub(crate) g: Gateway,
    pub(crate) up: Arc<Hosts>,
    pub(crate) fall: Arc<AtomicBool>,
    pub(crate) clock: Arc<Manual>,
    pub(crate) pi: String,
    pub(crate) mind: String,
    pub(crate) companion: String,
}

pub(crate) const LANE: &str = "home/qwen3:8b";

pub(crate) struct Opts {
    pub(crate) lane: Option<&'static str>,
    pub(crate) fall: bool,
    pub(crate) private_mode: bool,
    pub(crate) cloud: Sharing,
    pub(crate) cloud_off_from: usize,
    pub(crate) cloud_tight: bool,
    pub(crate) lane_tight: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { lane: Some(LANE), fall: true, private_mode: false, cloud: Sharing::Share, cloud_off_from: usize::MAX, cloud_tight: false, lane_tight: false }
    }
}

pub(crate) fn rig(o: Opts) -> Rig {
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
        cloud_off_from: o.cloud_off_from,
        cloud_asks: AtomicUsize::new(0),
        cloud_tight: o.cloud_tight,
        lane_tight: o.lane_tight,
    };
    let mut g = Gateway {
        tokens,
        accounts: Arc::new(switch),
        upstream: up.clone(),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(), params_refused: Default::default(), health: Default::default(), holds: Default::default(),
    };
    g.health = Arc::new(Health::with_clock(clock.clone()));
    Rig { g, up, fall, clock, pi, mind, companion }
}

/// What the gateway wrote: status, the head, the body.
pub(crate) struct Raw {
    pub(crate) status: u16,
    pub(crate) head: String,
    pub(crate) body: String,
}

impl Raw {
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let prefix = format!("{name}: ");
        self.head.lines().find_map(|l| l.strip_prefix(&prefix)).map(str::to_string)
    }
}

pub(crate) fn raw(g: &Gateway, req: &crate::http::Request) -> Raw {
    let mut out = Vec::new();
    g.respond(req, &mut out);
    let text = String::from_utf8(out).unwrap();
    let status = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    Raw { status, head: head.to_string(), body: body.to_string() }
}

pub(crate) fn ask_as(r: &Rig, token: &str, model: &str, mark: bool) -> Raw {
    let mut req = request("POST", "/v1/chat/completions", Some(token), json!({"model": model, "messages": [{"role": "user", "content": "hi"}]}));
    if mark {
        req.headers.push((yantrik_ml::asked::HEADER.to_string(), yantrik_ml::asked::BY_ITSELF.to_string()));
    }
    raw(&r.g, &req)
}

pub(crate) fn ask(r: &Rig) -> Raw {
    ask_as(r, &r.pi, "cloud/gpt-oss-120b", false)
}

pub(crate) fn last(r: &Rig) -> Call {
    r.g.log.recent().pop().unwrap()
}

#[test]
fn an_account_that_is_unreachable_before_the_first_byte_falls_to_the_lane_and_the_lane_answers() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    let a = ask(&r);
    assert_eq!(a.status, 200, "{}", a.body);
    assert!(a.body.contains("from home"), "{}", a.body);
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud", "home"]);
}

#[test]
fn a_5xx_from_the_account_falls_to_the_lane() {
    for status in [500, 502, 503, 529] {
        let r = rig(Opts::default());
        r.up.set("cloud", &[Out::Status(status)]);
        let a = ask(&r);
        assert_eq!(a.status, 200, "{status}: {}", a.body);
        assert!(a.body.contains("from home"), "{status}: {}", a.body);
        assert_eq!(last(&r).outcome, format!("fell_to_lane+upstream_{status}"), "{status}: the account's own status stays in the row");
    }
}

#[test]
fn a_4xx_from_the_account_goes_to_the_caller_whole_and_does_not_fall() {
    for status in [400, 401, 403, 404, 422] {
        let r = rig(Opts::default());
        r.up.set("cloud", &[Out::Status(status)]);
        let a = ask(&r);
        assert_eq!(a.status, status, "{}", a.body);
        assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud"], "{status}: nothing went to the lane");
        assert!(!r.g.health.is_down("cloud"), "{status}: an account that answered is not down");
    }
}

#[test]
fn a_429_that_names_a_wait_waits_and_does_not_fall_even_when_the_wait_runs_out() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Limited(4), Out::Ok]);
    let a = ask(&r);
    assert!(a.status == 200 && a.body.contains("from cloud"), "{}", a.body);
    assert_eq!(r.up.waits.lock().unwrap().len(), 1);
    assert_eq!(r.up.count("home"), 0);
    // A window that keeps closing is still the account's answer, never a reason to fall.
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Limited(4)]);
    let a = ask(&r);
    assert_eq!(a.status, 429, "{}", a.body);
    assert_eq!(r.up.count("home"), 0);
    assert_eq!(r.up.count("cloud"), 4, "one send and three waits, the same as without a lane");
}

#[test]
fn a_stream_cut_half_way_is_not_sent_again_and_the_turn_ends_with_a_sentence_that_says_so() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Cut]);
    let a = ask(&r);
    assert!(a.body.contains("par"), "what arrived is kept: {}", a.body);
    assert!(a.body.contains("stream_cut") && a.body.contains("not sent again"), "{}", a.body);
    assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud"], "nothing was re-sent");
    assert_eq!(last(&r).outcome, "upstream_cut_off");
    assert!(!r.g.health.is_down("cloud"), "the account answered; it is not down");
}

#[test]
fn after_a_fall_the_next_calls_skip_the_dead_account_and_pay_no_timeout() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    for _ in 0..6 {
        let a = ask(&r);
        assert!(a.status == 200 && a.body.contains("from home"), "{}", a.body);
    }
    assert_eq!(r.up.count("cloud"), 1, "only the first call tried it");
    assert_eq!(r.up.count("home"), 6);
    assert_eq!(last(&r).outcome, "fell_to_lane");
}

#[test]
fn the_account_is_tried_again_after_the_interval_and_calls_return_to_it() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    ask(&r);
    r.clock.advance(Duration::from_secs(29));
    ask(&r);
    assert_eq!(r.up.count("cloud"), 1, "not yet");
    // It is back, and one ordinary call finds out.
    r.up.set("cloud", &[Out::Ok]);
    r.clock.advance(Duration::from_secs(2));
    let a = ask(&r);
    assert!(a.body.contains("from cloud"), "{}", a.body);
    assert_eq!(a.header("x-yantrik-answered-by").as_deref(), Some("cloud/gpt-oss-120b"));
    assert!(!r.g.health.is_down("cloud"));
    let b = ask(&r);
    assert!(b.body.contains("from cloud"), "{}", b.body);
    assert_eq!(last(&r).outcome, "");
    assert_eq!(r.up.count("cloud"), 3);
}

#[test]
fn a_try_that_fails_again_backs_off_and_only_one_call_is_let_through_per_interval() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    ask(&r);
    r.clock.advance(Duration::from_secs(31));
    let a = ask(&r);
    assert!(a.body.contains("from home"), "the failed try still gets its answer: {}", a.body);
    assert_eq!(r.up.count("cloud"), 2);
    // The interval is now 60 s: 31 s on, nothing is tried; at 61 s, one call is.
    r.clock.advance(Duration::from_secs(31));
    ask(&r);
    assert_eq!(r.up.count("cloud"), 2);
    r.clock.advance(Duration::from_secs(30));
    for _ in 0..4 {
        ask(&r);
    }
    assert_eq!(r.up.count("cloud"), 3, "four calls at once, one real attempt");
}

#[test]
fn a_lane_that_is_itself_down_gives_one_clear_refusal_naming_both() {
    for lane_out in [Out::Down, Out::Status(503)] {
        let r = rig(Opts::default());
        r.up.set("cloud", &[Out::Down]);
        r.up.set("home", &[lane_out]);
        let a = ask(&r);
        assert_eq!(a.status, 502, "{}", a.body);
        assert!(a.body.contains("account_and_lane_down"), "{}", a.body);
        assert!(a.body.contains("cloud") && a.body.contains("home/qwen3:8b"), "{}", a.body);
        assert_eq!(*r.up.sent.lock().unwrap(), vec!["cloud", "home"], "one try each, no more");
        let row = last(&r);
        assert_eq!((row.status, row.outcome.as_str(), row.lane_for.clone()), (502, "account_and_lane_down", None));
    }
}

#[test]
fn the_log_row_and_the_headers_say_who_answered_and_who_was_asked() {
    let r = rig(Opts::default());
    r.up.set("cloud", &[Out::Down]);
    let a = ask(&r);
    assert_eq!(a.header("x-yantrik-answered-by").as_deref(), Some(LANE));
    assert_eq!(a.header("x-yantrik-fell-from").as_deref(), Some("cloud/gpt-oss-120b"));
    let row = last(&r);
    assert_eq!((row.account.as_str(), row.model.as_str()), ("home", "qwen3:8b"));
    assert_eq!(row.lane_for.as_deref(), Some("cloud/gpt-oss-120b"));
    assert_eq!((row.outcome.as_str(), row.status), ("fell_to_lane", 200));
    // A privacy lane has `lane_for` and no such outcome: the two are told apart.
    let r = rig(Opts { cloud: Sharing::Lane, ..Opts::default() });
    let a = ask_as(&r, &r.mind, "cloud/gpt-oss-120b", false);
    assert_eq!(a.status, 200, "{}", a.body);
    let row = last(&r);
    assert!(row.lane_for.is_some() && row.outcome.is_empty(), "{row:?}");
    assert!(a.header("x-yantrik-fell-from").is_none());
    // A call the account answered names the account.
    let r = rig(Opts::default());
    assert_eq!(ask(&r).header("x-yantrik-answered-by").as_deref(), Some("cloud/gpt-oss-120b"));
}
