//! The nobody-there rule and the sources, against the real registry: every tool the rules name exists under the
//! category they assume, and real ones are refused with nobody there. This file is its own process.

use yantrik_companion::config::CompanionConfig;
use yantrik_companion::tools::build_registry;
use yantrik_companion_core::background_research::person_turn;
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::taint;
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

const NAMED: [(&str, &str); 17] = [
    ("email_read", "email"), ("email_list", "email"), ("email_search", "email"), ("email_check", "email"),
    ("email_delete", "email"), ("email_move", "email"), ("email_archive", "email"),
    ("calendar_list_events", "calendar"), ("calendar_today", "calendar"),
    ("calendar_delete_event", "calendar"), ("calendar_update_event", "calendar"),
    ("whatsapp_send", "whatsapp"), ("telegram_send", "telegram"),
    ("web_fetch", "network"), ("http_fetch", "network"), ("clipboard_fetch_url", "clipboard"),
    ("vault_get", "vault"),
];

/// The default registry plus the tools a config switches on (mail, calendar, the two chats).
fn registry() -> ToolRegistry {
    let mut reg = build_registry(&CompanionConfig::default());
    let account: yantrik_companion_core::config::EmailAccountConfig =
        serde_json::from_value(serde_json::json!({"name": "Test", "email": "a@example.test", "provider": "imap"})).expect("an account");
    yantrik_companion::tools::email::register(&mut reg, vec![account]);
    yantrik_companion::tools::calendar::register(&mut reg, Vec::new(), None);
    yantrik_companion::tools::telegram::register(&mut reg, "token", "chat");
    yantrik_companion::tools::whatsapp::register(&mut reg, "id", "token", "recipient");
    reg
}

fn run(reg: &ToolRegistry, name: &str, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    reg.execute(&ctx, name, &args)
}

#[test]
fn every_tool_the_rules_name_is_a_real_tool_in_the_category_they_assume() {
    let reg = registry();
    let meta = reg.list_metadata(PermissionLevel::Dangerous);
    for (name, category) in NAMED {
        let found = meta.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("`{name}` is not a registered tool: the rules name something that is gone"));
        assert_eq!(found.category, category, "{name}");
    }
}

#[test]
fn the_real_mail_and_calendar_changes_and_sends_are_refused_with_nobody_there() {
    let reg = registry();
    for name in ["email_delete", "email_move", "email_archive", "calendar_delete_event", "calendar_update_event", "whatsapp_send", "telegram_send"] {
        let out = run(&reg, name, serde_json::json!({}));
        assert!(out.starts_with("Refused:") && out.contains("nobody is here") && out.contains("Nothing was changed"), "{name}: {out}");
    }
}

#[test]
fn the_persons_own_turn_is_not_met_by_the_nobody_there_rule() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    for name in ["email_delete", "calendar_delete_event", "whatsapp_send"] {
        let out = run(&reg, name, serde_json::json!({}));
        assert!(!out.contains("nobody is here"), "{name}: {out}");
    }
    taint::end_turn();
}

#[test]
fn a_real_fetch_then_a_credential_is_refused_and_the_fetch_stays_outward() {
    let reg = registry();
    for fetch in ["web_fetch", "http_fetch"] {
        taint::begin_turn();
        let _person = person_turn();
        let _ = run(&reg, fetch, serde_json::json!({"url": "http://127.0.0.1:9/"})); // may fail here (no network); noted either way
        let out = run(&reg, "vault_get", serde_json::json!({}));
        assert!(out.starts_with("Refused:") && out.contains(&format!("`{fetch}`")) && out.contains("returns a credential"), "{fetch}: {out}");
        taint::end_turn();
    }
}

#[test]
fn mail_and_calendar_reads_are_sources_for_the_real_credential_rule() {
    let reg = registry();
    for reader in ["email_read", "calendar_today"] {
        taint::begin_turn();
        let _person = person_turn();
        taint::note_call(reader, "email", &serde_json::json!({}));
        let out = run(&reg, "vault_get", serde_json::json!({}));
        assert!(out.starts_with("Refused:") && out.contains(&format!("`{reader}`")), "{reader}: {out}");
        taint::end_turn();
    }
}

#[test]
fn execute_urges_run_at_most_at_standard() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/background.rs")).expect("read background.rs");
    let line = src.lines().find(|l| l.contains("let max_perm = parse_permission(")).expect("the urge executor's ceiling");
    assert!(line.contains(".min(PermissionLevel::Standard)"), "{line}");
}

/// The registered `app_action` tool, through `ToolRegistry::execute`, with nobody there: the named actions are refused
/// before any app is started (guarded by the `check_app_action` call in `app_act::refused_first`).
#[test]
fn the_real_app_action_tool_refuses_the_named_actions_with_nobody_there() {
    let reg = registry();
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event"), ("shell", "files_delete")] {
        let out = run(&reg, "app_action", serde_json::json!({"app": app, "action": action}));
        assert!(out.starts_with("Refused:") && out.contains("nobody is here") && out.contains(action), "{app}.{action}: {out}");
    }
}

/// A scripted model that asks for one tool once, then answers.
struct AsksForAFetch(std::sync::Mutex<bool>, &'static str);

impl yantrik_ml::LLMBackend for AsksForAFetch {
    fn chat(&self, _: &[yantrik_ml::ChatMessage], _: &yantrik_ml::GenerationConfig, _: Option<&[serde_json::Value]>) -> anyhow::Result<yantrik_ml::LLMResponse> {
        let first = !std::mem::replace(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()), true);
        Ok(yantrik_ml::LLMResponse {
            text: if first { String::new() } else { "done".into() },
            prompt_tokens: 0,
            completion_tokens: 1,
            tool_calls: if first { vec![yantrik_ml::ToolCall { name: self.1.into(), arguments: serde_json::json!({"url": "http://127.0.0.1:9/"}) }] } else { vec![] },
            api_tool_calls: vec![],
            stop_reason: "stop".into(),
        })
    }
    fn chat_streaming(&self, m: &[yantrik_ml::ChatMessage], c: &yantrik_ml::GenerationConfig, t: Option<&[serde_json::Value]>, _: &mut dyn FnMut(&str)) -> anyhow::Result<yantrik_ml::LLMResponse> {
        self.chat(m, c, t)
    }
    fn count_tokens(&self, text: &str) -> anyhow::Result<usize> {
        Ok(text.len())
    }
    fn backend_name(&self) -> &str {
        "asks-for-a-fetch"
    }
}

/// `spawn_parallel_agents` for real: a sub-agent fetches a page; the parent, which ran nothing itself, is then refused a
/// credential, and the refusal names the fetch (guarded by the merge in `sub_agent::spawn_parallel_agents`).
#[test]
fn what_a_real_sub_agent_fetched_is_merged_into_its_parent_for_the_secrets_rule() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    let mut companion_config = CompanionConfig::default();
    companion_config.tools.max_permission = "dangerous".into();
    let config = yantrik_companion::sub_agent::SubAgentConfig {
        max_steps: 3,
        max_tokens: 64,
        temperature: 0.0,
        user_name: "Test".into(),
        db_path: ":memory:".into(),
        embedding_dim: 384,
        companion_config,
        ceiling: PermissionLevel::Dangerous,
        incognito: true,
    };
    let results = yantrik_companion::sub_agent::spawn_parallel_agents(
        vec!["fetch a page".into()],
        std::sync::Arc::new(AsksForAFetch(std::sync::Mutex::new(false), "web_fetch")),
        config,
    );
    assert!(results[0].tool_calls_made.contains(&"web_fetch".to_string()), "the sub-agent never fetched: the test proves nothing: {:?}", results[0].tool_calls_made);
    let out = run(&reg, "vault_get", serde_json::json!({}));
    assert!(out.starts_with("Refused:") && out.contains("`web_fetch`") && out.contains("returns a credential"), "{out}");
    taint::end_turn();
}

/// A sub-agent that ran `vault_get` leaves the parent holding the secret too: the sub-agent took a credential, the parent
/// then reads a page, and is refused an outward call (guarded by the merge of the secret half in
/// `sub_agent::spawn_parallel_agents`).
#[test]
fn what_a_real_sub_agent_took_in_as_a_secret_is_the_parents_too() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    let mut companion_config = CompanionConfig::default();
    companion_config.tools.max_permission = "dangerous".into();
    let config = yantrik_companion::sub_agent::SubAgentConfig {
        max_steps: 3,
        max_tokens: 64,
        temperature: 0.0,
        user_name: "Test".into(),
        db_path: ":memory:".into(),
        embedding_dim: 384,
        companion_config,
        ceiling: PermissionLevel::Dangerous,
        incognito: true,
    };
    let results = yantrik_companion::sub_agent::spawn_parallel_agents(
        vec!["look something up".into()],
        std::sync::Arc::new(AsksForAFetch(std::sync::Mutex::new(false), "vault_get")),
        config,
    );
    assert!(results[0].tool_calls_made.contains(&"vault_get".to_string()), "the sub-agent never ran vault_get: the test proves nothing: {:?}", results[0].tool_calls_made);
    taint::note("web_fetch", "network"); // then the parent reads a page itself
    let out = run(&reg, "web_fetch", serde_json::json!({"url": "https://127.0.0.1:9/"}));
    assert!(out.starts_with("Refused:") && out.contains("`vault_get`") && out.contains("`web_fetch`"), "{out}");
    taint::end_turn();
}
