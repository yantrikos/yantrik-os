use super::headers::{duration, observe, Observed};
use super::quota::{next_day_start, pacific_offset, Outcome};
use super::tiers::Reset;
use super::*;

/// 2026-09-30 12:00:00 UTC.
const NOON: i64 = 1_790_769_600;

fn pool(enabled: &[&str], keyed: &[&str]) -> Pool {
    let ledger = ledger::Ledger::open(&std::env::temp_dir().join(format!("pool-ledger-{}-{}.json", std::process::id(), rand::random::<u32>())));
    Pool::new(
        Settings { enabled: enabled.iter().map(|s| s.to_string()).collect(), keyed: keyed.iter().map(|s| s.to_string()).collect(), openrouter_paid_credit: false },
        ledger,
    )
}

fn ok() -> Outcome {
    Outcome { status: 200, observed: Observed::default(), tokens: 10 }
}

fn refused(retry_after: Option<f64>) -> Outcome {
    Outcome { status: 429, observed: Observed { retry_after, ..Observed::default() }, tokens: 0 }
}

#[test]
fn only_switched_on_providers_with_their_key_are_used() {
    let mut p = pool(&["groq"], &[]);
    assert!(p.pick(&Need::default(), NOON).is_err(), "Groq needs a key");
    let status = p.status(NOON);
    assert_eq!(status.iter().find(|s| s.id == "groq").unwrap().state, "Needs a key");
    assert_eq!(status.iter().find(|s| s.id == "openrouter").unwrap().state, "Off");
    let mut p = pool(&["groq", "ovh"], &["groq"]);
    for _ in 0..10 {
        let pick = p.pick(&Need::default(), NOON).unwrap();
        assert!(["groq", "ovh"].contains(&pick.tier.id));
    }
    // OVH needs no key.
    let mut p = pool(&["ovh"], &[]);
    assert_eq!(p.pick(&Need::default(), NOON).unwrap().tier.id, "ovh");
}

#[test]
fn a_private_turn_never_goes_where_prompts_may_be_trained_on() {
    let mut p = pool(&["gemini", "kilo", "groq"], &["gemini", "groq"]);
    // Not shareable is the default: a turn is private unless its caller says otherwise.
    let private = Need::default();
    for _ in 0..20 {
        let pick = p.pick(&private, NOON).unwrap();
        assert!(!pick.tier.trains_on_prompts, "{} trains on prompts", pick.tier.id);
    }
    // Z.ai's terms let it train on what individuals send (rechecked 2026-09-30): never private.
    let mut zai = pool(&["zai"], &["zai"]);
    assert!(zai.pick(&private, NOON).is_err(), "a private turn went to Z.ai");
    assert_eq!(zai.pick(&Need { shareable: true, ..Need::default() }, NOON).unwrap().tier.id, "zai");
    let mut only_trainers = pool(&["gemini", "kilo"], &["gemini"]);
    let none = only_trainers.pick(&private, NOON).unwrap_err();
    assert_eq!(none.reason, "No switched-on free provider has a model that can do this.");
}

#[test]
fn a_public_answer_only_goes_where_the_terms_allow_it() {
    let mut p = pool(&["groq", "ovh", "kilo"], &["groq"]);
    let public = Need { public: true, ..Need::default() };
    for _ in 0..10 {
        assert_eq!(p.pick(&public, NOON).unwrap().tier.id, "groq", "only Groq's terms allow end users");
    }
}

#[test]
fn a_model_must_be_able_to_do_the_job() {
    let mut p = pool(&["groq"], &["groq"]);
    let code = Need { coding: true, tools: true, ..Need::default() };
    for _ in 0..10 {
        assert_ne!(p.pick(&code, NOON).unwrap().model.id, "openai/gpt-oss-20b", "a light coder is not used for code");
    }
    // Gemini trains on what it is sent: only a shareable turn may go there.
    let huge = Need { min_context: 500_000, shareable: true, ..Need::default() };
    assert!(p.pick(&huge, NOON).is_err(), "no Groq model holds 500K tokens");
    let mut p = pool(&["gemini"], &["gemini"]);
    assert_eq!(p.pick(&huge, NOON).unwrap().model.id, "gemini-3.8-flash");
}

#[test]
fn a_full_daily_window_moves_on_and_says_when_it_frees() {
    let mut p = pool(&["groq"], &["groq"]);
    let need = Need::default();
    let tier = tiers::tier("groq").unwrap();
    let big = &tier.models[0];
    // Spend gpt-oss-120b's 1,000 requests, a few seconds apart so the per-minute limit holds.
    for i in 0..1_000 {
        p.record(Pick { tier, model: big }, &need, NOON + i * 3, &ok());
    }
    let later = NOON + 3_100;
    for _ in 0..10 {
        assert_ne!(p.pick(&need, later).unwrap().model.id, big.id, "its day is spent");
    }
    // Spend the other two as well: nothing is left until midnight UTC, and the pool says so.
    for m in &tier.models[1..] {
        for i in 0..1_000 {
            p.record(Pick { tier, model: m }, &need, NOON + i * 3, &ok());
        }
    }
    let none = p.pick(&need, later).unwrap_err();
    assert_eq!(none.until, Some(next_day_start(Reset::UtcMidnight, later)));
    assert!(p.status(later).iter().find(|s| s.id == "groq").unwrap().state.starts_with("Resting for"));
    assert!(p.pick(&need, next_day_start(Reset::UtcMidnight, later) + 1).is_ok(), "a new day, a new allowance");
}

#[test]
fn a_refusal_rests_for_as_long_as_it_asked_and_no_longer() {
    let mut p = pool(&["ovh"], &[]);
    let need = Need::default();
    let pick = p.pick(&need, NOON).unwrap();
    p.record(pick, &need, NOON, &refused(Some(30.0)));
    assert!(p.pick(&need, NOON + 29).is_err());
    assert!(p.pick(&need, NOON + 31).is_ok());
}

#[test]
fn a_refusal_that_does_not_say_for_how_long_backs_off_doubling() {
    let mut p = pool(&["zai"], &["zai"]);
    let need = Need { coding: true, shareable: true, ..Need::default() };
    let pick = p.pick(&need, NOON).unwrap();
    p.record(pick, &need, NOON, &refused(None));
    assert!(p.pick(&need, NOON + 59).is_err());
    let again = p.pick(&need, NOON + 61).unwrap();
    p.record(again, &need, NOON + 61, &refused(None));
    assert!(p.pick(&need, NOON + 61 + 119).is_err(), "the second refusal rests twice as long");
    assert!(p.pick(&need, NOON + 61 + 121).is_ok());
}

#[test]
fn what_the_provider_says_is_left_overrides_the_table() {
    let mut p = pool(&["groq"], &["groq"]);
    let need = Need::default();
    let tier = tiers::tier("groq").unwrap();
    for m in tier.models {
        let told = Outcome {
            status: 200,
            observed: Observed { remaining_requests: Some(0), reset_requests: Some(7.0), ..Observed::default() },
            tokens: 5,
        };
        p.record(Pick { tier, model: m }, &need, NOON, &told);
    }
    assert!(p.pick(&need, NOON + 5).is_err(), "Groq said none are left for 7 s");
    assert!(p.pick(&need, NOON + 8).is_ok());
}

#[test]
fn a_refused_key_stops_that_provider_until_it_is_set_up_again() {
    let mut p = pool(&["groq"], &["groq"]);
    let need = Need::default();
    let pick = p.pick(&need, NOON).unwrap();
    p.record(pick, &need, NOON, &Outcome { status: 401, ..Outcome::default() });
    assert!(p.pick(&need, NOON + 86_400 * 3).is_err(), "a refused key is not retried on a timer");
    assert_eq!(p.status(NOON).iter().find(|s| s.id == "groq").unwrap().state, "Key refused: set it up again");
    p.key_changed("groq");
    assert!(p.pick(&need, NOON).is_ok());
}

#[test]
fn three_failures_in_a_row_open_the_breaker() {
    // OVH allows 2 a minute: the failures are 31 s apart, so only the breaker can stop the fourth.
    let mut p = pool(&["ovh"], &[]);
    let need = Need::default();
    for i in 0..3 {
        let pick = p.pick(&need, NOON + i * 31).unwrap();
        p.record(pick, &need, NOON + i * 31, &Outcome { status: 502, ..Outcome::default() });
    }
    let third = NOON + 62;
    assert!(p.pick(&need, third + 100).is_err(), "the breaker is open");
    assert!(p.pick(&need, third + 301).is_ok(), "and closes after five minutes");
}

#[test]
fn a_task_keeps_its_model_until_that_model_refuses() {
    let mut p = pool(&["groq", "ovh"], &["groq"]);
    let task = Need { sticky: Some("build-the-game".into()), ..Need::default() };
    let first = p.pick(&task, NOON).unwrap();
    p.record(first, &task, NOON, &ok());
    for i in 1..20 {
        let next = p.pick(&task, NOON + i * 5).unwrap();
        assert_eq!((next.tier.id, next.model.id), (first.tier.id, first.model.id), "step {i} moved");
        p.record(next, &task, NOON + i * 5, &ok());
    }
    p.record(first, &task, NOON + 200, &refused(Some(600.0)));
    let moved = p.pick(&task, NOON + 201).unwrap();
    assert_ne!((moved.tier.id, moved.model.id), (first.tier.id, first.model.id));
}

#[test]
fn equal_providers_share_the_load_in_turn() {
    let mut p = pool(&["groq"], &["groq"]);
    let need = Need::default();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..12 {
        seen.insert(p.pick(&need, NOON).unwrap().model.id);
    }
    assert!(seen.len() >= 2, "only {seen:?} was ever used");
}

#[test]
fn openrouter_allows_more_after_credit_was_bought_once() {
    let mut p = pool(&["openrouter"], &["openrouter"]);
    assert_eq!(p.status(NOON).iter().find(|s| s.id == "openrouter").unwrap().daily_cap, Some(50));
    p.set(Settings { enabled: vec!["openrouter".into()], keyed: vec!["openrouter".into()], openrouter_paid_credit: true });
    assert_eq!(p.status(NOON).iter().find(|s| s.id == "openrouter").unwrap().daily_cap, Some(1_000));
}

#[test]
fn rate_limit_headers_are_read_in_every_shape_providers_use() {
    let groq = |name: &str| match name {
        "x-ratelimit-remaining-requests" => Some("14".to_string()),
        "x-ratelimit-reset-requests" => Some("2m59.56s".to_string()),
        "x-ratelimit-remaining-tokens" => Some("7900".to_string()),
        "x-ratelimit-reset-tokens" => Some("120ms".to_string()),
        _ => None,
    };
    let o = observe(groq);
    assert_eq!(o.remaining_requests, Some(14));
    assert!((o.reset_requests.unwrap() - 179.56).abs() < 1e-9);
    assert_eq!(o.remaining_tokens, Some(7_900));
    assert!((o.reset_tokens.unwrap() - 0.12).abs() < 1e-9);
    let ovh = |name: &str| match name {
        "ratelimit-remaining" => Some("0".to_string()),
        "ratelimit-reset" => Some("23".to_string()),
        "retry-after" => Some("23".to_string()),
        _ => None,
    };
    let o = observe(ovh);
    assert_eq!((o.remaining_requests, o.reset_requests, o.retry_after), (Some(0), Some(23.0), Some(23.0)));
    assert_eq!(observe(|_| None), Observed::default());
    for bad in ["", "soon", "5x", "Wed, 21 Oct 2026 07:28:00 GMT"] {
        assert_eq!(duration(bad), None, "{bad}");
    }
    assert_eq!(duration("1h2m3s"), Some(3_723.0));
}

#[test]
fn gemini_resets_at_midnight_in_los_angeles_daylight_saving_included() {
    // 2026: PDT from 8 March 10:00 UTC to 1 November 09:00 UTC.
    assert_eq!(pacific_offset(1_772_964_000 - 1), -8 * 3_600, "just before DST starts");
    assert_eq!(pacific_offset(1_772_964_000), -7 * 3_600, "DST starts 2026-03-08 10:00 UTC");
    assert_eq!(pacific_offset(1_793_523_600 - 1), -7 * 3_600, "just before DST ends");
    assert_eq!(pacific_offset(1_793_523_600), -8 * 3_600, "DST ends 2026-11-01 09:00 UTC");
    // 2026-09-30 12:00 UTC is 05:00 PDT; that day ends at 2026-10-01 07:00 UTC.
    assert_eq!(next_day_start(Reset::PacificMidnight, NOON), 1_790_838_000);
    assert_eq!(next_day_start(Reset::UtcMidnight, NOON), 1_790_812_800);
}

#[test]
fn the_ledger_counts_the_day_and_survives_a_restart() {
    let path = std::env::temp_dir().join(format!("pool-ledger-restart-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut l = ledger::Ledger::open(&path);
    l.add("2026-09-30", "groq", "a", true, 100);
    l.add("2026-09-30", "groq", "b", false, 0);
    l.add("2026-09-30", "ovh", "c", true, 7);
    l.save().unwrap();
    let back = ledger::Ledger::open(&path);
    let day = back.day("2026-09-30");
    assert_eq!(day["groq"], ledger::Totals { requests: 2, tokens: 100, refused: 1 });
    assert_eq!(day["ovh"].requests, 1);
    assert_eq!(ledger::day_name(0), "1970-01-01");
    assert_eq!(ledger::day_name(NOON), "2026-09-30");
    let _ = std::fs::remove_file(&path);
}

/// A provider's header is believed only within reason (security review of #535, 1 Oct 2026):
/// `inf` and `1e308` overflowed the clock into no wait at all, which is a hot loop against a
/// provider asking us to wait; a billion seconds rested a model for thirty years.
#[test]
fn a_hostile_wait_header_is_bounded_and_never_overflows() {
    for hostile in ["inf", "1e308", "NaN", "-5", "999999999", "99999999h"] {
        if let Some(s) = duration(hostile) {
            assert!(s.is_finite() && (0.0..=headers::MAX_WAIT_SECS).contains(&s), "{hostile} gave {s}");
        }
    }
    let mut p = pool(&["groq"], &["groq"]);
    let pick = p.pick(&Need::default(), NOON).unwrap();
    let rested = pick.model.id;
    // The worst a header can carry, straight into the quota: no panic, and a wait of at most a day.
    p.record(pick, &Need::default(), NOON, &refused(Some(f64::MAX)));
    for _ in 0..10 {
        assert_ne!(p.pick(&Need::default(), NOON + 60).unwrap().model.id, rested, "it was told to wait, and must");
    }
    let back = (0..20).any(|_| p.pick(&Need::default(), NOON + 86_400 + 60).is_ok_and(|q| q.model.id == rested));
    assert!(back, "no header rests a model for more than a day");
}

/// A caller that does not say a turn is shareable keeps it away from every provider that may
/// train on it: privacy fails closed.
#[test]
fn a_turn_is_private_unless_its_caller_says_otherwise() {
    let mut trainers = pool(&["gemini", "kilo", "zai"], &["gemini", "zai"]);
    assert!(trainers.pick(&Need::default(), NOON).is_err());
    let shareable = Need { shareable: true, ..Need::default() };
    assert!(trainers.pick(&shareable, NOON).is_ok());
}
