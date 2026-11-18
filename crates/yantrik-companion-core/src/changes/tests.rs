//! The rules of `changes`, driven through the real `ToolRegistry::execute` with stand-in tools
//! that claim a name and a category, which is all the policy reads.

use crate::background_research::person_turn;
use crate::permission::PermissionLevel;
use crate::taint;
use crate::tools::{Tool, ToolContext, ToolRegistry};
use serde_json::json;

struct Fake(&'static str, &'static str);

impl Tool for Fake {
    fn name(&self) -> &'static str {
        self.0
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Standard
    }
    fn category(&self) -> &'static str {
        self.1
    }
    fn definition(&self) -> serde_json::Value {
        json!({})
    }
    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        format!("{} ran", self.0)
    }
}

const READS: [&str; 6] = ["email_read", "email_list", "email_search", "email_check", "calendar_list_events", "calendar_today"];
const MAIL_CHANGES: [&str; 3] = ["email_delete", "email_move", "email_archive"];
const CALENDAR_CHANGES: [&str; 2] = ["calendar_delete_event", "calendar_update_event"];
const MESSAGES: [&str; 2] = ["whatsapp_send", "telegram_send"];
const PLANS: [&str; 4] = ["create_recipe", "create_schedule", "create_automation", "run_recipe"];
/// What the rules do not hold, whatever was read: they wait for the approval card.
const NOT_HELD: [&str; 8] = ["write_file", "edit_file", "forget_memory", "forget_topic", "update_memory", "update_schedule", "queue_task", "remember"];

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    for (name, category) in [
        ("browse", "browser"),
        ("read_file", "files"),
        ("grep", "files"),
        ("vault_get", "vault"),
        ("email_send", "email"),
        ("email_reply", "email"),
        ("email_forward", "email"),
        ("email_reply_all", "email"),
    ] {
        reg.register(Box::new(Fake(name, category)));
    }
    for name in READS.iter().chain(&MAIL_CHANGES).chain(&CALENDAR_CHANGES).chain(&MESSAGES).chain(&PLANS).chain(&NOT_HELD) {
        reg.register(Box::new(Fake(name, "other")));
    }
    reg
}

fn run(reg: &ToolRegistry, name: &str, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    reg.execute(&ctx, name, &args)
}

fn ran(out: &str, name: &str) -> bool {
    out == format!("{name} ran")
}

fn held() -> Vec<&'static str> {
    MAIL_CHANGES.iter().chain(&CALENDAR_CHANGES).chain(&MESSAGES).chain(&PLANS).copied().collect()
}

/// A refusal in the existing style: what was refused, what brought the text in, nothing changed, what to do.
fn assert_refused_after(out: &str, name: &str, source: &str) {
    assert!(out.starts_with("Refused:"), "{name} was not refused: {out}");
    assert!(out.contains(&format!("`{name}`")), "{out}");
    assert!(out.contains(&format!("`{source}`")), "the refusal must name what brought the text in: {out}");
    assert!(out.contains("Nothing was changed"), "{out}");
    assert!(out.contains("in the app") && out.contains("next message"), "it must say its scope: {out}");
    assert!(out.contains("this message"), "{out}");
}

#[test]
fn each_held_tool_is_refused_in_the_persons_turn_after_each_kind_of_read() {
    let reg = registry();
    for reader in READS.iter().chain(&["browse", "read_file", "grep"]) {
        taint::begin_turn();
        let _person = person_turn();
        assert!(ran(&run(&reg, reader, json!({})), reader));
        for name in held() {
            assert_refused_after(&run(&reg, name, json!({})), name, reader);
        }
        taint::end_turn();
    }
}

#[test]
fn a_clean_turn_still_deletes_sends_and_plans() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    for name in held() {
        assert!(ran(&run(&reg, name, json!({})), name), "{name} was refused in a clean turn");
    }
    taint::end_turn();
}

#[test]
fn what_the_rules_do_not_hold_still_runs_after_a_read() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    assert!(ran(&run(&reg, "email_read", json!({})), "email_read"));
    for name in NOT_HELD {
        assert!(ran(&run(&reg, name, json!({"path": "/tmp/x", "content": "y"})), name), "{name}");
    }
    taint::end_turn();
}

#[test]
fn mail_sends_are_not_this_rules_and_are_not_untrusted_reads() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    for send in ["email_send", "email_reply", "email_reply_all", "email_forward"] {
        assert!(!super::reads_mail(send) && !super::is_change(send), "{send}");
        assert!(ran(&run(&reg, send, json!({})), send));
    }
    assert!(ran(&run(&reg, "email_delete", json!({})), "email_delete"), "a send does not taint the turn");
    taint::end_turn();
}

#[test]
fn with_nobody_there_the_mail_and_calendar_changes_and_the_two_sends_are_refused() {
    let reg = registry();
    let nobody: Vec<&str> = MAIL_CHANGES.iter().chain(&CALENDAR_CHANGES).chain(&MESSAGES).copied().collect();
    // A turn nobody started, and no turn at all (recipe steps, urges).
    for with_a_turn in [true, false] {
        if with_a_turn {
            taint::begin_turn();
        } else {
            taint::end_turn();
        }
        for name in &nobody {
            let out = run(&reg, name, json!({}));
            assert!(out.starts_with("Refused:") && out.contains(&format!("`{name}`")), "{name}: {out}");
            assert!(out.contains("nobody is here") && out.contains("Nothing was changed"), "{out}");
        }
    }
    taint::end_turn();
}

#[test]
fn planning_unattended_is_allowed_until_the_turn_reads_something() {
    let reg = registry();
    taint::begin_turn();
    for name in PLANS {
        assert!(ran(&run(&reg, name, json!({})), name), "{name}");
    }
    assert!(ran(&run(&reg, "email_read", json!({})), "email_read"));
    for name in PLANS {
        assert_refused_after(&run(&reg, name, json!({})), name, "email_read");
    }
    taint::end_turn();
}

#[test]
fn a_new_turn_is_not_held_back_by_what_the_last_one_read() {
    // The scope the sentence names: this message. The card, not this rule, protects the next one.
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    assert!(ran(&run(&reg, "email_search", json!({})), "email_search"));
    assert!(run(&reg, "create_schedule", json!({})).starts_with("Refused:"));
    taint::begin_turn();
    assert!(ran(&run(&reg, "create_schedule", json!({})), "create_schedule"));
    taint::end_turn();
}

#[test]
fn mail_and_calendar_reads_are_sources_for_the_credential_rules_too() {
    let reg = registry();
    for reader in READS {
        taint::begin_turn();
        assert!(ran(&run(&reg, reader, json!({})), reader));
        let out = run(&reg, "vault_get", json!({}));
        assert!(out.starts_with("Refused:") && out.contains(&format!("`{reader}`")), "{reader}: {out}");
        taint::end_turn();
    }
}

#[test]
fn a_fetched_page_is_an_untrusted_source_and_the_fetch_stays_outward() {
    let mut reg = registry();
    for name in ["web_fetch", "http_fetch", "clipboard_fetch_url"] {
        reg.register(Box::new(Fake(name, "network")));
    }
    for fetch in ["web_fetch", "http_fetch", "clipboard_fetch_url"] {
        taint::begin_turn();
        let _person = person_turn();
        assert!(ran(&run(&reg, fetch, json!({})), fetch));
        for name in held() {
            assert_refused_after(&run(&reg, name, json!({})), name, fetch);
        }
        taint::end_turn();
    }
    // Still outward for the credential rules: a secret and a page in one turn stop the fetch, as before.
    taint::begin_turn();
    assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"));
    assert!(ran(&run(&reg, "browse", json!({})), "browse"));
    let out = run(&reg, "web_fetch", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("can send data outward"), "{out}");
    taint::end_turn();
}

#[test]
fn read_file_and_grep_are_classified_as_on_main() {
    let reg = registry();
    for reader in ["read_file", "grep"] {
        taint::begin_turn();
        assert!(ran(&run(&reg, reader, json!({"path": "~/Documents/todo.md"})), reader));
        let out = run(&reg, "vault_get", json!({}));
        assert!(out.starts_with("Refused:") && out.contains(reader), "{reader} must stay an untrusted source: {out}");
        taint::end_turn();
    }
}

#[test]
fn what_a_sub_agent_read_is_merged_into_its_parent() {
    let reg = std::sync::Arc::new(registry());
    taint::begin_turn();
    let _person = person_turn();
    let carried = (taint::carry(), crate::background_research::carry());
    let child = reg.clone();
    let read = std::thread::spawn(move || {
        taint::adopt(carried.0);
        let _p = crate::background_research::adopt(carried.1);
        assert!(ran(&run(&child, "email_read", json!({})), "email_read"));
        taint::state().0
    })
    .join()
    .expect("the sub-agent's thread");
    assert_eq!(read.as_deref(), Some("email_read"));
    assert!(ran(&run(&reg, "create_schedule", json!({})), "create_schedule"), "the parent has not taken it yet");
    taint::note(read.as_deref().unwrap_or_default(), "browser");
    assert_refused_after(&run(&reg, "create_schedule", json!({})), "create_schedule", "email_read");
    taint::end_turn();
}

#[test]
fn the_named_app_actions_are_held_and_no_others() {
    use crate::changes::check_app_action;
    // Unattended: all three.
    taint::begin_turn();
    for (app, action) in [("calendar", "delete_event"), ("app-calendar", "update_event"), ("shell", "files_delete"), ("app-shell", "files_delete"), ("files", "files_delete")] {
        let said = check_app_action(app, action).expect_err("nobody is here");
        assert!(said.contains("nobody is here") && said.contains("Nothing was changed"), "{said}");
    }
    taint::end_turn();
    // The person's turn: clean it runs; after a read the calendar ones are refused, file deletion waits for the card.
    taint::begin_turn();
    let _person = person_turn();
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event"), ("shell", "files_delete"), ("files", "files_delete")] {
        assert!(check_app_action(app, action).is_ok(), "{app}.{action} in a clean turn");
    }
    taint::note("email_read", "email");
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event")] {
        let said = check_app_action(app, action).expect_err("refused after a read");
        assert!(said.contains("`email_read`") && said.contains("Nothing was changed") && said.contains("next message"), "{said}");
    }
    assert!(check_app_action("shell", "files_delete").is_ok());
    for (app, action) in [("notes", "purge"), ("calendar", "list"), ("files", "files_open"), ("shell", "open_app"), ("terminal", "run")] {
        assert!(check_app_action(app, action).is_ok(), "{app}.{action} is not named");
    }
    taint::end_turn();
}
