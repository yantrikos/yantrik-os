//! The change-and-plan rules against the real registry: every tool the rules name exists under the
//! category they assume, a real one is refused in the cases the rules give, and the unattended
//! urge ceiling is in the source. This file is its own process.

use yantrik_companion::config::CompanionConfig;
use yantrik_companion::tools::build_registry;
use yantrik_companion_core::background_research::person_turn;
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::taint;
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

const NAMED: [(&str, &str); 24] = [
    ("email_read", "email"), ("email_list", "email"), ("email_search", "email"), ("email_check", "email"),
    ("email_delete", "email"), ("email_move", "email"), ("email_archive", "email"),
    ("calendar_delete_event", "calendar"), ("calendar_update_event", "calendar"),
    ("forget_memory", "memory_hygiene"), ("forget_topic", "memory"),
    ("whatsapp_send", "whatsapp"), ("telegram_send", "telegram"),
    ("write_file", "files"), ("create_recipe", "automation"), ("create_schedule", "scheduler"), ("create_automation", "automation"),
    ("update_schedule", "scheduler"), ("queue_task", "tasks"), ("toggle_automation", "automation"),
    ("email_send", "email"), ("email_reply", "email"), ("email_reply_all", "email"), ("email_forward", "email"),
];

/// The default registry plus the tools a config switches on (mail, calendar, the two chats), so the
/// real ones are there to be refused.
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

/// A scratch home, so the real `write_file` may write there. Set once, before any tool runs.
fn home() -> std::path::PathBuf {
    static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("yantrik-changes-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("the home");
        std::env::set_var("HOME", &home);
        home
    })
    .clone()
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
fn the_real_change_tools_are_refused_after_a_mail_read() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    taint::note_call("email_read", "email", &serde_json::json!({}));
    for name in ["email_delete", "email_move", "email_archive", "calendar_delete_event", "calendar_update_event", "forget_memory",
                 "forget_topic", "whatsapp_send", "telegram_send", "create_recipe", "create_schedule", "create_automation"] {
        let out = run(&reg, name, serde_json::json!({}));
        assert!(out.starts_with("Refused:") && out.contains("`email_read`") && out.contains("Nothing was changed"), "{name}: {out}");
    }
    taint::end_turn();
}

#[test]
fn the_real_mail_and_calendar_changes_and_sends_are_refused_with_nobody_there() {
    let reg = registry();
    for name in ["email_delete", "email_move", "email_archive", "calendar_delete_event", "calendar_update_event", "whatsapp_send", "telegram_send"] {
        let out = run(&reg, name, serde_json::json!({}));
        assert!(out.starts_with("Refused:") && out.contains("nobody is here"), "{name}: {out}");
    }
}

#[test]
fn an_existing_file_is_not_overwritten_after_a_read_by_the_real_tool_and_a_new_one_is() {
    let reg = registry();
    let dir = home().join("docs");
    std::fs::create_dir_all(&dir).expect("make the folder");
    let there = dir.join("there.txt");
    std::fs::write(&there, "old").expect("write");
    taint::begin_turn();
    let _person = person_turn();
    taint::note_call("browse", "browser", &serde_json::json!({}));
    let out = run(&reg, "write_file", serde_json::json!({"path": there.to_string_lossy(), "content": "new"}));
    assert!(out.starts_with("Refused:") && out.contains("`browse`"), "{out}");
    assert_eq!(std::fs::read_to_string(&there).expect("read"), "old", "the refused write changed the file");
    let fresh = dir.join("fresh.txt");
    let out = run(&reg, "write_file", serde_json::json!({"path": fresh.to_string_lossy(), "content": "new"}));
    assert!(!out.starts_with("Refused:"), "{out}");
    assert_eq!(std::fs::read_to_string(&fresh).expect("a new file is written"), "new");
    taint::end_turn();
}

#[test]
fn mail_sends_still_go_to_outward_not_to_this_rule() {
    // A send after a read is not refused by the change rule (the card in outward.rs asks); it is
    // refused or asked by what it always was, never with this rule's sentence.
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    taint::note_call("email_read", "email", &serde_json::json!({}));
    for name in ["email_send", "email_reply", "email_reply_all", "email_forward"] {
        let out = run(&reg, name, serde_json::json!({}));
        assert!(!out.contains("Nothing was changed"), "{name} met the change rule: {out}");
    }
    taint::end_turn();
}

#[test]
fn execute_urges_run_at_most_at_standard() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/background.rs")).expect("read background.rs");
    let line = src.lines().find(|l| l.contains("let max_perm = parse_permission(")).expect("the urge executor's ceiling");
    assert!(line.contains(".min(PermissionLevel::Standard)"), "{line}");
}
