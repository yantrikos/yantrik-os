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

const MAIL_READS: [&str; 4] = ["email_read", "email_list", "email_search", "email_check"];
const MAIL_CHANGES: [&str; 3] = ["email_delete", "email_move", "email_archive"];
const CALENDAR_CHANGES: [&str; 2] = ["calendar_delete_event", "calendar_update_event"];
const MESSAGES: [&str; 2] = ["whatsapp_send", "telegram_send"];
const FORGETS: [&str; 3] = ["forget_memory", "forget_topic", "update_memory"];
const PLANS: [&str; 6] = ["create_recipe", "create_schedule", "create_automation", "update_schedule", "toggle_automation", "queue_task"];
const OTHER_CHANGES: [&str; 7] =
    ["edit_file", "script_patch", "script_delete", "manage_files", "vault_delete", "delete_automation", "cancel_schedule"];

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    for (name, category) in [
        ("browse", "browser"),
        ("read_file", "files"),
        ("write_file", "files"),
        ("email_send", "email"),
        ("email_reply", "email"),
        ("email_forward", "email"),
        ("email_reply_all", "email"),
        ("recall", "memory"),
        ("remember", "memory"),
    ] {
        reg.register(Box::new(Fake(name, category)));
    }
    for name in MAIL_READS.iter().chain(&MAIL_CHANGES) {
        reg.register(Box::new(Fake(name, "email")));
    }
    for name in CALENDAR_CHANGES {
        reg.register(Box::new(Fake(name, "calendar")));
    }
    for name in MESSAGES {
        reg.register(Box::new(Fake(name, name.trim_end_matches("_send"))));
    }
    for name in FORGETS {
        reg.register(Box::new(Fake(name, "memory")));
    }
    for name in PLANS.iter().chain(&OTHER_CHANGES) {
        reg.register(Box::new(Fake(name, "automation")));
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

/// A refusal in the existing style: what was refused, what brought the text in, nothing changed,
/// and what the person can do.
fn assert_refused_after(out: &str, name: &str, source: &str) {
    assert!(out.starts_with("Refused:"), "{name} was not refused: {out}");
    assert!(out.contains(&format!("`{name}`")), "{out}");
    assert!(out.contains(&format!("`{source}`")), "the refusal must name what brought the text in: {out}");
    assert!(out.contains("Nothing was changed"), "{out}");
    assert!(out.contains("in the app") && out.contains("new conversation"), "{out}");
}

/// Every change and plan, with arguments that make it one.
fn every_change() -> Vec<(&'static str, serde_json::Value)> {
    let scratch = std::env::temp_dir().join(format!("yantrik-changes-{}-exists.txt", std::process::id()));
    std::fs::write(&scratch, "old").expect("write the scratch file");
    let mut all: Vec<(&'static str, serde_json::Value)> = Vec::new();
    for name in MAIL_CHANGES.iter().chain(&CALENDAR_CHANGES).chain(&MESSAGES).chain(&FORGETS).chain(&PLANS).chain(&OTHER_CHANGES) {
        all.push((name, json!({})));
    }
    all.push(("write_file", json!({"path": scratch.to_string_lossy(), "content": "new"})));
    all
}

fn person_after(reader: &str, category: &str, reg: &ToolRegistry) -> (crate::background_research::PersonTurn, ()) {
    taint::begin_turn();
    let guard = person_turn();
    let out = run(reg, reader, json!({"path": "/tmp/x"}));
    assert!(ran(&out, reader), "{reader} ({category}) did not run: {out}");
    (guard, ())
}

#[test]
fn each_change_is_refused_in_the_persons_turn_after_each_mail_read() {
    let reg = registry();
    for reader in MAIL_READS {
        let (_person, _) = person_after(reader, "email", &reg);
        for (name, args) in every_change() {
            assert_refused_after(&run(&reg, name, args), name, reader);
        }
        taint::end_turn();
    }
}

#[test]
fn each_change_is_refused_in_the_persons_turn_after_a_page_or_a_file() {
    let reg = registry();
    for reader in ["browse", "read_file"] {
        let (_person, _) = person_after(reader, "any", &reg);
        for (name, args) in every_change() {
            assert_refused_after(&run(&reg, name, args), name, reader);
        }
        taint::end_turn();
    }
}

#[test]
fn a_clean_turn_still_changes_and_plans() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    for (name, args) in every_change() {
        let out = run(&reg, name, args);
        assert!(ran(&out, name), "{name} was refused in a clean turn: {out}");
    }
    // Reading things that are the person's own is not reading someone else's text.
    assert!(ran(&run(&reg, "recall", json!({})), "recall"));
    assert!(ran(&run(&reg, "email_delete", json!({})), "email_delete"));
    taint::end_turn();
}

#[test]
fn a_new_file_is_still_written_after_a_read_and_so_is_an_append() {
    let reg = registry();
    let dir = std::env::temp_dir().join(format!("yantrik-changes-{}-new", std::process::id()));
    let fresh = dir.join("not-there-yet.txt");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the folder");
    let existing = dir.join("there.txt");
    std::fs::write(&existing, "old").expect("write");
    taint::begin_turn();
    let _person = person_turn();
    assert!(ran(&run(&reg, "email_read", json!({})), "email_read"));
    assert!(ran(&run(&reg, "write_file", json!({"path": fresh.to_string_lossy(), "content": "x"})), "write_file"));
    assert!(ran(&run(&reg, "write_file", json!({"path": existing.to_string_lossy(), "content": "x", "append": true})), "write_file"));
    let over = run(&reg, "write_file", json!({"path": existing.to_string_lossy(), "content": "x", "append": false}));
    assert_refused_after(&over, "write_file", "email_read");
    let over = run(&reg, "write_file", json!({"path": existing.to_string_lossy(), "content": "x"}));
    assert_refused_after(&over, "write_file", "email_read");
    // Ordinary tools stay as they were.
    assert!(ran(&run(&reg, "remember", json!({})), "remember"));
    assert!(ran(&run(&reg, "recall", json!({})), "recall"));
    taint::end_turn();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mail_sends_are_not_this_rules_and_are_not_untrusted_reads() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    for send in ["email_send", "email_reply", "email_reply_all", "email_forward"] {
        assert!(!reads_or_changes(send), "{send}");
        assert!(ran(&run(&reg, send, json!({})), send));
    }
    // A send does not taint the turn, so the change after it still runs.
    assert!(ran(&run(&reg, "email_delete", json!({})), "email_delete"));
    taint::end_turn();
}

fn reads_or_changes(name: &str) -> bool {
    super::reads_mail(name) || super::is_change(name, &json!({}))
}

#[test]
fn the_mail_and_calendar_changes_and_the_two_sends_are_refused_with_nobody_there() {
    let reg = registry();
    taint::begin_turn(); // a turn nobody started: no person_turn()
    for name in MAIL_CHANGES.iter().chain(&CALENDAR_CHANGES).chain(&MESSAGES) {
        let out = run(&reg, name, json!({}));
        assert!(out.starts_with("Refused:") && out.contains(&format!("`{name}`")), "{name}: {out}");
        assert!(out.contains("nobody is here") && out.contains("Nothing was changed"), "{out}");
    }
    taint::end_turn();
    // And with no turn at all, as recipe steps and urges run.
    for name in MAIL_CHANGES.iter().chain(&CALENDAR_CHANGES).chain(&MESSAGES) {
        assert!(run(&reg, name, json!({})).starts_with("Refused:"), "{name}");
    }
}

#[test]
fn file_writes_and_memory_changes_stay_allowed_for_recipes() {
    let reg = registry();
    let existing = std::env::temp_dir().join(format!("yantrik-changes-{}-recipe.txt", std::process::id()));
    std::fs::write(&existing, "old").expect("write");
    for _ in 0..2 {
        for name in FORGETS {
            assert!(ran(&run(&reg, name, json!({})), name), "{name}");
        }
        assert!(ran(&run(&reg, "write_file", json!({"path": existing.to_string_lossy(), "content": "n"})), "write_file"));
        assert!(ran(&run(&reg, "remember", json!({})), "remember"));
        taint::begin_turn(); // the second pass is inside a turn that read nothing
    }
    taint::end_turn();
}

#[test]
fn an_unattended_turn_that_read_mail_may_not_plan_either() {
    let reg = registry();
    taint::begin_turn();
    assert!(ran(&run(&reg, "email_read", json!({})), "email_read"));
    for name in PLANS {
        assert_refused_after(&run(&reg, name, json!({})), name, "email_read");
    }
    taint::end_turn();
}

#[test]
fn a_new_turn_forgets_what_was_read() {
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
fn mail_reads_taint_the_other_rules_too() {
    let reg = registry();
    let mut r = ToolRegistry::new();
    r.register(Box::new(Fake("vault_get", "vault")));
    r.register(Box::new(Fake("email_read", "email")));
    drop(reg);
    taint::begin_turn();
    assert!(ran(&run(&r, "email_read", json!({})), "email_read"));
    let out = run(&r, "vault_get", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("email_read"), "{out}");
    taint::end_turn();
}
