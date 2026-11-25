//! The day's budget (#1400): the ledger across a restart, the companion's share, a call the day
//! cannot take moved to a sibling model or refused at once, the answer length fitted, and that
//! nothing of any call's content is in a log row or the state file. Time is a clock a test moves.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

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
}

impl Accounts for Days {
    fn models(&self) -> Vec<ModelInfo> {
        let mut all = self.inner.models();
        let mut twin = all[0].clone();
        twin.id = "cloud/gpt-oss-20b".into();
        twin.name = "gpt-oss-20b".into();
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
        accounts: Arc::new(Days { inner: switch, context }),
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
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = state_path(&format!("plain{}", N.fetch_add(1, Ordering::SeqCst)));
    (rig(Some(path.clone()), Arc::new(AtomicI64::new(evening())), None), path)
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
    first.reserve("cloud/gpt-oss-120b", DayEnd::UtcMidnight, 4_000, true);
    first.correct("cloud/gpt-oss-120b", DayEnd::UtcMidnight, 5_500);
    first.reserve("cloud/gpt-oss-120b", DayEnd::UtcMidnight, 1_000, false);
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
    r.g.holds.day.reserve("cloud/gpt-oss-120b", DayEnd::UtcMidnight, LIMIT - 10, false);
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
    r.g.holds.day.reserve("cloud/gpt-oss-120b", DayEnd::UtcMidnight, LIMIT - 10, false);
    // The sibling's own day has no room either.
    r.g.holds.day.reserve("cloud/gpt-oss-20b", DayEnd::UtcMidnight, LIMIT - 10, false);
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    assert_eq!(r.up.count("cloud"), 0);
}

#[test]
fn a_call_nothing_can_take_is_refused_at_once_naming_the_account_the_limit_and_the_reset() {
    let (r, _) = plain();
    for m in ["gpt-oss-120b", "gpt-oss-20b"] {
        r.g.holds.day.reserve(&format!("cloud/{m}"), DayEnd::UtcMidnight, LIMIT - 3_000, false);
    }
    let big = "x".repeat(16_000);
    let a = ask(&r, &r.pi, "cloud/gpt-oss-120b", json!({"messages": [{"role": "user", "content": big}]}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    let said: Value = serde_json::from_str(&a.body).unwrap();
    assert_eq!(said["error"]["code"], "day_spent");
    let words = said["error"]["message"].as_str().unwrap();
    assert!(words.starts_with("cloud provider has 3,000 of its 200K tokens left today"), "{words}");
    assert!(words.contains("needs about 4,0") && words.contains("00:00 UTC") && words.contains("in 6 h"), "{words}");
    assert_eq!(r.up.count("cloud"), 0, "the provider was never asked");
    assert_eq!((last(&r).status, last(&r).outcome.as_str()), (429, "day_spent"));
    assert!(!a.body.contains(&big[..50]));
}

#[test]
fn a_spent_request_limit_is_said_in_counts_and_a_reset() {
    let (r, _) = plain();
    for m in ["gpt-oss-120b", "gpt-oss-20b"] {
        for _ in 0..1_000 {
            r.g.holds.day.reserve(&format!("cloud/{m}"), DayEnd::UtcMidnight, 1, false);
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
        r.g.holds.day.reserve(&format!("cloud/{m}"), DayEnd::UtcMidnight, LIMIT - 3_000, false);
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
    assert_eq!(clamp_answer(&mut b, 1_000), Some((5_000, 1_000)));
    assert_eq!((b["max_tokens"].clone(), b["max_completion_tokens"].clone()), (json!(1_000), json!(50)));
    let mut none = json!({"messages": []});
    assert_eq!(clamp_answer(&mut none, 10), None);
    assert!(none.get("max_tokens").is_none(), "no length is added");
    let mut zero = json!({"max_tokens": 9});
    clamp_answer(&mut zero, 0);
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
