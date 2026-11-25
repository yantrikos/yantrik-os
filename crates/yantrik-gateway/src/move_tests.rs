//! A call one account cannot take goes to another of the person's accounts (#1401): Groq spent,
//! Kilo with room; and the cases that must never move.

use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::budget::{Budget, Daily, DayEnd};
use crate::fall_tests::{raw, Hosts, Manual, Out, Raw, Rig, Switch};
use crate::fit::Room;
use crate::health::Health;
use crate::hold::Holds;
use crate::log::{Call, CallLog};
use crate::policy::{Refusal, Sharing, Target};
use crate::server::{Accounts, Gateway, ModelInfo};
use crate::tests::{request, Fake};
use crate::tokens::{mint, Grant, Tokens};

const GROQ: &str = "cloud/gpt-oss-120b";
const KILO: &str = "elsewhere/kilo-m";
const SPENT: &str = "Rate limit reached for model `m` on tokens per day (TPD): Limit 200000, Used 199500, Requested 4193. Please try again in 14m30.8s.";

/// Groq (`cloud`) and Kilo (`elsewhere`), each with a daily limit, and the home lane.
struct Pair {
    inner: Switch,
    kilo_sharing: Sharing,
    kilo_tools: bool,
    with_kilo: bool,
}

impl Accounts for Pair {
    fn models(&self) -> Vec<ModelInfo> {
        let mut all = self.inner.models();
        if self.with_kilo {
            let mut kilo = all[0].clone();
            (kilo.id, kilo.account, kilo.name) = (KILO.into(), "elsewhere".into(), "kilo-m".into());
            kilo.caps.tools = self.kilo_tools;
            all.insert(1, kilo);
        }
        all
    }
    fn target(&self, account: &str, model: &str) -> Result<Target, Refusal> {
        let mut t = self.inner.target(account, model)?;
        match account {
            "elsewhere" => {
                t.sharing = self.kilo_sharing;
                t.provider = "Kilo (free)".into();
            }
            "cloud" => t.provider = "Groq (free)".into(),
            _ => {}
        }
        Ok(t)
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
    fn room(&self, _account: &str, _model: &str) -> Room {
        Room::default()
    }
    fn daily(&self, account: &str, _model: &str) -> Daily {
        match account {
            "cloud" => Daily { tokens: Some(200_000), requests: Some(1_000), per_account: true, end: DayEnd::UtcMidnight },
            "elsewhere" => Daily { tokens: Some(150_000), requests: Some(1_000), per_account: true, end: DayEnd::UtcMidnight },
            _ => Daily::default(),
        }
    }
}

fn rig_with(kilo_sharing: Sharing, kilo_tools: bool, with_kilo: bool) -> Rig {
    let tokens = Arc::new(Tokens::in_memory());
    let (pi, mind, companion) = (mint().unwrap(), mint().unwrap(), mint().unwrap());
    tokens.register(&pi, Grant { harness: "pi".into(), private_context: false }).unwrap();
    tokens.register(&mind, Grant { harness: "mind".into(), private_context: true }).unwrap();
    tokens.register(&companion, Grant { harness: crate::asked::COMPANION.into(), private_context: true }).unwrap();
    let up = Arc::new(Hosts::default());
    let clock = Arc::new(Manual::default());
    let switch = Switch {
        inner: Fake { private_mode: false, cloud: Sharing::Share, lane: None },
        fall: Arc::new(AtomicBool::new(false)),
        cloud_off_from: usize::MAX,
        cloud_asks: AtomicUsize::new(0),
        cloud_tight: false,
        lane_tight: false,
    };
    let g = Gateway {
        tokens,
        accounts: Arc::new(Pair { inner: switch, kilo_sharing, kilo_tools, with_kilo }),
        upstream: up.clone(),
        log: Arc::new(CallLog::in_memory()),
        template_refused: Default::default(),
        params_refused: Default::default(),
        health: Arc::new(Health::with_clock(clock.clone())),
        holds: Arc::new(Holds::with_budget(clock.clone(), Budget::in_memory())),
    };
    Rig { g, up, fall: Arc::new(AtomicBool::new(false)), clock, pi, mind, companion }
}

fn rig() -> Rig {
    rig_with(Sharing::Share, true, true)
}

fn ask(r: &Rig, token: &str, extra: Value, mark: bool) -> Raw {
    let mut body = json!({"model": GROQ, "messages": [{"role": "user", "content": "hi"}]});
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

/// `key`'s day used up to within 10 tokens of its limit.
fn spend(r: &Rig, rid: u64, key: &str, limit: u64) {
    r.g.holds.day.reserve(rid, key, DayEnd::UtcMidnight, limit - 10, false);
}

fn said_moved(a: &Raw, r: &Rig) {
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(a.header("x-yantrik-answered-by").as_deref(), Some(KILO), "{}", a.head);
    assert_eq!(a.header("x-yantrik-fell-from").as_deref(), Some(GROQ));
    let row = last(r);
    assert_eq!((row.account.as_str(), row.moved_from.as_deref()), ("elsewhere", Some(GROQ)));
    assert!(row.ok() && row.outcome.is_empty(), "{row:?}");
    let line = serde_json::to_string(&row).unwrap();
    assert!(!line.contains("\"hi\"") && !line.contains("Rate limit"), "counts and names only: {line}");
}

#[test]
fn groq_spent_for_the_day_and_kilo_with_room_is_answered_by_kilo_before_anything_is_sent_to_groq() {
    let r = rig();
    spend(&r, 1, "cloud", 200_000);
    let a = ask(&r, &r.pi, json!({}), false);
    said_moved(&a, &r);
    assert_eq!(r.up.count("cloud"), 0);
    assert_eq!(r.up.count("elsewhere"), 1);
    assert_eq!(r.up.bodies.lock().unwrap()[0].1["model"], "kilo-m");
    assert_eq!(last(&r).moved_why.as_deref(), Some("day"));
}

#[test]
fn an_account_that_says_the_day_is_spent_moves_the_call_before_the_first_byte() {
    let r = rig();
    r.up.set("cloud", &[Out::Said(429, SPENT)]);
    let a = ask(&r, &r.pi, json!({}), false);
    said_moved(&a, &r);
    assert_eq!((r.up.count("cloud"), r.up.count("elsewhere")), (1, 1));
    assert_eq!(last(&r).moved_why.as_deref(), Some("day"));
    assert!(!r.g.health.is_down("cloud"), "an account that is spent is not down");
}

#[test]
fn an_account_that_is_down_moves_the_call_and_a_second_failure_is_not_tried_again() {
    let r = rig();
    r.up.set("cloud", &[Out::Down]);
    let a = ask(&r, &r.pi, json!({}), false);
    said_moved(&a, &r);
    assert_eq!(last(&r).moved_why.as_deref(), Some("down"));
    assert!(r.g.health.is_down("cloud"));
    // When the second account fails too, the call ends: one move per call.
    let r = rig();
    r.up.set("cloud", &[Out::Status(503)]);
    r.up.set("elsewhere", &[Out::Down]);
    let a = ask(&r, &r.pi, json!({}), false);
    assert_eq!(a.status, 502, "{}", a.body);
    assert_eq!((r.up.count("cloud"), r.up.count("elsewhere")), (1, 1));
}

#[test]
fn a_call_too_large_for_the_account_moves_when_a_shrink_cannot_fit_it() {
    let r = rig();
    r.up.set("cloud", &[Out::Status(413)]);
    let a = ask(&r, &r.pi, json!({}), false);
    said_moved(&a, &r);
    assert_eq!(last(&r).moved_why.as_deref(), Some("size"));
}

#[test]
fn nothing_moves_where_the_person_did_not_agree_the_data_may_go() {
    // A private-marked conversation: Kilo set to "keep personal things on this computer", or Don't share.
    for sharing in [Sharing::Lane, Sharing::Off] {
        let r = rig_with(sharing, true, true);
        spend(&r, 1, "cloud", 200_000);
        let a = ask(&r, &r.mind, json!({}), false);
        assert_eq!(a.status, 429, "{sharing:?}: {}", a.body);
        assert_eq!(r.up.count("elsewhere"), 0, "{sharing:?}");
        let r = rig_with(sharing, true, true);
        r.up.set("cloud", &[Out::Down]);
        assert_eq!(ask(&r, &r.mind, json!({}), false).status, 502);
        assert_eq!(r.up.count("elsewhere"), 0, "{sharing:?}");
    }
    // The same account takes a call that carries no private context.
    let r = rig_with(Sharing::Off, true, true);
    spend(&r, 1, "cloud", 200_000);
    assert_eq!(ask(&r, &r.pi, json!({}), false).status, 200);
    assert_eq!(r.up.count("elsewhere"), 1);
    // And a private-marked conversation goes to a sharing account.
    let r = rig();
    spend(&r, 1, "cloud", 200_000);
    assert_eq!(ask(&r, &r.mind, json!({}), false).status, 200);
}

#[test]
fn a_tool_call_never_moves_to_a_model_without_tool_calling() {
    let tool = json!({"type": "function", "function": {"name": "read", "parameters": {"type": "object"}}});
    let r = rig_with(Sharing::Share, false, true);
    spend(&r, 1, "cloud", 200_000);
    let a = ask(&r, &r.pi, json!({"tools": [tool]}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    let r = rig_with(Sharing::Share, false, true);
    r.up.set("cloud", &[Out::Down]);
    assert_eq!(ask(&r, &r.pi, json!({"tools": [tool]}), false).status, 502);
    assert_eq!(r.up.count("elsewhere"), 0);
    // Without tools the same model takes the call.
    let r = rig_with(Sharing::Share, false, true);
    spend(&r, 1, "cloud", 200_000);
    assert_eq!(ask(&r, &r.pi, json!({}), false).status, 200);
}

#[test]
fn a_call_the_companion_started_by_itself_never_moves() {
    let r = rig();
    spend(&r, 1, "cloud", 200_000);
    assert_eq!(ask(&r, &r.companion, json!({}), true).status, 429);
    let r = rig();
    r.up.set("cloud", &[Out::Said(429, SPENT)]);
    assert_eq!(ask(&r, &r.companion, json!({}), true).status, 429);
    let r = rig();
    r.up.set("cloud", &[Out::Down]);
    assert_eq!(ask(&r, &r.companion, json!({}), true).status, 502);
    assert_eq!(r.up.count("elsewhere"), 0);
}

#[test]
fn with_no_other_account_and_no_lane_the_words_are_todays_and_name_when_the_day_frees() {
    let r = rig_with(Sharing::Share, true, false);
    spend(&r, 1, "cloud", 200_000);
    let a = ask(&r, &r.pi, json!({}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    assert!(a.body.contains("day_spent") && a.body.contains("frees up at 00:00 UTC"), "{}", a.body);
    // Both accounts spent: the same, naming the account the person asked for.
    let r = rig();
    spend(&r, 1, "cloud", 200_000);
    spend(&r, 2, "elsewhere", 150_000);
    let a = ask(&r, &r.pi, json!({}), false);
    assert_eq!(a.status, 429, "{}", a.body);
    assert!(a.body.contains("Groq (free)") && a.body.contains("frees up"), "{}", a.body);
    assert_eq!(r.up.count("elsewhere"), 0);
}
