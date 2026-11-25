//! The day's budget (#1400): the ledger across a restart, the companion's share, a call the day
//! cannot take moved to a sibling model or refused at once, the answer length fitted, and that
//! nothing of any call's content is in a log row or the state file. Time is a clock a test moves.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::budget::{Budget, DayEnd, Daily};
use crate::fall_tests::{raw, Hosts, Manual, Out, Raw, Rig, Switch};
use crate::fit::{clamp_answer, Room};
use crate::health::Health;
use crate::hold::Holds;
use crate::log::{Call, CallLog};
use crate::policy::{Refusal, Target};
use crate::server::{Accounts, Gateway, ModelInfo};
use crate::tests::{request, Fake};
use crate::tokens::{mint, Grant, Tokens};

const LIMIT: u64 = 200_000;
const SECRET: &str = "the-secret-words-of-a-page";
/// An answer that reports its usage.
const USED_50K: &str = r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":50000,"completion_tokens":5000}}"#;

/// `Switch`'s accounts, with a second model on `cloud` and Groq's daily limits on both.
struct Days {
    inner: Switch,
    context: Option<u64>,
    /// What the second model on `cloud` can do.
    twin: fn(&mut yantrik_ml::model_caps::ModelCaps),
}

impl Accounts for Days {
    fn models(&self) -> Vec<ModelInfo> {
        let mut all = self.inner.models();
        let mut twin = all[0].clone();
        twin.id = "cloud/gpt-oss-20b".into();
        twin.name = "gpt-oss-20b".into();
        (self.twin)(&mut twin.caps);
        all.insert(1, twin);
        all
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
        if account == "cloud" { Room { per_minute: None, context: self.context } } else { Room::default() }
    }
    fn daily(&self, account: &str, _model: &str) -> Daily {
        if account == "cloud" { Daily { tokens: Some(LIMIT), requests: Some(1_000), per_account: false, end: DayEnd::UtcMidnight } } else { Daily::default() }
    }
}

/// 18:00 UTC on some day: six hours before it ends.
fn evening() -> i64 {
    let now = 1_764_000_000;
    now - now % 86_400 + 18 * 3600
}

fn state_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gateway-day-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("gateway-day.json")
}

fn rig(path: Option<PathBuf>, wall: Arc<AtomicI64>, context: Option<u64>) -> Rig {
    rig_with(path, wall, context, |_| {})
}

fn rig_with(path: Option<PathBuf>, wall: Arc<AtomicI64>, context: Option<u64>, twin: fn(&mut yantrik_ml::model_caps::ModelCaps)) -> Rig {
    let tokens = Arc::new(Tokens::in_memory());
    let (pi, mind, companion) = (mint().unwrap(), mint().unwrap(), mint().unwrap());
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    tokens.register(&mind, Grant { harness: "mind".into(), private_context: true }).unwrap();
    tokens.register(&companion, Grant { harness: crate::asked::COMPANION.into(), private_context: true }).unwrap();
    let up = Arc::new(Hosts::default());
    let clock = Arc::new(Manual::default());
    let switch = Switch {
        inner: Fake { private_mode: false, cloud: crate::policy::Sharing::Share, lane: None },
        fall: Arc::new(AtomicBool::new(false)),
        cloud_off_from: usize::MAX,
        cloud_asks: AtomicUsize::new(0),
        cloud_tight: false,
        lane_tight: false,
    };
    let wall: Arc<dyn Fn() -> i64 + Send + Sync> = Arc::new(move || wall.load(Ordering::SeqCst));
    let budget = match path {
        Some(p) => Budget::at_with(p, wall),
        None => Budget::in_memory(),
    };
    let g = Gateway {
        tokens,
        accounts: Arc::new(Days { inner: switch, context, twin }),
        upstream: up.clone(),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(),
        params_refused: Default::default(),
        health: Arc::new(Health::with_clock(clock.clone())),
        holds: Arc::new(Holds::with_budget(clock.clone(), budget)),
    };
    Rig { g, up, fall: Arc::new(AtomicBool::new(false)), clock, pi, mind, companion }
}

/// A gateway whose day ledger is kept in a file of its own, read at the clock `wall`.
fn plain() -> (Rig, PathBuf) {
    plain_with(|_| {})
}

fn plain_with(twin: fn(&mut yantrik_ml::model_caps::ModelCaps)) -> (Rig, PathBuf) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = state_path(&format!("plain{}", N.fetch_add(1, Ordering::SeqCst)));
    (rig_with(Some(path.clone()), Arc::new(AtomicI64::new(evening())), None, twin), path)
}

fn ask(r: &Rig, token: &str, model: &str, extra: Value, mark: bool) -> Raw {
    let mut body = json!({"model": model, "messages": [{"role": "user", "content": format!("hi {SECRET}")}]});
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let mut req = request("POST", "/v1/chat/completions", Some(token), body);
    if mark {
        req.headers.push((yantrik_ml::asked::HEADER.to_string(), yantrik_ml::asked::BY_ITSELF.to_string()));
    }
    raw(&r.g, &req)
}

fn last(r: &Rig) -> Call {
    r.g.log.recent().pop().unwrap()
}

fn today(r: &Rig, key: &str) -> crate::budget::Today {
    r.g.holds.day.today(key, DayEnd::UtcMidnight)
}

#[test]
fn the_ledger_survives_a_restart_and_starts_a_new_day_empty() {
    let path = state_path("restart");
    let wall = Arc::new(AtomicI64::new(evening()));
    let w: Arc<dyn Fn() -> i64 + Send + Sync> = { let wall = wall.clone(); Arc::new(move || wall.load(Ordering::SeqCst)) };
    let first = Budget::at_with(path.clone(), w.clone());
    first.reserve(1, "cloud/gpt-oss-120b", DayEnd::UtcMidnight, 4_000, true);
    first.settle(1, Some(5_500));
    first.reserve(2, "cloud/gpt-oss-120b", DayEnd::UtcMidnight, 1_000, false);
    let again = Budget::at_with(path.clone(), w.clone());
    let t = again.today("cloud/gpt-oss-120b", DayEnd::UtcMidnight);
    assert_eq!((t.tokens, t.requests, t.itself_tokens, t.itself_requests), (6_500, 2, 5_500, 1), "the real count replaced the estimate");
    assert_eq!(t.resets_in.as_secs(), 6 * 3600);
    wall.fetch_add(7 * 3600, Ordering::SeqCst);
    let next = Budget::at_with(path, w).today("cloud/gpt-oss-120b", DayEnd::UtcMidnight);
    assert_eq!((next.tokens, next.requests), (0, 0), "the day ended at midnight UTC");
}

#[test]
fn the_state_file_is_private_and_holds_counts_only() {
    let (r, path) = plain();
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 200, "{}", a.body);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("cloud/gpt-oss-120b"));
    assert!(!text.contains(SECRET) && !text.contains("hi "), "{text}");
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let row = serde_json::to_string(&last(&r)).unwrap();
    assert!(!row.contains(SECRET), "a log row carries no content: {row}");
}

#[test]
fn the_companion_is_refused_past_its_share_while_an_asked_call_still_goes() {
    let (r, _) = plain();
    r.up.set("cloud", &[Out::Said(200, USED_50K)]);
    let first = ask(&r, &r.companion, "cloud/gpt-oss-120b", json!({}), true);
    assert_eq!(first.status, 200, "{}", first.body);
    assert_eq!(today(&r, "cloud/gpt-oss-120b").itself_tokens, 55_000, "the real count, prompt and answer");
    let sent = r.up.count("cloud");
    let kept = ask(&r, &r.companion, "cloud/gpt-oss-120b", json!({}), true);
    assert_eq!(kept.status, 429, "{}", kept.body);
    let said: Value = serde_json::from_str(&kept.body).unwrap();
    assert_eq!(said["error"]["code"], "kept_for_asked");
    let words = said["error"]["message"].as_str().unwrap();
    assert!(words.contains("55K") && words.contains("50K") && words.contains("00:00 UTC") && words.contains("6 h"), "{words}");
    assert_eq!(r.up.count("cloud"), sent, "nothing was sent");
    assert_eq!(last(&r).outcome, "kept_for_asked");
    let asked = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(asked.status, 200, "the person's own call still goes: {}", asked.body);
    assert_eq!(r.up.count("cloud"), sent + 1);
    let unmarked = ask(&r, &r.companion, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(unmarked.status, 200, "a call without the mark is an asked one");
}

#[test]
fn the_share_is_a_named_quarter() {
    assert_eq!(crate::asked::COMPANION_SHARE_PERCENT, 25);
}

#[test]
fn a_call_the_day_cannot_take_moves_to_a_sibling_model_with_room() {
    let (r, _) = plain();
    r.g.holds.day.reserve(9_001, "cloud/gpt-oss-120b", DayEnd::UtcMidnight, LIMIT - 10, false);
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(r.up.count("cloud"), 1);
    assert_eq!(r.up.bodies.lock().unwrap()[0].1["model"], "gpt-oss-20b", "sent to the sibling model, by its own name");
    let row = last(&r);
    assert_eq!((row.account.as_str(), row.model.as_str()), ("cloud", "gpt-oss-20b"));
    assert!(today(&r, "cloud/gpt-oss-20b").requests == 1 && today(&r, "cloud/gpt-oss-120b").requests == 1, "each model's own day");
}

#[test]
fn a_move_obeys_the_checks_a_fall_makes() {
    let (r, _) = plain();
    r.g.holds.day.reserve(9_001, "cloud/gpt-oss-120b", DayEnd::UtcMidnight, LIMIT - 10, false);
    // The sibling's own day has no room either.
    r.g.holds.day.reserve(9_002, "cloud/gpt-oss-20b", DayEnd::UtcMidnight, LIMIT - 10, false);
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    assert_eq!(r.up.count("cloud"), 0);
}

#[test]
fn a_call_nothing_can_take_is_refused_at_once_naming_the_account_the_limit_and_the_reset() {
    let (r, _) = plain();
    for m in ["gpt-oss-120b", "gpt-oss-20b"] {
        r.g.holds.day.reserve(9_100, &format!("cloud/{m}"), DayEnd::UtcMidnight, LIMIT - 3_000, false);
    }
    let big = "x".repeat(16_000);
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({"messages": [{"role": "user", "content": big}]}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    let said: Value = serde_json::from_str(&a.body).unwrap();
    assert_eq!(said["error"]["code"], "day_spent");
    let words = said["error"]["message"].as_str().unwrap();
    assert!(words.starts_with("cloud provider has 3,000 of its 200K tokens left today"), "{words}");
    assert!(words.contains("needs about 5,0") && words.contains("00:00 UTC") && words.contains("in 6 h"), "{words}");
    assert_eq!(r.up.count("cloud"), 0, "the provider was never asked");
    assert_eq!((last(&r).status, last(&r).outcome.as_str()), (429, "day_spent"));
    assert!(!a.body.contains(&big[..50]));
}

#[test]
fn a_spent_request_limit_is_said_in_counts_and_a_reset() {
    let (r, _) = plain();
    for m in ["gpt-oss-120b", "gpt-oss-20b"] {
        for _ in 0..1_000 {
            r.g.holds.day.reserve(9_200, &format!("cloud/{m}"), DayEnd::UtcMidnight, 1, false);
        }
    }
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 429);
    assert!(a.body.contains("has used all 1,000 of today's requests") && a.body.contains("00:00 UTC"), "{}", a.body);
}

#[test]
fn the_answer_length_is_fitted_to_what_the_day_can_still_give_and_never_raised() {
    let (r, _) = plain();
    for m in ["gpt-oss-120b", "gpt-oss-20b"] {
        r.g.holds.day.reserve(9_100, &format!("cloud/{m}"), DayEnd::UtcMidnight, LIMIT - 3_000, false);
    }
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({"max_tokens": 32_768}), false);
    assert_eq!(a.status, 200, "{}", a.body);
    let sent = r.up.bodies.lock().unwrap()[0].1["max_tokens"].as_u64().unwrap();
    assert!(sent < 3_000 && sent > 2_900, "the day's 3,000 less the prompt: {sent}");
    let low = ask(&r, &r.pi, "cloud/gpt-oss-20b", json!({"max_tokens": 100}), false);
    assert_eq!(low.status, 200, "{}", low.body);
    assert_eq!(r.up.bodies.lock().unwrap()[1].1["max_tokens"], 100, "what the harness asked for is a ceiling, never a floor");
}

#[test]
fn the_answer_length_is_fitted_to_the_context_less_the_prompt() {
    let wall = Arc::new(AtomicI64::new(evening()));
    let r = rig(Some(state_path("context")), wall, Some(4_000));
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({"max_completion_tokens": 32_768}), false);
    assert_eq!(a.status, 200, "{}", a.body);
    let sent = r.up.bodies.lock().unwrap()[0].1["max_completion_tokens"].as_u64().unwrap();
    assert!(sent < 4_000 && sent > 3_900, "{sent}");
}

#[test]
fn clamp_answer_only_lowers_and_leaves_an_unnamed_length_alone() {
    let mut b = json!({"max_tokens": 5_000, "max_completion_tokens": 50});
    assert_eq!(clamp_answer(&mut b, 1_000, 0), Some((5_000, 1_000)));
    assert_eq!((b["max_tokens"].clone(), b["max_completion_tokens"].clone()), (json!(1_000), json!(50)));
    let mut none = json!({"messages": []});
    assert_eq!(clamp_answer(&mut none, 10, 0), None);
    assert!(none.get("max_tokens").is_none(), "no length is added");
    let mut zero = json!({"max_tokens": 9});
    clamp_answer(&mut zero, 0, 0);
    assert_eq!(zero["max_tokens"], 1);
}

#[test]
fn a_refused_call_gives_its_estimate_back_and_use_is_reported_in_counts() {
    let (r, _) = plain();
    r.up.set("cloud", &[Out::Status(500)]);
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 500);
    let t = today(&r, "cloud/gpt-oss-120b");
    assert_eq!((t.tokens, t.requests), (0, 1), "a call that was not answered used no tokens");
    r.up.set("cloud", &[Out::Said(200, USED_50K)]);
    ask(&r, &r.companion, "cloud/gpt-oss-120b", json!({}), true);
    let lines = r.g.day_lines("cloud");
    assert_eq!(lines.len(), 2, "one line per model, as the tier scopes the limit");
    let words = lines[0].words("Groq (free)");
    assert_eq!(words, "Groq (free), gpt-oss-120b: 55K of 200K tokens and 1 of 1,000 requests today; the companion has used 55K tokens.".replace("1 of 1,000", "2 of 1,000"), "{words}");
}

/// The day's two models both near the end, so a move is possible only to the one that can take it.
fn nearly_spent(r: &Rig, model: &str) {
    r.g.holds.day.reserve(9_300, &format!("cloud/{model}"), DayEnd::UtcMidnight, LIMIT - 10, false);
}

#[test]
fn two_calls_that_overlap_each_replace_their_own_estimate_in_either_order() {
    let wall = Arc::new(AtomicI64::new(evening()));
    let w: Arc<dyn Fn() -> i64 + Send + Sync> = { let wall = wall.clone(); Arc::new(move || wall.load(Ordering::SeqCst)) };
    for order in [[1, 2], [2, 1]] {
        let b = Budget::at_with(state_path(&format!("overlap{}", order[0])), w.clone());
        b.reserve(1, "k", DayEnd::UtcMidnight, 6_000, false);
        b.reserve(2, "k", DayEnd::UtcMidnight, 7_000, true);
        for rid in order {
            b.settle(rid, Some(if rid == 1 { 2_000 } else { 3_000 }));
        }
        let t = b.today("k", DayEnd::UtcMidnight);
        assert_eq!((t.tokens, t.requests, t.itself_tokens), (5_000, 2, 3_000), "{order:?}: the two real counts and nothing of either estimate");
    }
    // A call that settles twice, or that was never reserved, changes nothing.
    let b = Budget::in_memory();
    b.reserve(5, "k", DayEnd::UtcMidnight, 100, false);
    b.settle(5, Some(40));
    b.settle(5, Some(40));
    b.settle(6, Some(40));
    assert_eq!(b.today("k", DayEnd::UtcMidnight).tokens, 40);
    // A call that was not answered gives its estimate back; one that reported no usage keeps it.
    b.reserve(7, "k", DayEnd::UtcMidnight, 100, false);
    b.settle(7, Some(0));
    b.reserve(8, "k", DayEnd::UtcMidnight, 100, false);
    b.settle(8, None);
    assert_eq!(b.today("k", DayEnd::UtcMidnight).tokens, 140);
}

#[test]
fn the_minutes_ledger_corrects_each_call_by_its_own_key_too() {
    let h = Holds::with_clock(Arc::new(Manual::default()));
    h.spend("k", 1, 6_000);
    h.spend("k", 2, 7_000);
    h.correct("k", 1, 1_000);
    h.correct("k", 2, 2_000);
    // 3,000 are used of 8,000 a minute: 5,000 fit now and 5,001 wait.
    assert_eq!(h.until_room("k", 5_000, 8_000), Duration::ZERO);
    assert!(h.until_room("k", 5_001, 8_000) > Duration::ZERO);
}

#[test]
fn what_each_documented_wording_says_the_account_has_counted_is_read_as_numbers() {
    use crate::budget::{day_used, DayUsed};
    let groq_tpd = "Rate limit reached for model `openai/gpt-oss-120b` in organization `org_01` service tier `on_demand` on tokens per day (TPD): Limit 200000, Used 196699, Requested 4193. Please try again in 14m30.8s.";
    assert_eq!(day_used(groq_tpd), DayUsed { tokens: Some(196_699), requests: None });
    let groq_rpd = "Rate limit reached for model `x` in organization `org_01` service tier `on_demand` on requests per day (RPD): Limit 1000, Used 1000, Requested 1. Please try again in 1m26.4s.";
    assert_eq!(day_used(groq_rpd), DayUsed { tokens: None, requests: Some(1_000) });
    let openai = "Rate limit reached for gpt-4o in organization org-abc on tokens per day (TPD): Limit 1,500,000, Used 1,499,000, Requested 5,000. Please try again in 1h2m.";
    assert_eq!(day_used(openai), DayUsed { tokens: Some(1_499_000), requests: None });
    let words_only = "You exceeded your current quota, please check your plan. tokens per day";
    assert_eq!(day_used(words_only), DayUsed::default(), "a wording without a count says nothing");
    let minute = "Rate limit reached ... on tokens per minute (TPM): Limit 8000, Used 7000, Requested 4193. Please try again in 6s.";
    assert_eq!(day_used(minute), DayUsed::default(), "a minute is not a day");
}

#[test]
fn a_429_that_states_the_days_count_becomes_the_ledgers_count_and_its_words_go_no_further_than_numbers() {
    let (r, path) = plain();
    r.up.set(
        "cloud",
        &[Out::Said(429, "Rate limit reached for model `m` on tokens per day (TPD): Limit 200000, Used 199500, Requested 4193. Please try again in 14m30.8s. the-secret-words-of-a-page")],
    );
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    assert_eq!(today(&r, "cloud/gpt-oss-120b").tokens, 199_500, "the account's own count, with nothing of the refused call added or taken off");
    let t = today(&r, "cloud/gpt-oss-120b");
    assert_eq!(t.requests, 1);
    // Set from the account's count, not added to ours: the next call sees the day nearly spent.
    r.up.set("cloud", &[Out::Ok]);
    let next = ask(&r, &r.pi, "cloud/gpt-oss-20b", json!({}), false);
    assert_eq!(next.status, 200, "the other model has its own day: {}", next.body);
    let again = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(again.status, 200, "moved to the sibling, which has room: {}", again.body);
    assert_eq!(last(&r).moved_from.as_deref(), Some("cloud/gpt-oss-120b"));
    let state = std::fs::read_to_string(path).unwrap();
    assert!(!state.contains(SECRET) && !state.contains("Rate limit"), "{state}");
}

#[test]
fn the_count_an_account_stated_is_kept_past_midnight_while_it_says_it_still_counts_it() {
    let path = state_path("held");
    let wall = Arc::new(AtomicI64::new(evening()));
    let w: Arc<dyn Fn() -> i64 + Send + Sync> = { let wall = wall.clone(); Arc::new(move || wall.load(Ordering::SeqCst)) };
    let b = Budget::at_with(path, w);
    // 18:00 UTC; the account says its count frees up in 9 hours, which is after our midnight.
    b.learn(0, "k", DayEnd::UtcMidnight, crate::budget::DayUsed { tokens: Some(199_000), requests: None }, Some(Duration::from_secs(9 * 3600)));
    wall.fetch_add(7 * 3600, Ordering::SeqCst);
    let t = b.today("k", DayEnd::UtcMidnight);
    assert_eq!(t.tokens, 199_000, "past our midnight the account still counts it");
    assert_eq!(t.resets_in.as_secs(), 2 * 3600, "and says when it frees");
    wall.fetch_add(3 * 3600, Ordering::SeqCst);
    assert_eq!(b.today("k", DayEnd::UtcMidnight).tokens, 0, "then the day is new");
}

#[test]
fn a_call_is_never_sent_with_an_answer_of_almost_nothing() {
    let (r, _) = plain();
    // 300 tokens left today on both models: the prompt fits, a useful answer does not.
    for m in ["gpt-oss-120b", "gpt-oss-20b"] {
        r.g.holds.day.reserve(9_400, &format!("cloud/{m}"), DayEnd::UtcMidnight, LIMIT - 300, false);
    }
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({"max_tokens": 4_000}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    assert!(a.body.contains("day_spent") && a.body.contains("300 of its 200K tokens left"), "{}", a.body);
    assert_eq!(r.up.count("cloud"), 0);
    // A harness that asked for less than the minimum is not made to take more.
    let small = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({"max_tokens": 100}), false);
    assert_eq!(small.status, 200, "{}", small.body);
    assert_eq!(r.up.bodies.lock().unwrap()[0].1["max_tokens"], 100);
    // Where it is sent, the clamp stops at the minimum, however little the day or context leaves.
    let mut b = json!({"max_tokens": 32_768});
    assert_eq!(clamp_answer(&mut b, 50, crate::day::MIN_ANSWER), Some((32_768, 1_024)));
    let mut low = json!({"max_tokens": 500});
    assert_eq!(clamp_answer(&mut low, 50, crate::day::MIN_ANSWER), None, "never raised to the floor");
}

#[test]
fn a_move_says_which_model_answered_which_was_asked_for_and_the_row_keeps_both_names() {
    let (r, _) = plain();
    nearly_spent(&r, "gpt-oss-120b");
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(a.header("x-yantrik-answered-by").as_deref(), Some("cloud/gpt-oss-20b"));
    assert_eq!(a.header("x-yantrik-fell-from").as_deref(), Some("cloud/gpt-oss-120b"));
    let row = last(&r);
    assert_eq!(row.moved_from.as_deref(), Some("cloud/gpt-oss-120b"));
    assert_eq!(row.model, "gpt-oss-20b");
    let line = serde_json::to_string(&row).unwrap();
    assert!(line.contains("\"moved_from\":\"cloud/gpt-oss-120b\"") && !line.contains(SECRET), "{line}");
    // A call that was not moved says nothing of it.
    let plain_call = ask(&r, &r.pi, "cloud/gpt-oss-20b", json!({}), false);
    assert!(plain_call.header("x-yantrik-fell-from").is_none() && last(&r).moved_from.is_none());
}

fn no_tools(c: &mut yantrik_ml::model_caps::ModelCaps) {
    c.tools = false;
}
fn no_sight(c: &mut yantrik_ml::model_caps::ModelCaps) {
    c.vision = false;
}
fn small_window(c: &mut yantrik_ml::model_caps::ModelCaps) {
    c.context = 2_000;
}

#[test]
fn a_move_skips_a_model_that_cannot_do_what_the_call_needs() {
    let tool = json!({"type": "function", "function": {"name": "read", "parameters": {"type": "object"}}});
    let image = json!([{"type": "text", "text": "what is this"}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}]);
    let big = "x".repeat(16_000);
    // Each case: what the sibling cannot do, and the call that needs it.
    let cases: [(fn(&mut yantrik_ml::model_caps::ModelCaps), Value); 3] = [
        (no_tools, json!({"tools": [tool]})),
        (no_sight, json!({"messages": [{"role": "user", "content": image}]})),
        (small_window, json!({"messages": [{"role": "user", "content": big}]})),
    ];
    for (i, (twin, body)) in cases.into_iter().enumerate() {
        let (r, _) = plain_with(twin);
        r.g.holds.day.reserve(9_500, "cloud/gpt-oss-120b", DayEnd::UtcMidnight, LIMIT - 10, false);
        let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", body, false);
        assert_eq!(a.status, 429, "case {i} was moved to a model that cannot take it: {}", a.body);
        assert_eq!(r.up.count("cloud"), 0, "case {i}");
    }
    // The same calls go to a sibling that can.
    let (r, _) = plain();
    nearly_spent(&r, "gpt-oss-120b");
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({"tools": [tool]}), false);
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(last(&r).moved_from.as_deref(), Some("cloud/gpt-oss-120b"));
}
