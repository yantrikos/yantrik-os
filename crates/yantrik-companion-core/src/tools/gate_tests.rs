//! Does the gate actually fire?
//!
//! `taint`'s own tests check the policy. These drive the real [`ToolRegistry::execute`] with
//! real tools, because a correct policy that is never consulted protects nothing — and the
//! wiring is one forgotten line, in a function that has two other early returns.

use super::*;
use crate::permission::PermissionLevel;

/// A tool that does nothing but claim a name and a category, which is all the policy reads.
struct Fake {
    name: &'static str,
    category: &'static str,
}

impl Tool for Fake {
    fn name(&self) -> &'static str {
        self.name
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn category(&self) -> &'static str {
        self.category
    }
    fn definition(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        format!("{} ran", self.name)
    }
}

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(Fake { name: "browse", category: "browser" }));
    reg.register(Box::new(Fake { name: "vault_get", category: "vault" }));
    reg.register(Box::new(Fake { name: "browser_login", category: "browser" }));
    reg.register(Box::new(Fake { name: "run_command", category: "system" }));
    reg
}

/// The audit log writes to the database, so a real one is needed even though nothing reads it
/// back. In memory, so the test leaves nothing behind.
fn db() -> yantrikdb_core::YantrikDB {
    yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database")
}

fn ctx(db: &yantrikdb_core::YantrikDB) -> ToolContext<'_> {
    ToolContext {
        db,
        max_permission: PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    }
}

#[test]
fn a_page_cannot_make_the_agent_fetch_a_credential() {
    let db = db();
    let ctx = ctx(&db);
    let reg = registry();
    crate::taint::begin_turn();
    let _person = crate::background_research::person_turn();

    assert_eq!(reg.execute(&ctx, "browse", &serde_json::json!({})), "browse ran");

    let refused = reg.execute(&ctx, "vault_get", &serde_json::json!({}));
    assert!(refused.starts_with("Refused:"), "the gate did not fire: {refused}");
    assert!(refused.contains("browse"), "and it must name what caused it: {refused}");
}

#[test]
fn the_ordinary_case_is_untouched() {
    // If the rule breaks ordinary work it will be turned off, and then it protects nothing.
    let db = db();
    let ctx = ctx(&db);
    let reg = registry();
    crate::taint::begin_turn();
    let _person = crate::background_research::person_turn();

    assert_eq!(reg.execute(&ctx, "browse", &serde_json::json!({})), "browse ran");
    assert_eq!(
        reg.execute(&ctx, "run_command", &serde_json::json!({})),
        "run_command ran",
        "no credential is in play, so there is nothing to leak"
    );
}

#[test]
fn logging_in_mid_session_still_works() {
    let db = db();
    let ctx = ctx(&db);
    let reg = registry();
    crate::taint::begin_turn();
    let _person = crate::background_research::person_turn();

    reg.execute(&ctx, "browse", &serde_json::json!({}));
    assert_eq!(
        reg.execute(&ctx, "browser_login", &serde_json::json!({})),
        "browser_login ran",
        "a tool that never returns the secret must stay usable after browsing"
    );
}

#[test]
fn the_trifecta_is_refused_through_the_registry() {
    let db = db();
    let ctx = ctx(&db);
    let reg = registry();
    crate::taint::begin_turn();
    let _person = crate::background_research::person_turn();

    reg.execute(&ctx, "vault_get", &serde_json::json!({}));
    reg.execute(&ctx, "browse", &serde_json::json!({}));

    let refused = reg.execute(&ctx, "run_command", &serde_json::json!({}));
    assert!(refused.starts_with("Refused:"), "{refused}");
    assert!(refused.contains("vault_get") && refused.contains("browse"), "{refused}");
}

#[test]
fn a_failed_tool_does_not_taint_what_follows() {
    // `note` runs after execute, so a tool that never produced content never made the turn
    // untrusted. Checked because the ordering is easy to get backwards.
    let db = db();
    let ctx = ctx(&db);
    let reg = registry();
    crate::taint::begin_turn();
    let _person = crate::background_research::person_turn();

    let unknown = reg.execute(&ctx, "no_such_tool", &serde_json::json!({}));
    assert!(unknown.starts_with("Unknown tool"));
    assert_eq!(
        reg.execute(&ctx, "vault_get", &serde_json::json!({})),
        "vault_get ran",
        "a tool that did not run cannot have brought anything in"
    );
}

/// The rule about who started the turn (`crate::reach`), through the real dispatch: what a model
/// reaches for after a refused search must not run either.
mod reach_through_the_registry {
    use super::*;

    const WEB: &[(&str, &str)] = &[
        ("web_search", "browser"),
        ("browse", "browser"),
        ("launch_browser", "browser"),
        ("browser_search", "browser"),
        ("search_sources", "life_assistant"),
        ("http_fetch", "network"),
        ("web_fetch", "network"),
    ];

    fn registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        for &(name, category) in WEB {
            reg.register(Box::new(Fake { name, category }));
        }
        reg.register(Box::new(Fake { name: "read_file", category: "files" }));
        reg
    }

    #[test]
    fn nothing_that_reaches_the_web_runs_in_a_turn_nobody_started() {
        let db = db();
        let ctx = ctx(&db);
        let reg = registry();
        for (name, _) in WEB {
            let said = reg.execute(&ctx, name, &serde_json::json!({}));
            assert!(said.starts_with("Refused:") && !said.ends_with("ran"), "{name} ran unattended: {said}");
        }
        assert_eq!(reg.execute(&ctx, "read_file", &serde_json::json!({})), "read_file ran", "a local tool is untouched");
    }

    #[test]
    fn a_turn_the_person_started_still_searches_and_browses() {
        let db = db();
        let ctx = ctx(&db);
        let reg = registry();
        let _person = crate::background_research::person_turn();
        for (name, _) in WEB {
            assert_eq!(reg.execute(&ctx, name, &serde_json::json!({})), format!("{name} ran"));
        }
    }
}
