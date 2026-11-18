//! The rule of `changes` (nobody there), and what `taint` counts as untrusted for the secrets rule, driven through
//! the real `ToolRegistry::execute` with stand-in tools that claim a name and a category.

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
const FETCHES: [&str; 3] = ["web_fetch", "http_fetch", "clipboard_fetch_url"];
const MAIL_CHANGES: [&str; 3] = ["email_delete", "email_move", "email_archive"];
const CALENDAR_CHANGES: [&str; 2] = ["calendar_delete_event", "calendar_update_event"];
const MESSAGES: [&str; 2] = ["whatsapp_send", "telegram_send"];

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    for (name, category) in [
        ("browse", "browser"),
        ("read_file", "files"),
        ("grep", "files"),
        ("vault_get", "vault"),
        ("run_command", "system"),
        ("web_fetch", "network"),
        ("http_fetch", "network"),
        ("clipboard_fetch_url", "clipboard"),
        ("email_send", "email"),
        ("email_reply", "email"),
        ("email_forward", "email"),
        ("email_reply_all", "email"),
        ("create_schedule", "scheduler"),
        ("write_file", "files"),
        ("forget_memory", "memory"),
        ("read_clipboard", "clipboard"),
        ("clipboard_history", "clipboard"),
        ("clipboard_analyze", "clipboard"),
    ] {
        reg.register(Box::new(Fake(name, category)));
    }
    for name in READS.iter().chain(&MAIL_CHANGES).chain(&CALENDAR_CHANGES).chain(&MESSAGES) {
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
    MAIL_CHANGES.iter().chain(&CALENDAR_CHANGES).chain(&MESSAGES).copied().collect()
}

// ── Nobody there ──

#[test]
fn with_nobody_there_the_mail_and_calendar_changes_and_the_two_sends_are_refused() {
    let reg = registry();
    // A turn nobody started, and no turn at all (recipe steps, urges).
    for with_a_turn in [true, false] {
        if with_a_turn {
            taint::begin_turn();
        } else {
            taint::end_turn();
        }
        for name in held() {
            let out = run(&reg, name, json!({}));
            assert!(out.starts_with("Refused:") && out.contains(&format!("`{name}`")), "{name}: {out}");
            assert!(out.contains("nobody is here") && out.contains("Nothing was changed") && out.contains("in the app"), "{out}");
        }
    }
    taint::end_turn();
}

#[test]
fn the_persons_own_turn_is_not_held_back_by_this_rule() {
    let reg = registry();
    taint::begin_turn();
    let _person = person_turn();
    for name in held() {
        assert!(ran(&run(&reg, name, json!({})), name), "{name} was refused in the person's own turn");
    }
    // Even after a read: that is the card's job (#1313), not this rule's.
    assert!(ran(&run(&reg, "email_read", json!({})), "email_read"));
    for name in held() {
        assert!(ran(&run(&reg, name, json!({})), name), "{name} after a read");
    }
    taint::end_turn();
}

#[test]
fn file_writes_memory_changes_and_plans_are_not_this_rules_and_mail_sends_stay_behind_outward() {
    let reg = registry();
    taint::begin_turn();
    for name in ["write_file", "forget_memory", "create_schedule", "email_send", "email_reply", "email_reply_all", "email_forward"] {
        assert!(!super::is_change(name), "{name}");
        assert!(ran(&run(&reg, name, json!({"path": "/tmp/x"})), name), "{name} with nobody there");
    }
    taint::end_turn();
}

#[test]
fn the_named_app_actions_are_refused_with_nobody_there_and_no_others() {
    use crate::changes::check_app_action;
    taint::begin_turn();
    for (app, action) in [("calendar", "delete_event"), ("app-calendar", "update_event"), ("shell", "files_delete"), ("files", "files_delete")] {
        let said = check_app_action(app, action).expect_err("nobody is here");
        assert!(said.contains("nobody is here") && said.contains("Nothing was changed"), "{said}");
    }
    for (app, action) in [("notes", "purge"), ("calendar", "list"), ("files", "files_open"), ("terminal", "run")] {
        assert!(check_app_action(app, action).is_ok(), "{app}.{action} is not named");
    }
    taint::end_turn();
    let _person = person_turn();
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event"), ("shell", "files_delete")] {
        assert!(check_app_action(app, action).is_ok(), "{app}.{action} in the person's turn");
    }
}

#[test]
fn the_app_action_refusal_reads_whole() {
    taint::begin_turn();
    let said = crate::changes::check_app_action("calendar", "delete_event").expect_err("nobody is here");
    assert!(said.starts_with("Refused: `app_action` (`calendar.delete_event`) changes or deletes a calendar event, and nobody is here to ask"), "{said}");
    taint::end_turn();
}

// ── Untrusted content, for the secrets rule ──

#[test]
fn the_clipboards_history_and_analysis_are_sources_like_its_reading() {
    let reg = registry();
    for reader in ["read_clipboard", "clipboard_history", "clipboard_analyze"] {
        taint::begin_turn();
        assert!(ran(&run(&reg, reader, json!({})), reader));
        let out = run(&reg, "vault_get", json!({}));
        assert!(out.starts_with("Refused:") && out.contains(&format!("`{reader}`")), "{reader}: {out}");
        taint::end_turn();
    }
}

#[test]
fn mail_and_calendar_reads_are_sources_for_the_credential_rules() {
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
fn a_fetched_page_then_a_credential_is_refused() {
    let reg = registry();
    let _person = person_turn(); // a fetch or a page is reached only in a turn the person started
    for fetch in FETCHES {
        taint::begin_turn();
        assert!(ran(&run(&reg, fetch, json!({})), fetch));
        let out = run(&reg, "vault_get", json!({}));
        assert!(out.starts_with("Refused:") && out.contains(&format!("`{fetch}`")) && out.contains("returns a credential"), "{fetch}: {out}");
        taint::end_turn();
    }
}

#[test]
fn a_secret_then_a_fetch_is_still_refused_when_a_page_was_read_too() {
    // The fetch is both: it carries something out, and it brings a page back. A secret, a page, then a fetch is the trifecta.
    let reg = registry();
    let _person = person_turn(); // a fetch or a page is reached only in a turn the person started
    for fetch in FETCHES {
        taint::begin_turn();
        assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"));
        assert!(ran(&run(&reg, "browse", json!({})), "browse"));
        let out = run(&reg, fetch, json!({}));
        assert!(out.starts_with("Refused:") && out.contains("can send data outward"), "{fetch}: {out}");
        taint::end_turn();
    }
    // A secret then a fetch alone: the fetch brings the page back, so the turn is now untrusted, and the next outward call is refused.
    taint::begin_turn();
    assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"));
    assert!(ran(&run(&reg, "web_fetch", json!({})), "web_fetch"), "the fetch itself ran: only the credential had been read");
    let out = run(&reg, "http_fetch", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("`vault_get`") && out.contains("`web_fetch`"), "{out}");
    taint::end_turn();
}

#[test]
fn a_secret_a_page_then_clipboard_fetch_url_is_refused_whatever_its_category() {
    // `clipboard_fetch_url` is category `clipboard`, so `classify` never called it outward; a fetch can carry a secret out in its address.
    let reg = registry();
    let _person = person_turn();
    taint::begin_turn();
    assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"));
    assert!(ran(&run(&reg, "browse", json!({})), "browse"));
    let out = run(&reg, "clipboard_fetch_url", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("can send data outward") && out.contains("`vault_get`") && out.contains("`browse`"), "{out}");
    taint::end_turn();
    // Without the secret, or without the page, it runs.
    taint::begin_turn();
    assert!(ran(&run(&reg, "browse", json!({})), "browse"));
    assert!(ran(&run(&reg, "clipboard_fetch_url", json!({})), "clipboard_fetch_url"));
    taint::end_turn();
}

#[test]
fn a_fetch_does_not_stop_being_outward() {
    let reg = registry();
    let _person = person_turn(); // a fetch or a page is reached only in a turn the person started
    taint::begin_turn();
    assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"));
    assert!(ran(&run(&reg, "browse", json!({})), "browse"));
    for fetch in FETCHES {
        assert!(run(&reg, fetch, json!({})).contains("can send data outward"), "{fetch}");
    }
    assert!(run(&reg, "run_command", json!({})).contains("can send data outward"));
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
fn what_a_sub_agent_read_is_merged_into_its_parent_for_the_secrets_rule() {
    let reg = std::sync::Arc::new(registry());
    taint::begin_turn();
    let _person = person_turn();
    let carried = (taint::carry(), crate::background_research::carry());
    let child = reg.clone();
    let read = std::thread::spawn(move || {
        taint::adopt(carried.0);
        let _p = crate::background_research::adopt(carried.1);
        assert!(ran(&run(&child, "web_fetch", json!({})), "web_fetch"));
        taint::state().0
    })
    .join()
    .expect("the sub-agent's thread");
    assert_eq!(read.as_deref(), Some("web_fetch"));
    assert!(ran(&run(&reg, "vault_get", json!({})), "vault_get"), "the parent has not taken it yet");
    taint::end_turn();
    // The merge, as `sub_agent` does it: the parent notes what the child read.
    taint::begin_turn();
    taint::note(read.as_deref().unwrap_or_default(), "browser");
    let out = run(&reg, "vault_get", json!({}));
    assert!(out.starts_with("Refused:") && out.contains("web_fetch"), "{out}");
    taint::end_turn();
}

#[test]
fn a_program_on_the_socket_leaves_the_persons_turn_as_it_found_it() {
    taint::begin_turn();
    let _person = person_turn();
    taint::note("browse", "browser");
    {
        let _aside = crate::changes::aside();
        assert_eq!(taint::state().0, None, "the outside turn starts with nothing");
        taint::mark_outside("a program on the companion's socket");
    }
    assert_eq!(taint::state().0.as_deref(), Some("browse"), "the person's turn was changed by the outside one");
    taint::end_turn();
}
