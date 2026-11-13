//! With background research on and nobody there, only `web_search` may reach the web (#1175, round 5,
//! medium 3). The card promises "your search terms, to your own SearXNG or else DuckDuckGo", which a
//! page, a package, a probe or a tool server's call does not keep, so turning research on opens nothing
//! but the search. This is its own test binary because it turns the switch on for the whole process.

mod common;

use common::tools;
use yantrik_companion::tools::build_registry;
use yantrik_companion_core::background_research;
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::reach::{check_call, reach, Reach};
use yantrik_companion_core::tools::ToolContext;

/// Arguments that make the ones whose reach depends on them as wide as a model could.
fn widest() -> serde_json::Value {
    serde_json::json!({ "app": "terminal", "host": "a.host-the-model-made-up.example", "sandbox": "network", "query": "rust news", "url": "https://example.com" })
}

#[test]
fn research_on_and_nobody_there_refuses_every_registered_tool_but_web_search_unless_it_is_local_or_service() {
    background_research::init(true, None);
    assert!(background_research::is_on() && !background_research::person_started());
    let (mut refused, mut ran) = (0, Vec::new());
    for (name, category) in tools() {
        let class = reach(name, category, &widest());
        let verdict = check_call(name, category, &widest());
        match class {
            Reach::Local | Reach::Service => assert_eq!(verdict, Ok(()), "{name} is {class:?}"),
            _ if name == "web_search" => {
                assert_eq!(class, Reach::Web);
                assert_eq!(verdict, Ok(()), "research is on: the search is the one thing it unlocks");
                ran.push(name);
            }
            _ => {
                assert!(verdict.as_ref().is_err_and(|said| said.starts_with("Refused:")), "{name} ({class:?}) ran with research on and nobody there");
                refused += 1;
            }
        }
    }
    assert_eq!(ran, ["web_search"], "nothing else is let through by the switch");
    assert!(refused >= 20, "only {refused} tools were refused");
}

/// The same through the registry that runs them, so a path that skips `check_call` would show.
#[test]
fn the_registry_runs_none_of_them_either() {
    background_research::init(true, None);
    let reg = build_registry(&common::config_with_everything_on());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    for (name, category) in tools() {
        if name != "web_search" && matches!(reach(name, category, &widest()), Reach::Web | Reach::Browser) {
            let said = reg.execute(&ctx, name, &widest());
            assert!(said.starts_with("Refused:"), "`{name}` ran with research on and nobody there: {said}");
        }
    }
}
