use super::*;

/// Every tier, and whether it is tight. A new tier is not in this table, and the last test
/// fails until someone has answered the question for it.
const TABLE: &[(&str, bool)] = &[
    ("groq", false),       // 30 a minute, 1,000 a day
    ("openrouter", true),  // 50 a day (1,000 only after $10 of credit, which the table does not know)
    ("gemini", false),     // limits not published
    ("cloudflare", false), // 300 a minute; tokens a day
    ("zai", false),        // limits not published
    ("mistral", false),    // limits not published
    ("kilo", false),       // 200 an hour: an hour is not what the rule reads
    ("ovh", true),         // 2 a minute
];

#[test]
fn every_free_tier_has_its_answer() {
    for (id, tight) in TABLE {
        let t = tier(id).unwrap_or_else(|| panic!("{id} is not a tier"));
        assert_eq!(t.is_tight(), *tight, "{id}");
    }
}

#[test]
fn a_new_tier_forces_the_question() {
    let known: Vec<&str> = TABLE.iter().map(|(id, _)| *id).collect();
    for t in FREE_TIERS {
        assert!(known.contains(&t.id), "{} is a new tier: say whether it is tight in TABLE", t.id);
    }
    assert_eq!(known.len(), FREE_TIERS.len(), "a row in TABLE names no tier");
}

#[test]
fn the_edges_are_at_most_five_a_minute_and_at_most_a_hundred_a_day() {
    // Leaked: a test's few limits live for the process, as the real table's do.
    let with = |limits: &[Limit]| FreeTier { limits: Box::leak(limits.to_vec().into_boxed_slice()), ..*tier("groq").unwrap() };
    assert!(with(&[rpm(5, PerAccount)]).is_tight());
    assert!(!with(&[rpm(6, PerAccount)]).is_tight());
    assert!(with(&[rpd(100, PerAccount)]).is_tight());
    assert!(!with(&[rpd(101, PerAccount)]).is_tight());
    assert!(!with(&[tpm(1, PerAccount), tpd(10, PerAccount), rph(1, PerAccount)]).is_tight(), "tokens and an hour are not read");
    assert!(!with(&[]).is_tight(), "an unpublished limit is not tight");
}

#[test]
fn the_limit_is_told_in_the_tiers_own_number() {
    assert_eq!(tier("ovh").unwrap().tight_limit().and_then(allows_words).as_deref(), Some("2 requests a minute"));
    assert_eq!(tier("openrouter").unwrap().tight_limit().and_then(allows_words).as_deref(), Some("50 requests a day"));
    assert_eq!(allows_words(&rpm(1, PerModel)).as_deref(), Some("1 request a minute"));
    assert_eq!(allows_words(&rph(200, PerAccount)), None);
    assert_eq!(allows_words(&tpd(10, PerAccount)), None);
    assert_eq!(tier("groq").unwrap().tight_limit(), None);
}

#[test]
fn groqs_day_is_two_limits_each_model_has_its_own() {
    let groq = tier("groq").unwrap();
    let (tokens, requests) = (groq.per_day(Counts::Tokens).unwrap(), groq.per_day(Counts::Requests).unwrap());
    assert_eq!((tokens.amount, requests.amount), (200_000, 1_000));
    assert!(tokens.scope == Scope::PerModel && requests.scope == Scope::PerModel);
}

#[test]
fn a_tier_that_publishes_no_day_has_none() {
    assert!(tier("gemini").unwrap().per_day(Counts::Tokens).is_none());
    assert!(tier("kilo").unwrap().per_day(Counts::Requests).is_none(), "an hour is not a day");
}
