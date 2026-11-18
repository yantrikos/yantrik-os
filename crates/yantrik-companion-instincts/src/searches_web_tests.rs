//! Which instincts search the web, and that every one says so (#1171).
//!
//! Background research is held back by what an instinct *declares* (`Instinct::searches_web`), never
//! by what its urge happens to say: the shipped 4B model is the Medium tier, whose urges say "web
//! search" or "search for" where the Large tier's say `web_search`, and a gate that read the words
//! missed all of them. These tests read the words from the other side: they run every instinct at
//! every model tier and fail on one that tells the model to search without having declared it.

use yantrik_companion_core::background_research;
use yantrik_companion_core::config::InstinctSettings;
use yantrik_companion_core::types::{ModelTier, UrgeSpec};

use crate::load_instincts;
use crate::test_util::{state_at, TEST_NOW};

/// Every instinct that sends the person's interests or place to the web, by name.
const SEARCHING: [&str; 21] = [
    "ActivityRecommender",
    "ConnectionWeaver",
    "ContextBridge",
    "CookingCompanion",
    "DealWatch",
    "DecisionLab",
    "DeepDive",
    "DevilsAdvocate",
    "DreamKeeper",
    "GoldenFind",
    "InterestIntelligence",
    "LocalPulse",
    "MoneyMind",
    "MythBuster",
    "NightOwl",
    "OpportunityScout",
    "PhilosophyCompanion",
    "SkillForge",
    "TraditionKeeper",
    "WonderSense",
    "WorldSense",
];

/// Words that send a model to the web, in any tier's phrasing.
const SEARCH_WORDS: [&str; 6] = ["web_search", "web search", "search for", "search_sources", "browser_search", "browse "];

/// Instincts whose text names a search word only to forbid it ("no web_search"), and their files.
const FORBIDS_SEARCH: [(&str, &str); 2] = [("SecondBrain", "second_brain.rs"), ("GrowthMirror", "growth_mirror.rs")];

/// The default settings with every instinct's switch on, so the ones that ship off are read too.
fn all_on() -> InstinctSettings {
    let mut value = serde_json::to_value(InstinctSettings::default()).unwrap();
    for (key, setting) in value.as_object_mut().unwrap().iter_mut() {
        if key.ends_with("_enabled") {
            *setting = true.into();
        }
    }
    serde_json::from_value(value).unwrap()
}

fn says_search(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    SEARCH_WORDS.iter().any(|w| lower.contains(w))
}

/// What every instinct says over two weeks of hourly ticks at `tier`, so rate limits and time
/// windows open: (declares, instinct, urge). The urge carries the flag as `evaluate_instincts` sets it.
fn everything_said_at(tier: ModelTier) -> Vec<(bool, String, UrgeSpec)> {
    let instincts = load_instincts(&all_on());
    let mut said = Vec::new();
    for tick in 0..(24u32 * 14) {
        let now = TEST_NOW + f64::from(tick) * 3600.0;
        for ago in [600.0, 40_000.0] {
            let mut state = state_at(now, Some(ago));
            state.model_tier = tier;
            state.current_hour = tick % 24;
            state.current_day_of_week = (tick / 24) % 7;
            state.user_interests = vec!["fishing".into(), "rust".into(), "cooking".into()];
            state.user_location = "Austin".into();
            for instinct in &instincts {
                for urge in instinct.evaluate(&state) {
                    let searches = instinct.searches_web();
                    said.push((searches, instinct.name().to_string(), UrgeSpec { searches, ..urge }));
                }
            }
        }
    }
    said
}

const TIERS: [ModelTier; 4] = [ModelTier::Tiny, ModelTier::Small, ModelTier::Medium, ModelTier::Large];

#[test]
fn the_instincts_that_search_are_exactly_the_ones_declared() {
    let mut declared: Vec<String> =
        load_instincts(&all_on()).iter().filter(|i| i.searches_web()).map(|i| i.name().to_string()).collect();
    declared.sort();
    let mut expected: Vec<String> = SEARCHING.iter().map(ToString::to_string).collect();
    expected.sort();
    assert_eq!(declared, expected);
}

#[test]
fn no_instinct_tells_the_model_to_search_at_any_tier_without_declaring_it() {
    for tier in TIERS {
        for (declares, name, urge) in everything_said_at(tier) {
            assert!(
                declares || !says_search(&urge.reason) || FORBIDS_SEARCH.iter().any(|(n, _)| *n == name),
                "{name} at {tier:?} tells the model to search and does not declare `searches_web`: {}",
                urge.reason
            );
        }
    }
}

/// The 4B model the image ships is Medium, whose wording the old gate missed. With research off,
/// every urge that sends interests or place to the web is held back at every tier, and what is left
/// says nothing about searching.
#[test]
fn research_off_holds_back_every_searching_urge_at_every_tier() {
    for tier in TIERS {
        let said = everything_said_at(tier);
        let searching = said.iter().filter(|(declares, ..)| *declares).count();
        if tier != ModelTier::Tiny {
            assert!(searching > 0, "{tier:?}: the test saw no searching urge, so it proves nothing");
        }
        let (kept, held) = background_research::gate(said.into_iter().map(|(.., urge)| urge).collect(), false);
        assert_eq!(held.len(), searching, "{tier:?}");
        for urge in kept {
            assert!(!says_search(&urge.reason) || FORBIDS_SEARCH.iter().any(|(n, _)| *n == urge.instinct_name), "{tier:?} {}: {}", urge.instinct_name, urge.reason);
        }
    }
}

#[test]
fn the_news_watch_urge_of_the_shipped_model_is_held_back() {
    let said = everything_said_at(ModelTier::Medium);
    let news: Vec<&UrgeSpec> = said.iter().filter(|(_, name, _)| name == "WorldSense").map(|(.., urge)| urge).collect();
    assert!(!news.is_empty(), "news watch spoke at the Medium tier");
    assert!(news.iter().all(|u| u.searches && says_search(&u.reason) && !u.reason.contains("web_search")), "the wording the old gate did not read");
    let (kept, held) = background_research::gate(news.into_iter().cloned().collect(), false);
    assert!(kept.is_empty(), "nothing of it reaches the model");
    assert!(held.iter().all(|n| n == "WorldSense"));
}

/// An instinct file that says "search" in an urge must declare it, even for a branch the ticks
/// above never reach.
#[test]
fn every_instinct_source_that_tells_the_model_to_search_declares_it() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        if !file.ends_with(".rs") || ["lib.rs", "test_util.rs", "searches_web_tests.rs"].contains(&file.as_str()) {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let code = source.split("#[cfg(test)]").next().unwrap();
        if says_search(code) && !FORBIDS_SEARCH.iter().any(|(_, f)| *f == file) {
            assert!(code.contains("fn searches_web"), "{file} tells the model to search and does not declare it");
        }
    }
}
