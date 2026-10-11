//! Put natural requests to a judge over the companion's whole tool catalogue, and show what
//! routing saves in the schemas a chat model is sent.
//!
//!   JUDGE_URL=https://api.typesafe.ai JUDGE_MODEL=jev-latest JUDGE_KEY_ENV=JEV_API_KEY \
//!     cargo run --release -p yantrik-companion --example judge_probe
//!   JUDGE_URL=http://127.0.0.1:8009 JUDGE_MODEL=kev-latest cargo run ... --example judge_probe
//!
//!   JUDGE_URL=http://127.0.0.1:11434 JUDGE_MODEL=nimble cargo run ... --example judge_probe   # Ollama 0.35+
//!
//! The whole catalogue is offered at once, which is harder than the similarity shortlist the
//! companion offers, so the accuracy here is a floor. Kev in particular spreads its probability
//! thin over this many options (on 2026-09-26: 11/14 right, but no pick above 0.35 bar two), so
//! it clears `route_at` only on the 20-tool shortlist, as measured on 2026-09-22 (69% of
//! requests at p >= 0.7, 98% of those right).

use std::time::{Duration, Instant};

use yantrik_companion::companion::CORE_TOOLS;
use yantrik_companion::config::CompanionConfig;
use yantrik_companion::judge_route::{ask, decide, NO_TOOL};
use yantrik_companion::tool_cache::build_compact_card;
use yantrik_companion::tools::{build_registry, PermissionLevel};
use yantrik_ml::judge::{Judge, SystemOneJudge};

const REQUESTS: &[(&str, &[&str])] = &[
    ("what's it like outside in Dallas right now?", &["get_weather"]),
    ("do I have anything on this afternoon?", &["calendar_today", "calendar_list_events"]),
    ("remind me to call mom at 6", &["set_reminder"]),
    ("any new mail from Sam?", &["email_search", "email_check", "email_list"]),
    ("how much room is left on my drive", &["disk_usage"]),
    ("what's 18% of 2450", &["calculate"]),
    ("keep my github token somewhere safe: ghp_example", &["vault_store"]),
    ("the fan is going crazy, what's eating the CPU?", &["list_processes", "diagnose_process", "system_info"]),
    ("find the notes I wrote about the Goa trip", &["search_files", "life_search"]),
    ("thanks, that's all for now", &[NO_TOOL]),
    ("look up reviews of the Framework 16 laptop", &["web_search", "browser_search", "search_sources"]),
    ("tell Priya I'll be there, answering her last mail", &["email_reply", "email_search"]),
    ("what did I tell you about my allergies?", &["recall", "recall_preferences"]),
    ("grab the release notes page from blog.rust-lang.org", &["web_fetch", "http_fetch", "browse"]),
];

fn main() -> Result<(), String> {
    let url = std::env::var("JUDGE_URL").unwrap_or_else(|_| "http://127.0.0.1:8009".into());
    let model = std::env::var("JUDGE_MODEL").unwrap_or_else(|_| "kev-latest".into());
    let key_env = std::env::var("JUDGE_KEY_ENV").ok();
    let judge = SystemOneJudge::new(&url, &model, key_env.as_deref(), Duration::from_secs(30));

    let mut config = CompanionConfig::default();
    config.tools.enabled = true;
    let registry = build_registry(&config);
    let defs = registry.definitions_for(CORE_TOOLS, PermissionLevel::Dangerous);
    let catalogue: Vec<(f32, String, String)> = defs
        .iter()
        .map(|d| (0.0, d["function"]["name"].as_str().unwrap_or("").to_string(), build_compact_card(d)))
        .collect();
    println!("{} tools offered to {model} at {url}\n", catalogue.len());

    let size = |names: &[&str]| -> usize {
        registry.definitions_for(names, PermissionLevel::Dangerous).iter().map(|d| d.to_string().len()).sum()
    };
    let (mut right, mut followed, mut followed_right, mut ms_total) = (0, 0, 0, 0u128);
    // A judge that takes fewer options than the catalogue (Ollama's take 26) is offered, per
    // request, the tools that answer it and then distractors in catalogue order up to its cap —
    // the size of the shortlist the companion really sends, without the similarity ranking.
    let cap = judge.max_choice_options();
    if let Some(cap) = cap {
        println!("{model} takes at most {cap} options: each request offers its answer plus distractors, {} in all
", cap - 1);
    }
    for (request, want) in REQUESTS {
        let offered: Vec<(f32, String, String)> = match cap {
            None => catalogue.clone(),
            Some(cap) => {
                let (mut first, rest): (Vec<_>, Vec<_>) =
                    catalogue.iter().cloned().partition(|(_, name, _)| want.contains(&name.as_str()));
                first.extend(rest);
                first.truncate(cap - 1);
                first
            }
        };
        let started = Instant::now();
        let r = ask(&judge, request, &[], &offered)?;
        let ms = started.elapsed().as_millis();
        ms_total += ms;
        let pick = r.pick.clone().unwrap_or_else(|| NO_TOOL.into());
        let ok = want.contains(&pick.as_str());
        let decision = decide(&r, 0.7);
        let follows = !matches!(decision, yantrik_companion::judge_route::Decision::Fallback);
        right += ok as usize;
        followed += follows as usize;
        followed_right += (follows && ok) as usize;
        println!("{} {:<52} -> {:<22} p={:.2} multi={:.2} {ms:>5} ms  {:?}",
                 if ok { "ok  " } else { "MISS" }, request, pick, r.p, r.multi_step, decision);
    }
    let n = REQUESTS.len();
    println!("\ntop-1 {right}/{n}; followed at p>=0.7: {followed}/{n}, of which right {followed_right}; mean {} ms", ms_total / n as u128);

    // What a routed prompt sends, against the ordinary shortlist for a Medium-tier model (20 tools).
    let ordinary: Vec<&str> = CORE_TOOLS.iter().copied().take(20).collect();
    let routed: Vec<&str> = vec!["discover_tools", "get_weather"];
    let (a, b) = (size(&ordinary), size(&routed));
    println!("tool schemas: ordinary 20 tools {a} chars (~{} tokens), routed {} tools {b} chars (~{} tokens): {:.0}% fewer",
             a / 4, routed.len(), b / 4, 100.0 * (1.0 - b as f64 / a as f64));
    Ok(())
}
