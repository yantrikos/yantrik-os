//! A 429 that names a short wait is waited out and sent again; none of these sleeps (the fake
//! upstream records the wait asked for).

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::log::{Call, CallLog};
use crate::policy::{Refusal, Sharing, Target};
use crate::server::{Accounts, Gateway, ModelInfo};
use crate::tests::{answer, request, Fake};
use crate::tokens::{mint, Grant, Tokens};
use crate::upstream::{Reply, Unreachable, Upstream};

const SSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";

/// What one send is answered with: its status, its content type, the wait it names, its body.
type Step = (u16, &'static str, Option<u64>, String);

fn ok(n: usize) -> Step {
    (200, "application/json", None, json!({"choices": [{"message": {"content": format!("answer {n}")}}]}).to_string())
}

fn limited(secs: Option<u64>) -> Step {
    (429, "application/json", secs, json!({"error": {"message": "API rate limit exceeded"}}).to_string())
}

/// Answers its steps in order; records what it was sent and every wait asked of it. `on_wait`
/// is what changes while it waits.
struct Scripted {
    steps: Mutex<Vec<Step>>,
    sent: Mutex<Vec<Vec<u8>>>,
    waits: Mutex<Vec<Duration>>,
    on_wait: Box<dyn Fn() + Send + Sync>,
}

impl Upstream for Scripted {
    fn chat(&self, _base_url: &str, _key: Option<&str>, body: &[u8]) -> Result<Reply, Unreachable> {
        self.sent.lock().unwrap().push(body.to_vec());
        let (status, ctype, wait, text) = self.steps.lock().unwrap().remove(0);
        Ok(Reply { status, content_type: ctype.into(), retry_after: wait.map(Duration::from_secs), body: Box::new(Cursor::new(text.into_bytes())) })
    }
    fn wait(&self, d: Duration) {
        self.waits.lock().unwrap().push(d);
        (self.on_wait)();
    }
}

/// The account's state once a wait has happened.
#[derive(Clone, Copy)]
enum Meanwhile {
    Nothing,
    PrivateMode,
    TierOff,
}

struct Changing {
    inner: Fake,
    after_wait: Arc<AtomicBool>,
    change: Meanwhile,
}

impl Accounts for Changing {
    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        if matches!(self.change, Meanwhile::TierOff) && self.after_wait.load(Ordering::SeqCst) {
            return Err(Refusal::new(403, "tier_off", "It isn't offered any more. Nothing was sent to it."));
        }
        self.inner.target(account, model)
    }
    fn private_mode(&self) -> bool {
        matches!(self.change, Meanwhile::PrivateMode) && self.after_wait.load(Ordering::SeqCst)
    }
}

fn rig(steps: Vec<Step>, change: Meanwhile) -> (Gateway, Arc<Scripted>, String) {
    let flag = Arc::new(AtomicBool::new(false));
    let on_wait = flag.clone();
    let upstream = Arc::new(Scripted { steps: Mutex::new(steps), sent: Default::default(), waits: Default::default(), on_wait: Box::new(move || on_wait.store(true, Ordering::SeqCst)) });
    let tokens = Arc::new(Tokens::in_memory());
    let pi = mint().unwrap();
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    let g = Gateway {
        tokens,
        accounts: Arc::new(Changing { inner: Fake { private_mode: false, cloud: Sharing::Share, lane: None }, after_wait: flag, change }),
        upstream: upstream.clone(),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(),
        params_refused: Default::default(), health: Default::default(),
    };
    (g, upstream, pi)
}

fn ask(g: &Gateway, pi: &str, stream: bool) -> (u16, String) {
    let body = json!({"model": "cloud/gpt-oss-120b", "stream": stream, "messages": [{"role": "user", "content": "hi"}]});
    answer(g, &request("POST", "/v1/chat/completions", Some(pi), body))
}

fn last(g: &Gateway) -> Call {
    g.log.recent().pop().unwrap()
}

fn secs(d: &[Duration]) -> Vec<u64> {
    d.iter().map(|d| d.as_secs()).collect()
}

#[test]
fn a_named_wait_is_waited_out_and_the_same_bytes_go_again() {
    let (g, up, pi) = rig(vec![limited(Some(4)), ok(1)], Meanwhile::Nothing);
    let (status, body) = ask(&g, &pi, false);
    assert_eq!((status, body.contains("answer 1")), (200, true), "{body}");
    let waits = up.waits.lock().unwrap().clone();
    assert_eq!(waits.len(), 1);
    assert!(waits[0] >= Duration::from_secs(4));
    let sent = up.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], sent[1]);
    let row = last(&g);
    assert_eq!((row.status, row.outcome.as_str()), (200, ""));
    assert!(row.waited_ms >= 4000, "{}", row.waited_ms);
}

#[test]
fn a_429_naming_no_wait_goes_to_the_caller_whole() {
    let (g, up, pi) = rig(vec![limited(None)], Meanwhile::Nothing);
    let (status, body) = ask(&g, &pi, false);
    assert_eq!(status, 429);
    assert!(body.contains("API rate limit exceeded"), "{body}");
    assert_eq!((up.waits.lock().unwrap().len(), up.sent.lock().unwrap().len()), (0, 1));
    let row = last(&g);
    assert_eq!((row.status, row.outcome.as_str(), row.waited_ms), (429, "upstream_429", 0));
}

#[test]
fn a_wait_past_the_budget_is_not_waited() {
    let (g, up, pi) = rig(vec![limited(Some(300))], Meanwhile::Nothing);
    let (status, body) = ask(&g, &pi, false);
    assert_eq!(status, 429);
    assert!(body.contains("API rate limit exceeded"), "{body}");
    assert_eq!((up.waits.lock().unwrap().len(), up.sent.lock().unwrap().len()), (0, 1));
}

#[test]
fn the_budget_is_for_the_whole_call() {
    let (g, up, pi) = rig(vec![limited(Some(40)), limited(Some(40))], Meanwhile::Nothing);
    let (status, body) = ask(&g, &pi, false);
    assert_eq!(status, 429);
    assert!(body.contains("API rate limit exceeded"), "{body}");
    assert_eq!((secs(&up.waits.lock().unwrap()), up.sent.lock().unwrap().len()), (vec![40], 2));
    let row = last(&g);
    assert_eq!((row.status, row.outcome.as_str()), (429, "upstream_429"));
    assert!(row.waited_ms >= 40_000);
}

#[test]
fn two_waits_in_a_row_end_in_the_answer() {
    let (g, up, pi) = rig(vec![limited(Some(4)), limited(Some(4)), ok(7)], Meanwhile::Nothing);
    let (status, body) = ask(&g, &pi, false);
    assert_eq!((status, body.contains("answer 7")), (200, true), "{body}");
    assert_eq!((up.waits.lock().unwrap().len(), up.sent.lock().unwrap().len()), (2, 3));
    assert!(last(&g).waited_ms >= 8000);
}

#[test]
fn a_provider_that_keeps_naming_no_time_at_all_is_not_asked_for_ever() {
    let (g, up, pi) = rig((0..50).map(|_| limited(Some(0))).collect(), Meanwhile::Nothing);
    let (status, body) = ask(&g, &pi, false);
    assert_eq!(status, 429);
    assert!(body.contains("API rate limit exceeded"), "{body}");
    let sent = up.sent.lock().unwrap().len();
    assert_eq!((up.waits.lock().unwrap().len(), sent), (crate::wait::MAX_WAITS as usize, crate::wait::MAX_WAITS as usize + 1));
    assert_eq!(last(&g).outcome, "upstream_429");
}

/// Review of #1297 (M1): the sum of the waits used to panic the connection's thread here, and the
/// caller read a closed connection with no row in the log.
#[test]
fn a_later_wait_near_the_largest_number_is_passed_on_and_not_added_up() {
    let (g, up, pi) = rig(vec![limited(Some(4)), limited(Some(u64::MAX))], Meanwhile::Nothing);
    let (status, body) = ask(&g, &pi, false);
    assert_eq!(status, 429);
    assert!(body.contains("API rate limit exceeded"), "{body}");
    assert_eq!((secs(&up.waits.lock().unwrap()), up.sent.lock().unwrap().len()), (vec![4], 2));
    assert_eq!(g.log.recent().len(), 1);
    assert_eq!(last(&g).outcome, "upstream_429");
}

#[test]
fn what_changed_during_the_wait_is_what_the_caller_reads() {
    for (change, want_status, want_code) in [(Meanwhile::PrivateMode, 403, "private_mode"), (Meanwhile::TierOff, 403, "tier_off")] {
        let (g, up, pi) = rig(vec![limited(Some(4)), ok(1)], change);
        let (status, body) = ask(&g, &pi, false);
        assert_eq!(status, want_status, "{body}");
        assert!(body.contains(want_code), "{body}");
        assert_eq!(up.sent.lock().unwrap().len(), 1, "nothing more was sent");
        let row = last(&g);
        assert_eq!((row.status, row.outcome.as_str()), (want_status, want_code));
        assert!(row.waited_ms > 0);
    }
}

/// Two calls a minute, each further call answered 429 with the seconds left in the window.
struct Window {
    used: Mutex<(u32, u64)>,
    served: Mutex<u32>,
}

impl Upstream for Window {
    fn chat(&self, _base_url: &str, _key: Option<&str>, _body: &[u8]) -> Result<Reply, Unreachable> {
        let mut w = self.used.lock().unwrap();
        let step = if w.0 < 2 {
            w.0 += 1;
            let mut n = self.served.lock().unwrap();
            *n += 1;
            ok(*n as usize)
        } else {
            limited(Some(60u64.saturating_sub(w.1 / 1000)))
        };
        Ok(Reply { status: step.0, content_type: step.1.into(), retry_after: step.2.map(Duration::from_secs), body: Box::new(Cursor::new(step.3.into_bytes())) })
    }
    fn wait(&self, d: Duration) {
        let mut w = self.used.lock().unwrap();
        w.1 += d.as_millis() as u64;
        if w.1 >= 60_000 {
            *w = (0, 0);
        }
    }
}

#[test]
fn five_two_call_turns_all_answer_in_order() {
    let tokens = Arc::new(Tokens::in_memory());
    let pi = mint().unwrap();
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    let g = Gateway {
        tokens,
        accounts: Arc::new(Fake { private_mode: false, cloud: Sharing::Share, lane: None }),
        // Another computer at the address used one call 51 s ago: the first turn's second call
        // meets a window with 9 s left.
        upstream: Arc::new(Window { used: Mutex::new((1, 51_000)), served: Mutex::new(0) }),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(),
        params_refused: Default::default(), health: Default::default(),
    };
    for n in 1..=10 {
        let (status, body) = ask(&g, &pi, false);
        assert_eq!((status, body.contains(&format!("answer {n}\""))), (200, true), "call {n}: {body}");
    }
    let rows = g.log.recent();
    assert!(rows.iter().all(Call::ok));
    let waited: Vec<u64> = rows.iter().map(|c| c.waited_ms).filter(|ms| *ms > 0).collect();
    assert_eq!(waited.iter().filter(|ms| **ms == 9_250).count(), 1, "the partly used window names 9 s: {waited:?}");
    assert!(waited.len() >= 4 && waited.iter().all(|ms| *ms <= 60_250), "{waited:?}");
}

#[test]
fn a_streamed_call_that_meets_a_429_reads_the_stream_of_the_second_send() {
    let (g, up, pi) = rig(vec![limited(Some(4)), (200, "text/event-stream", None, SSE.to_string())], Meanwhile::Nothing);
    let mut out = Vec::new();
    let body = json!({"model": "cloud/gpt-oss-120b", "stream": true, "messages": [{"role": "user", "content": "hi"}]});
    g.respond(&request("POST", "/v1/chat/completions", Some(&pi), body), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("HTTP/1.1 200") && text.contains("text/event-stream"), "{text}");
    assert!(text.ends_with(SSE), "{text}");
    assert_eq!(up.waits.lock().unwrap().len(), 1);
}
