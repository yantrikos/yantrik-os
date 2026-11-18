//! The rules of `changes`, driven through the real `ToolRegistry::execute` with stand-in tools
//! that claim a name and a category, which is all the policy reads.

use crate::background_research::person_turn;
use crate::changes::CLEAN;
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
    assert!(out.contains("in the app") && out.contains("start a new conversation"), "it must say what works: {out}");
    assert!(!out.contains("next message"), "it must not promise what is no longer true: {out}");
    assert!(out.contains("this conversation has already read"), "{out}");
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
    crate::changes::forget_conversation();
    }
}

#[test]
fn a_clean_turn_still_deletes_sends_and_plans() {
    let reg = registry();
    for name in held() {
        taint::begin_turn();
        let _person = person_turn();
        assert!(ran(&run(&reg, name, json!({})), name), "{name} was refused in a clean turn");
        taint::end_turn();
        crate::changes::forget_conversation();
    }
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
    crate::changes::forget_conversation();
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
    taint::end_turn();
    crate::changes::forget_conversation();
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
    crate::changes::forget_conversation();
        }
        for name in &nobody {
            let out = run(&reg, name, json!({}));
            assert!(out.starts_with("Refused:") && out.contains(&format!("`{name}`")), "{name}: {out}");
            assert!(out.contains("nobody is here") && out.contains("Nothing was changed"), "{out}");
        }
    }
    taint::end_turn();
    crate::changes::forget_conversation();
}

#[test]
fn planning_unattended_is_allowed_until_the_turn_reads_something() {
    let reg = registry();
    taint::begin_turn();
    for name in &PLANS[..3] {
        assert!(ran(&run(&reg, name, json!({})), name), "{name}");
    }
    assert!(ran(&run(&reg, "email_read", json!({})), "email_read"));
    for name in PLANS {
        assert_refused_after(&run(&reg, name, json!({})), name, "email_read");
    }
    taint::end_turn();
    crate::changes::forget_conversation();
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
    crate::changes::forget_conversation();
    }
}

/// Tools of kinds the narrow list never named: each must mark the message, so the change rules do not depend
/// on someone having named it. (The machine test, 17 Nov: `web_fetch` then `create_schedule` ran.)
fn registry_with_unknowns() -> ToolRegistry {
    let mut reg = registry();
    for (name, category) in [
        ("web_fetch", "network"),
        ("http_fetch", "network"),
        ("clipboard_fetch_url", "clipboard"),
        ("run_command", "desktop"),
        ("git_log", "git"),
        ("a_tool_of_a_kind_nobody_has_seen", "never-heard-of-it"),
        ("calculate", "calculator"),
        ("unit_convert", "calculator"),
        ("date_calc", "time"),
    ] {
        reg.register(Box::new(Fake(name, category)));
    }
    reg
}

#[test]
fn a_fetch_a_command_or_an_unknown_tool_then_a_change_is_refused() {
    let reg = registry_with_unknowns();
    for (reader, change) in [
        ("web_fetch", "create_schedule"),
        ("http_fetch", "calendar_delete_event"),
        ("run_command", "email_delete"),
        ("a_tool_of_a_kind_nobody_has_seen", "create_recipe"),
        ("clipboard_fetch_url", "telegram_send"),
        ("git_log", "create_automation"),
    ] {
        taint::begin_turn();
        let _person = person_turn();
        assert!(ran(&run(&reg, reader, json!({})), reader));
        assert_refused_after(&run(&reg, change, json!({})), change, reader);
        taint::end_turn();
    crate::changes::forget_conversation();
    }
}

#[test]
fn a_calculation_or_the_clock_then_a_change_is_allowed() {
    let reg = registry_with_unknowns();
    for name in held() {
        taint::begin_turn();
        let _person = person_turn();
        for (clean, _) in CLEAN {
            let _ = run(&reg, clean, json!({}));
        }
        assert!(ran(&run(&reg, name, json!({})), name), "{name} was refused after only arithmetic and the clock");
        taint::end_turn();
        crate::changes::forget_conversation();
    }
}

#[test]
fn the_clean_list_is_pinned_to_exact_name_and_category_pairs() {
    assert_eq!(
        CLEAN,
        [
            ("calculate", "calculator"),
            ("unit_convert", "calculator"),
            ("base64_encode", "encoding"),
            ("base64_decode", "encoding"),
            ("url_encode", "encoding"),
            ("json_format", "encoding"),
            ("date_calc", "time"),
        ]
    );
    for (name, category) in CLEAN {
        assert!(!super::marks(name, category), "{name}");
        assert!(super::marks(name, "plugin"), "a plugin that takes the name {name} still marks");
    }
    // word_count counts a file's words too (round 5, M3).
    for (name, category) in [("word_count", "text"), ("read_file", "files"), ("grep", "files"), ("hash_file", "text"), ("diff_files", "text"),
                             ("timer", "time"), ("recall", "memory"), ("get_weather", "weather"), ("email_send", "email"),
                             ("run_recipe", "automation"), ("app_action", "app"), ("web_fetch", "network")] {
        assert!(super::marks(name, category), "{name} reads something: it must mark the message");
    }
}

#[test]
fn a_guarded_tool_that_echoes_what_it_found_marks_and_one_that_echoes_its_arguments_does_not() {
    let reg = registry();
    for marking in ["email_delete", "email_move", "email_archive", "calendar_delete_event", "calendar_update_event"] {
        taint::begin_turn();
        let _person = person_turn();
        assert!(ran(&run(&reg, marking, json!({})), marking));
        assert_refused_after(&run(&reg, "create_schedule", json!({})), "create_schedule", marking);
        taint::end_turn();
        crate::changes::forget_conversation();
    }
    for unmarking in ["create_recipe", "create_schedule", "create_automation", "whatsapp_send", "telegram_send"] {
        taint::begin_turn();
        let _person = person_turn();
        assert!(ran(&run(&reg, unmarking, json!({})), unmarking));
        assert!(ran(&run(&reg, "email_delete", json!({})), "email_delete"), "{unmarking} must not mark the message");
        taint::end_turn();
        crate::changes::forget_conversation();
    }
}

#[test]
fn what_one_message_read_is_still_read_in_the_next_message_of_the_conversation_until_it_is_cleared() {
    let reg = registry_with_unknowns();
    taint::begin_turn();
    let _person = person_turn();
    assert!(ran(&run(&reg, "web_fetch", json!({})), "web_fetch"));
    taint::begin_turn(); // "ok, do what it says": no tool has run in this message
    let out = run(&reg, "create_schedule", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("an earlier message in this conversation, with `web_fetch`"), "{out}");
    assert!(out.contains("Nothing was changed") && out.contains("start a new conversation"), "{out}");
    // A new conversation: the history is cleared, and the rule lets go.
    crate::changes::forget_conversation();
    taint::begin_turn();
    assert!(ran(&run(&reg, "create_schedule", json!({})), "create_schedule"));
    taint::end_turn();
    crate::changes::forget_conversation();
}

#[test]
fn a_turn_nobody_started_neither_inherits_nor_leaves_a_conversation_mark() {
    let reg = registry_with_unknowns();
    taint::begin_turn();
    {
        let _person = person_turn();
        assert!(ran(&run(&reg, "web_fetch", json!({})), "web_fetch"));
    }
    // An unattended turn on the same thread: its own turn, not the conversation's.
    taint::begin_turn();
    assert!(ran(&run(&reg, "create_schedule", json!({})), "create_schedule"));
    assert!(ran(&run(&reg, "git_log", json!({})), "git_log"));
    taint::end_turn();
    crate::changes::forget_conversation();
    // And what it read does not become the person's conversation.
    taint::begin_turn();
    assert!(ran(&run(&reg, "git_log", json!({})), "git_log"));
    taint::begin_turn();
    let _person = person_turn();
    assert!(ran(&run(&reg, "create_schedule", json!({})), "create_schedule"));
    taint::end_turn();
    crate::changes::forget_conversation();
}

#[test]
fn the_companions_own_notice_put_in_front_of_the_persons_words_marks_the_turn() {
    let reg = registry();
    let _ = crate::changes::notice_in_context("[Context: you recently sent ...]".to_string());
    let _person = person_turn();
    taint::begin_turn();
    let out = run(&reg, "create_schedule", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("the companion's own earlier notice"), "{out}");
    taint::end_turn();
    crate::changes::forget_conversation();
    // Only the turn it was given to: the next one is clean.
    taint::begin_turn();
    assert!(ran(&run(&reg, "create_schedule", json!({})), "create_schedule"));
    taint::end_turn();
}

#[test]
fn the_broad_mark_is_carried_to_a_thread_that_works_for_the_turn() {
    // Mutation: `adopt` that drops `broad_from` leaves the child with no mark.
    taint::begin_turn();
    let _person = person_turn();
    crate::changes::mark("web_fetch");
    let carried = taint::carry();
    let seen = std::thread::spawn(move || {
        taint::adopt(carried);
        crate::changes::broad()
    })
    .join()
    .expect("the thread");
    assert_eq!(seen.as_deref(), Some("web_fetch"));
    taint::end_turn();
    crate::changes::forget_conversation();
}

#[test]
fn a_program_on_the_socket_leaves_the_broad_mark_as_it_found_it() {
    // Mutation: an outside turn that ends the person's turn (or marks it) changes `broad()`.
    taint::begin_turn();
    let _person = person_turn();
    crate::changes::mark("web_fetch");
    {
        let _aside = crate::changes::aside();
        taint::mark_outside("a program on the companion's socket");
        assert_eq!(crate::changes::broad(), None, "the outside turn starts with nothing");
        crate::changes::mark("a_tool_the_outside_ran");
    }
    assert_eq!(crate::changes::broad().as_deref(), Some("web_fetch"), "the person's turn was changed by the outside one");
    taint::end_turn();
    crate::changes::forget_conversation();
}

#[test]
fn the_narrow_merge_from_a_sub_agent_reaches_the_secrets_rule() {
    // Mutation: a merge that forgets the narrow read lets vault_get through after a sub-agent read mail.
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"), "nothing merged yet");
    taint::end_turn();
    taint::begin_turn();
    taint::note("email_read", "email"); // the narrow note a sub-agent's result is merged with
    let out = run(&reg, "vault_get", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("email_read"), "{out}");
    taint::end_turn();
    crate::changes::forget_conversation();
}

#[test]
fn after_a_sub_agents_broad_only_tool_the_refusal_names_that_tool_not_the_spawner() {
    struct Spawns;
    impl Tool for Spawns {
        fn name(&self) -> &'static str {
            "spawn_agents"
        }
        fn permission(&self) -> PermissionLevel {
            PermissionLevel::Standard
        }
        fn category(&self) -> &'static str {
            "delegation"
        }
        fn definition(&self) -> serde_json::Value {
            json!({})
        }
        fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
            // The sub-agent ran `web_fetch`; its result is merged before the parent uses it.
            crate::changes::mark("web_fetch");
            "spawned".into()
        }
    }
    let mut reg = registry();
    reg.register(Box::new(Spawns));
    taint::begin_turn();
    let _person = person_turn();
    assert_eq!(run(&reg, "spawn_agents", json!({})), "spawned");
    assert_refused_after(&run(&reg, "create_schedule", json!({})), "create_schedule", "web_fetch");
    taint::end_turn();
    crate::changes::forget_conversation();
}

#[test]
fn the_secrets_rule_is_not_widened_by_the_broad_mark() {
    let reg = registry_with_unknowns();
    taint::begin_turn();
    assert!(ran(&run(&reg, "git_log", json!({})), "git_log"));
    assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"), "the credential rule still reads only the narrow list");
    taint::end_turn();
    crate::changes::forget_conversation();
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
    crate::changes::forget_conversation();
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
        (taint::state().0, crate::changes::broad())
    })
    .join()
    .expect("the sub-agent's thread");
    assert_eq!(read.0.as_deref(), Some("email_read"));
    assert_eq!(read.1.as_deref(), Some("email_read"), "the broad mark came back too");
    assert!(ran(&run(&reg, "create_schedule", json!({})), "create_schedule"), "the parent has not taken it yet");
    taint::note(read.0.as_deref().unwrap_or_default(), "browser");
    crate::changes::mark(read.1.as_deref().unwrap_or_default());
    assert_refused_after(&run(&reg, "create_schedule", json!({})), "create_schedule", "email_read");
    taint::end_turn();
    crate::changes::forget_conversation();
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
    crate::changes::forget_conversation();
    // The person's turn: clean it runs; after a read the calendar ones are refused, file deletion waits for the card.
    taint::begin_turn();
    let _person = person_turn();
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event"), ("shell", "files_delete"), ("files", "files_delete")] {
        assert!(check_app_action(app, action).is_ok(), "{app}.{action} in a clean turn");
    }
    taint::note("email_read", "email");
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event")] {
        let said = check_app_action(app, action).expect_err("refused after a read");
        assert!(said.contains("`email_read`") && said.contains("Nothing was changed") && said.contains("new conversation"), "{said}");
    }
    assert!(check_app_action("shell", "files_delete").is_ok());
    for (app, action) in [("notes", "purge"), ("calendar", "list"), ("files", "files_open"), ("shell", "open_app"), ("terminal", "run")] {
        assert!(check_app_action(app, action).is_ok(), "{app}.{action} is not named");
    }
    taint::end_turn();
    crate::changes::forget_conversation();
}
