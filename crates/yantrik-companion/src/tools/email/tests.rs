//! The mail tools over a fake `email-service` (#1022): what they ask it, what they say back, what
//! grade each keeps, and that no mail client or mail secret is left in the companion.

use std::sync::Arc;

use rusqlite::Connection;
use yantrik_ipc_contracts::email::{EmailDetail, EmailFolder, ServiceError};

use super::{change, read, send, support};
use crate::email::fake::{account, message, FakeMail, Vault};
use crate::email::port::MailPort;
use crate::tools::{PermissionLevel, Tool, ToolRegistry};

fn world() -> (Arc<FakeMail>, Connection) {
    let mail = Arc::new(FakeMail::with_accounts(vec![account("ada-at-example.com", "ada@example.com")]));
    (mail, Connection::open_in_memory().unwrap())
}

fn port(mail: &Arc<FakeMail>) -> Arc<dyn MailPort> {
    mail.clone()
}

fn args(v: serde_json::Value) -> serde_json::Value {
    v
}

/// The mind's message number for uid `uid`, after a check has cached it.
fn cached_id(conn: &Connection, uid: u32) -> i64 {
    conn.query_row("SELECT id FROM emails WHERE uid = ?1", [uid], |r| r.get(0)).unwrap()
}

fn detail(from: &str, to: &[&str], cc: &[&str]) -> EmailDetail {
    EmailDetail {
        id: "7".into(),
        from: from.into(),
        to: to.iter().map(|s| s.to_string()).collect(),
        cc: cc.iter().map(|s| s.to_string()).collect(),
        bcc: Vec::new(),
        subject: "Plans".into(),
        body_html: String::new(),
        body_text: "Dinner at eight.".into(),
        date: "2026-10-30 09:15".into(),
        attachments: Vec::new(),
        thread_messages: Vec::new(),
        is_read: false,
        is_starred: false,
        message_id: "<m7@example.com>".into(),
    }
}

/// A checked inbox with one message, uid 7, from Priya.
fn checked() -> (Arc<FakeMail>, Connection, i64) {
    let (mail, conn) = world();
    mail.state.lock().unwrap().listed = vec![message(7, "Priya", "priya@example.com", "Plans")];
    let said = read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));
    assert!(said.starts_with("Synced 1 new emails"), "{said}");
    let id = cached_id(&conn, 7);
    (mail, conn, id)
}

// ── Reading goes to the service ───────────────────────────────────────────────────────────────

#[test]
fn a_check_lists_the_inbox_through_the_service_and_shows_the_line_the_pulse_reads() {
    let (mail, conn) = world();
    mail.state.lock().unwrap().listed = vec![message(7, "Priya", "priya@example.com", "Plans")];
    mail.state.lock().unwrap().folders = vec![EmailFolder::counted("INBOX", 1, 1)];

    let said = read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));

    assert!(said.contains("Synced 1 new emails for ada@example.com (ada@example.com). 1 unread of 1 in INBOX."), "{said}");
    assert!(said.contains("Priya | Plans | 2026-10-30 09:15") || said.contains("| Plans |"), "{said}");
    assert_eq!(mail.calls(), vec!["accounts", "list_messages ada-at-example.com INBOX", "list_folders ada-at-example.com"]);
}

/// 520 gate, 31 Oct: the companion said "41 unread in INBOX" while the Email app showed 60 of 78,
/// because it counted the 50 newest messages it had synced. A mailbox of 78 with 60 unseen, of
/// which only 50 are listed, is stated with the server's numbers.
#[test]
fn the_unread_count_is_the_mailboxs_not_the_synced_windows() {
    let (mail, conn) = world();
    {
        let mut state = mail.state.lock().unwrap();
        state.listed = (1..=50u32).map(|uid| message(uid, "Priya", "priya@example.com", "Hello")).collect();
        state.folders = vec![EmailFolder::counted("INBOX", 60, 78)];
    }

    let checked = read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));
    let listed = read::EmailListTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));

    for said in [&checked, &listed] {
        assert!(said.contains("60 unread of 78 in INBOX"), "{said}");
    }
}

#[test]
fn when_the_server_will_not_count_the_window_is_named_as_a_window() {
    let (mail, conn) = world();
    {
        let mut state = mail.state.lock().unwrap();
        state.listed = (1..=50u32).map(|uid| message(uid, "Priya", "priya@example.com", "Hello")).collect();
        state.folders = vec![EmailFolder::uncounted("INBOX", "the server did not answer")];
    }

    let said = read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));

    assert!(said.contains("unread among the newest 50 messages synced"), "{said}");
    assert!(!said.contains("unread in INBOX") && !said.contains(" of 78"), "{said}");
}

#[test]
fn a_second_check_keeps_the_numbers_the_mind_was_given() {
    let (mail, conn, id) = checked();
    read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));
    assert_eq!(cached_id(&conn, 7), id, "a listed message changed number on a re-check");
}

#[test]
fn reading_fetches_the_body_once_and_marks_it_read_on_the_server() {
    let (mail, conn, id) = checked();
    mail.state.lock().unwrap().detail = Some(detail("Priya <priya@example.com>", &[], &[]));
    let tool = read::EmailReadTool { mail: port(&mail) };

    let first = tool.run(&conn, &args(serde_json::json!({ "id": id })));
    let again = tool.run(&conn, &args(serde_json::json!({ "id": id })));

    assert!(first.contains("Dinner at eight.") && first.contains("From: Priya <priya@example.com>"), "{first}");
    assert_eq!(first, again);
    assert_eq!(mail.count("get_message"), 1, "the second read came from the cache");
    assert_eq!(mail.count("mark_read ada-at-example.com INBOX 7 true"), 1);
}

#[test]
fn search_goes_to_the_server_and_its_hits_get_numbers() {
    let (mail, conn) = world();
    mail.state.lock().unwrap().found = vec![message(9, "Priya", "priya@example.com", "Dinner plans")];

    let said = read::EmailSearchTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "query": "dinner" })));

    assert!(said.contains("Search 'dinner'") && said.contains("Dinner plans"), "{said}");
    assert_eq!(mail.count("search ada-at-example.com INBOX dinner"), 1);
}

#[test]
fn folders_come_from_the_service() {
    let (mail, conn) = world();
    mail.state.lock().unwrap().folders = vec![EmailFolder::counted("INBOX", 3, 40), EmailFolder::uncounted("Archive", "no")];
    let said = read::EmailListFoldersTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));
    assert!(said.contains("  - INBOX (3 unread)") && said.contains("  - Archive\n"), "{said}");
}

#[test]
fn a_shut_vault_is_said_in_the_services_own_words_and_nothing_is_cached() {
    let (mail, conn) = world();
    mail.state.lock().unwrap().listed = vec![message(7, "Priya", "priya@example.com", "Plans")];
    mail.set_vault(Vault::Shut);

    let said = read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));

    assert_eq!(said, "Fetch failed: Mail is waiting for the vault. Unlock it and mail will check again.");
    let cached: i64 = conn.query_row("SELECT COUNT(*) FROM emails", [], |r| r.get(0)).unwrap();
    assert_eq!(cached, 0);
}

#[test]
fn with_no_account_in_the_service_the_tools_say_so() {
    let conn = Connection::open_in_memory().unwrap();
    let mail: Arc<dyn MailPort> = Arc::new(FakeMail::empty());
    let said = read::EmailCheckTool { mail }.run(&conn, &args(serde_json::json!({})));
    assert_eq!(said, "No email accounts configured");
}

#[test]
fn an_account_is_picked_by_name_or_address_and_an_unknown_one_is_refused() {
    let both = vec![account("a", "ada@example.com"), account("b", "bob@work.example")];
    assert_eq!(support::pick(&both, Some("work")).unwrap().id, "b");
    assert_eq!(support::pick(&both, Some("1")).unwrap().id, "b");
    assert_eq!(support::pick(&both, None).unwrap().id, "a");
    let refused = support::pick(&both, Some("nobody")).unwrap_err();
    assert!(refused.contains("not found") && refused.contains("ada@example.com"), "{refused}");
}

// ── Sending goes to the service, and says so ─────────────────────────────────────────────────

#[test]
fn a_send_is_one_message_handed_to_the_service_from_the_named_account() {
    let (mail, conn) = world();

    let said = send::EmailSendTool { mail: port(&mail) }.run(
        &conn,
        &args(serde_json::json!({ "to": "priya@example.com", "subject": "Hi", "body": "Hello" })),
    );

    assert_eq!(said, "Email sent to priya@example.com from ada@example.com — subject: \"Hi\"");
    let sent = mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "ada-at-example.com");
    assert_eq!(sent[0].1.to, vec!["priya@example.com"]);
    assert_eq!((sent[0].1.subject.as_str(), sent[0].1.body.as_str()), ("Hi", "Hello"));
}

#[test]
fn a_send_the_service_refuses_is_reported_as_not_sent() {
    let (mail, conn) = world();
    mail.set_vault(Vault::Shut);
    let said = send::EmailSendTool { mail: port(&mail) }
        .run(&conn, &args(serde_json::json!({ "to": "p@example.com", "subject": "Hi", "body": "x" })));
    assert!(said.starts_with("Send failed: Mail is waiting for the vault"), "{said}");
    assert!(mail.sent().is_empty());
}

#[test]
fn a_reply_goes_to_the_sender_under_the_message_it_answers() {
    let (mail, conn, id) = checked();

    let said = send::EmailReplyTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "id": id, "body": "Yes!" })));

    assert_eq!(said, "Reply sent to priya@example.com — subject: \"Re: Plans\"");
    let (_, compose) = mail.sent().remove(0);
    assert_eq!(compose.to, vec!["priya@example.com"]);
    assert_eq!(compose.reply_to_id.as_deref(), Some("<m7@example.com>"));
    assert_eq!(compose.subject, "Re: Plans");
}

#[test]
fn a_reply_all_is_one_message_to_the_sender_with_the_others_copied_and_never_ourselves() {
    let (mail, conn, id) = checked();
    mail.state.lock().unwrap().detail = Some(detail(
        "Priya <priya@example.com>",
        &["Ada <ada@example.com>", "bob@example.com"],
        &["Cy <cy@example.com>", "BOB@example.com"],
    ));

    let said = send::EmailReplyAllTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "id": id, "body": "All good" })));

    assert!(said.starts_with("Reply-all sent to priya@example.com, bob@example.com, cy@example.com"), "{said}");
    let sent = mail.sent();
    assert_eq!(sent.len(), 1, "one message, not one per person");
    assert_eq!(sent[0].1.to, vec!["priya@example.com"]);
    assert_eq!(sent[0].1.cc, vec!["bob@example.com", "cy@example.com"]);
}

#[test]
fn a_forward_carries_the_original_and_goes_where_it_is_told() {
    let (mail, conn, id) = checked();
    mail.state.lock().unwrap().detail = Some(detail("Priya <priya@example.com>", &[], &[]));

    let said = send::EmailForwardTool { mail: port(&mail) }
        .run(&conn, &args(serde_json::json!({ "id": id, "to": "dan@example.com", "comment": "FYI" })));

    assert!(said.starts_with("Forwarded email"), "{said}");
    let (_, compose) = mail.sent().remove(0);
    assert_eq!(compose.to, vec!["dan@example.com"]);
    assert_eq!(compose.subject, "Fwd: Plans");
    assert!(compose.body.starts_with("FYI") && compose.body.contains("Dinner at eight."), "{}", compose.body);
}

// ── Changing mail goes to the service, in the message's own folder ──────────────────────────

#[test]
fn marking_everything_read_is_one_call_for_all_the_unread() {
    let (mail, conn) = world();
    mail.state.lock().unwrap().listed = vec![
        message(1, "A", "a@example.com", "one"),
        message(2, "B", "b@example.com", "two"),
    ];
    read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({})));

    let said = change::EmailMarkReadTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "all": true })));

    assert!(said.starts_with("Marked 2 emails as read in INBOX"), "{said}");
    assert_eq!(mail.count("mark_read ada-at-example.com INBOX"), 1);
    assert!(mail.calls().iter().any(|c| c.ends_with("1,2 true") || c.ends_with("2,1 true")), "{:?}", mail.calls());
}

#[test]
fn flag_unflag_unread_delete_move_and_archive_each_ask_the_service_once() {
    let (mail, conn, id) = checked();
    let a = |v: serde_json::Value| args(v);

    change::EmailFlagTool { mail: port(&mail) }.run(&conn, &a(serde_json::json!({ "id": id })));
    change::EmailUnflagTool { mail: port(&mail) }.run(&conn, &a(serde_json::json!({ "id": id })));
    change::EmailMarkUnreadTool { mail: port(&mail) }.run(&conn, &a(serde_json::json!({ "id": id })));
    let moved = change::EmailMoveTool { mail: port(&mail) }.run(&conn, &a(serde_json::json!({ "id": id, "folder": "Trash" })));
    assert!(moved.starts_with("Moved email"), "{moved}");

    for expected in [
        "mark_starred ada-at-example.com INBOX 7 true",
        "mark_starred ada-at-example.com INBOX 7 false",
        "mark_read ada-at-example.com INBOX 7 false",
        "move_message ada-at-example.com INBOX 7 Trash",
    ] {
        assert_eq!(mail.count(expected), 1, "{expected} in {:?}", mail.calls());
    }
}

#[test]
fn delete_and_archive_use_the_message_folder_and_the_providers_archive() {
    let (mail, conn, id) = checked();
    let archived = change::EmailArchiveTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "id": id })));
    assert!(archived.contains("[Gmail]/All Mail"), "{archived}");
    assert_eq!(mail.count("move_message ada-at-example.com INBOX 7 [Gmail]/All Mail"), 1);

    let (mail, conn, id) = checked();
    let deleted = change::EmailDeleteTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "id": id })));
    assert!(deleted.starts_with("Deleted email"), "{deleted}");
    assert_eq!(mail.count("delete_message ada-at-example.com INBOX 7"), 1);
    let left: i64 = conn.query_row("SELECT COUNT(*) FROM emails", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0, "a deleted message leaves the cache");
}

#[test]
fn a_refused_change_is_reported_and_the_cache_is_left_alone() {
    let (mail, conn, id) = checked();
    mail.state.lock().unwrap().refuse = Some(ServiceError { code: -32000, message: "IMAP STORE failed".into() });
    let said = change::EmailDeleteTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "id": id })));
    assert_eq!(said, "Failed: IMAP STORE failed");
    let left: i64 = conn.query_row("SELECT COUNT(*) FROM emails", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 1);
}

// ── Grades are what they were ─────────────────────────────────────────────────────────────────

/// The permission each mail tool had before the move to the service. A send, a reply, a forward or
/// any change stays `Standard` and so keeps its approval; delete stays `Sensitive`; reading stays
/// `Safe`. The approval and policy lists key on the name a tool is dispatched under
/// (`Tool::name()`), which is unchanged and pinned below.
const GRADES: &[(&str, PermissionLevel)] = &[
    ("email_check", PermissionLevel::Safe),
    ("email_list", PermissionLevel::Safe),
    ("email_read", PermissionLevel::Safe),
    ("email_search", PermissionLevel::Safe),
    ("email_list_folders", PermissionLevel::Safe),
    ("email_send", PermissionLevel::Standard),
    ("email_reply", PermissionLevel::Standard),
    ("email_reply_all", PermissionLevel::Standard),
    ("email_forward", PermissionLevel::Standard),
    ("email_mark_read", PermissionLevel::Standard),
    ("email_mark_unread", PermissionLevel::Standard),
    ("email_flag", PermissionLevel::Standard),
    ("email_unflag", PermissionLevel::Standard),
    ("email_move", PermissionLevel::Standard),
    ("email_archive", PermissionLevel::Standard),
    ("email_delete", PermissionLevel::Sensitive),
];

fn names_at(reg: &ToolRegistry, level: PermissionLevel) -> Vec<String> {
    reg.definitions(level)
        .iter()
        .filter_map(|d| d["function"]["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn every_mail_tool_keeps_the_grade_it_had() {
    let mut reg = ToolRegistry::new();
    super::register_with(&mut reg, Arc::new(FakeMail::empty()));

    assert_eq!(names_at(&reg, PermissionLevel::Dangerous).len(), GRADES.len(), "a tool was added or lost");
    for (name, grade) in GRADES {
        let appears_at = |level| names_at(&reg, level).iter().any(|n| n == name);
        let lowest = [PermissionLevel::Safe, PermissionLevel::Standard, PermissionLevel::Sensitive]
            .into_iter()
            .find(|l| appears_at(*l));
        assert_eq!(lowest, Some(*grade), "{name} changed grade");
    }
}

#[test]
fn each_tool_reports_the_name_and_grade_the_policy_lists_key_on() {
    let m = || -> Arc<dyn MailPort> { Arc::new(FakeMail::empty()) };
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(read::EmailCheckTool { mail: m() }),
        Box::new(read::EmailListTool { mail: m() }),
        Box::new(read::EmailReadTool { mail: m() }),
        Box::new(read::EmailSearchTool { mail: m() }),
        Box::new(read::EmailListFoldersTool { mail: m() }),
        Box::new(send::EmailSendTool { mail: m() }),
        Box::new(send::EmailReplyTool { mail: m() }),
        Box::new(send::EmailReplyAllTool { mail: m() }),
        Box::new(send::EmailForwardTool { mail: m() }),
        Box::new(change::EmailMarkReadTool { mail: m() }),
        Box::new(change::EmailMarkUnreadTool { mail: m() }),
        Box::new(change::EmailFlagTool { mail: m() }),
        Box::new(change::EmailUnflagTool { mail: m() }),
        Box::new(change::EmailMoveTool { mail: m() }),
        Box::new(change::EmailArchiveTool { mail: m() }),
        Box::new(change::EmailDeleteTool { mail: m() }),
    ];
    let have: Vec<(&str, PermissionLevel)> = tools.iter().map(|t| (t.name(), t.permission())).collect();
    assert_eq!(have, GRADES.to_vec());
    for t in &tools {
        assert_eq!(t.definition()["function"]["name"], t.name(), "the definition and the dispatch name differ");
    }
}

#[test]
fn a_line_break_in_what_a_model_chose_never_reaches_the_service() {
    let (mail, conn, id) = checked();
    let before = mail.calls().len();
    let hostile = "x\r\nZ1 UID STORE 1:* +FLAGS.SILENT (\\Deleted)\r\nZ2 EXPUNGE";

    let said = [
        read::EmailSearchTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "query": hostile }))),
        read::EmailSearchTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "query": "ok", "folder": hostile }))),
        read::EmailCheckTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "folder": hostile }))),
        read::EmailListFoldersTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "account": hostile }))),
        change::EmailMoveTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "id": id, "folder": hostile }))),
        change::EmailMarkReadTool { mail: port(&mail) }.run(&conn, &args(serde_json::json!({ "all": true, "folder": hostile }))),
    ];

    for answer in &said {
        assert!(answer.starts_with("Error: '") && answer.contains("control character"), "{answer}");
        assert!(!answer.contains("EXPUNGE"), "the hostile text was echoed: {answer}");
    }
    assert_eq!(mail.calls().len(), before, "a call reached the service: {:?}", mail.calls());
}

#[test]
fn the_tools_still_answer_to_their_old_names_and_the_category_email() {
    let mut reg = ToolRegistry::new();
    super::register_with(&mut reg, Arc::new(FakeMail::empty()));
    let mut have = names_at(&reg, PermissionLevel::Dangerous);
    have.sort();
    let mut want: Vec<String> = GRADES.iter().map(|(n, _)| n.to_string()).collect();
    want.sort();
    assert_eq!(have, want);
    let tool = send::EmailSendTool { mail: Arc::new(FakeMail::empty()) };
    assert_eq!(tool.category(), "email");
}

// ── One stack: no client, no mail secret in the companion ─────────────────────────────────────

fn companion_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The text of every `.rs` file under `dir`, with its path.
fn sources(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((path.display().to_string(), std::fs::read_to_string(&path).unwrap_or_default()));
        }
    }
}

/// The source with its test modules cut off, so a pin does not read itself.
fn production(text: &str) -> &str {
    text.split("#[cfg(test)]
mod tests {").next().unwrap_or(text)
}

#[test]
fn no_mail_client_is_left() {
    let root = companion_root();
    assert!(!root.join("src/email/imap_client.rs").exists());
    assert!(!root.join("src/email/smtp_client.rs").exists());

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for dep in ["imap =", "lettre", "native-tls", "yantrik-email-text"] {
        assert!(!manifest.contains(dep), "the companion's manifest still lists {dep}");
    }

    let mut all = Vec::new();
    sources(&root.join("src"), &mut all);
    for (path, text) in &all {
        // This file spells the words it forbids.
        if path.ends_with("tools/email/tests.rs") {
            continue;
        }
        for needle in ["imap::", "imap_client", "smtp_client", "lettre", "native_tls", "SmtpTransport"] {
            assert!(!text.contains(needle), "{path} still mentions `{needle}`");
        }
    }
}

#[test]
fn the_config_holds_no_mail_password_or_token_and_no_account_list() {
    let mut all = Vec::new();
    sources(&companion_root().join("../yantrik-companion-core/src"), &mut all);
    let config = all.iter().find(|(p, _)| p.ends_with("/config.rs") || p.ends_with("\\config.rs")).expect("config.rs").1.as_str();
    let section = &config[config.find("// ── Email Config").unwrap()..config.find("// ── Calendar Config").unwrap()];
    // The comment above the section says what used to be there; the code must not have it.
    let section: String = section.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("
");
    for needle in ["password", "oauth_access_token", "oauth_refresh_token", "EmailAccountConfig", "pub accounts"] {
        assert!(!section.contains(needle), "the email section of the config still has `{needle}`");
    }
}

#[test]
fn nothing_that_logs_or_prints_in_the_mail_code_names_a_secret() {
    let root = companion_root();
    let mut all = Vec::new();
    sources(&root.join("src/email"), &mut all);
    sources(&root.join("src/tools/email"), &mut all);
    sources(&root.join("src/calendar"), &mut all);
    let mut checked = 0;
    for (path, text) in &all {
        if path.ends_with("tests.rs") || path.ends_with("fake.rs") {
            continue;
        }
        for (n, line) in production(text).lines().enumerate() {
            let prints = line.contains("tracing::") || line.contains("println!") || line.contains("eprintln!") || line.contains("format!(\"{:?}\"");
            if !prints {
                continue;
            }
            checked += 1;
            let lower = line.to_lowercase();
            for word in ["password", "secret", "token", "credential"] {
                assert!(!lower.contains(word), "{path}:{} logs something that names `{word}`: {line}", n + 1);
            }
        }
    }
    assert!(checked > 0, "the pin saw no logging at all, so it is not looking");
}

#[test]
fn every_tracing_call_in_the_migration_is_a_fixed_sentence_or_counts() {
    let text = include_str!("../../email/legacy/mod.rs");
    let calls: Vec<&str> = production(text).lines().filter(|l| l.contains("tracing::")).collect();
    assert!(!calls.is_empty());
    for call in calls {
        assert!(!call.contains("{:?}") && !call.contains("{}") && !call.contains("%"), "a value is interpolated: {call}");
    }
}
