//! Tests of the shared ask and of the tools of this crate that use it (#1311): `write_file`
//! onto a path that exists, `edit_file` and `forget_memory`. The behaviours every tool must get
//! right come from [`crate::asks_scenarios::check_all`]; here are the ones specific to the ask
//! itself and to what these tools show.

use std::path::PathBuf;

use crate::asks::{quoted, Card, Question};
use crate::asks_scenarios::{check_all, Scenario};
use crate::fake_shell::{Answer, FakeShell, First};
use crate::{asks_files, asks_memory};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yantrik-asks-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn q(text: &str) -> serde_json::Value {
    quoted(text)
}

#[test]
fn the_four_questions_are_named_and_graded_as_the_shell_publishes_them() {
    let table: Vec<_> = Question::ALL.iter().map(|q| (q.action(), q.grade(), q.shown_whole())).collect();
    assert_eq!(
        table,
        [
            ("delete_for_good", "sensitive", true),
            ("send_message", "sensitive", true),
            ("plan_work", "sensitive", false),
            ("change_item", "standard", false),
        ]
    );
}

#[test]
fn a_card_shows_each_value_whole_or_is_not_asked() {
    // A card of the kind that cannot be undone shows a value up to 240 characters.
    let whole = |n: usize| Card::new("t").text("text", &"x".repeat(n)).args();
    assert!(Question::DeleteForGood.fits(&whole(200)).is_ok());
    assert!(Question::DeleteForGood.fits(&whole(300)).unwrap_err().contains("`text`"));
    // The other kind cuts a value at 60 and counts eight rows, so what it would cut is not asked.
    assert!(Question::ChangeItem.fits(&whole(50)).is_ok());
    assert!(Question::ChangeItem.fits(&whole(70)).is_err());
    let mut many = Card::new("t");
    for i in 0..8 {
        many = many.plain(&format!("k{i}"), "v");
    }
    assert!(Question::PlanWork.fits(&many.args()).unwrap_err().contains("fields"), "nine rows is one past the card's eight");
    // Characters the card can only show as escapes count at their escaped length.
    let hidden = Card::new("t").text("text", &"\u{200b}".repeat(10)).args();
    assert!(Question::ChangeItem.fits(&hidden).is_err());
}

#[test]
fn text_inside_a_value_cannot_pose_as_another_field() {
    let card = Card::new("t").text("subject", "Invoice; to: priya@example.com").args();
    assert_eq!(card["subject"], q("Invoice; to: priya@example.com"));
    let drawn = card["subject"].as_str().unwrap();
    assert!(drawn.starts_with('"') && drawn.ends_with('"'));
}

#[test]
fn the_tool_is_always_on_the_card_and_nothing_else_is_added() {
    let args = Card::new("write_file").plain("change", "x").args();
    assert_eq!(args, serde_json::json!({ "tool": "write_file", "change": "x" }));
}

/// "Overwrite groceries.txt with: milk": the card shows the path and the change, Deny leaves the
/// file byte-identical, Allow changes it.
#[test]
fn overwriting_a_file_asks_with_its_path_and_the_change() {
    let dir = scratch("overwrite");
    let file = dir.join("groceries.txt");
    let path = file.to_string_lossy().to_string();
    let original = "eggs\nbread\nbutter\n";
    let reset = || std::fs::write(&file, original).unwrap();
    check_all(&Scenario {
        action: "delete_for_good",
        grade: "sensitive",
        card: serde_json::json!({
            "tool": "write_file", "path": q(&path),
            "change": "replaces 18 bytes, 3 lines with 4 bytes, 1 line",
            "first_line": q("milk"), "last_line": q("milk"),
        }),
        call: &|variant, effect| {
            reset();
            let content = match variant {
                0 => "milk".to_string(),
                1 => "oats".to_string(),
                _ => "x".repeat(400),
            };
            match asks_files::overwrite(&path, &file.to_string_lossy(), &content) {
                Ok(()) => {
                    assert_eq!(std::fs::read_to_string(&file).unwrap(), content, "the file changed");
                    effect()
                }
                Err(why) => {
                    assert_eq!(std::fs::read_to_string(&file).unwrap(), original, "the file is byte-identical");
                    Err(why)
                }
            }
        },
    });
}

#[test]
fn editing_a_file_shows_what_is_removed_and_what_replaces_it() {
    let dir = scratch("edit");
    let file = dir.join("notes.txt");
    let path = file.to_string_lossy().to_string();
    let original = "alpha\nbeta\ngamma\n";
    check_all(&Scenario {
        action: "delete_for_good",
        grade: "sensitive",
        card: serde_json::json!({
            "tool": "edit_file", "path": q(&path),
            "change": "replaces 1 place(s) in a file of 17 bytes, 3 lines; it becomes 18 bytes, 3 lines",
            "removes_first": q("beta"), "removes_last": q("beta"),
            "first_line": q("delta"), "last_line": q("delta"),
        }),
        call: &|variant, effect| {
            std::fs::write(&file, original).unwrap();
            let new = match variant {
                0 => "delta".to_string(),
                1 => "omega".to_string(),
                _ => "z".repeat(400),
            };
            let after = original.replacen("beta", &new, 1);
            match asks_files::edit(&path, &file.to_string_lossy(), 1, "beta", &new, original, &after) {
                Ok(()) => {
                    assert_eq!(std::fs::read_to_string(&file).unwrap(), after);
                    effect()
                }
                Err(why) => {
                    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
                    Err(why)
                }
            }
        },
    });
}

/// The tools themselves: a new file and an append are not overwrites and are not asked about; a
/// file that is there is.
#[test]
fn write_file_asks_only_for_a_file_that_is_there_and_not_for_an_append() {
    use crate::{Tool, ToolContext};
    // The model's own scratch directory: the one place `validate_write_path` lets a test write.
    let Ok(base) = yantrik_ml::private_dir::scratch_dir() else { return };
    let dir = base.join(format!("yantrik-asks-tool-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext { db: &db, max_permission: crate::PermissionLevel::Standard, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    let file = dir.join("a.txt").to_string_lossy().to_string();
    let tool = crate::files::WriteFileTool;
    let shell = FakeShell::session(First::Pending, Answer::Denied);
    let made = tool.execute(&ctx, &serde_json::json!({"path": file, "content": "one"}));
    assert!(made.starts_with("Wrote"), "{made}");
    let appended = tool.execute(&ctx, &serde_json::json!({"path": file, "content": "two", "append": true}));
    assert!(appended.starts_with("Appended"), "{appended}");
    assert!(shell.cards().is_empty(), "neither a new file nor an append raised a card");
    let denied = tool.execute(&ctx, &serde_json::json!({"path": file, "content": "three"}));
    assert!(denied.contains("said no") && denied.contains("Nothing was changed"), "{denied}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "onetwo", "the file is as it was");
    assert_eq!(shell.cards().len(), 1);
}

#[test]
fn forgetting_one_memory_shows_its_own_text() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE memories (rid TEXT, text TEXT, half_life REAL, consolidation_status TEXT);").unwrap();
    let insert = |rid: &str, text: &str| {
        conn.execute("INSERT INTO memories VALUES (?1, ?2, 604800.0, 'active')", rusqlite::params![rid, text]).unwrap();
    };
    let status = |rid: &str| -> String { conn.query_row("SELECT consolidation_status FROM memories WHERE rid = ?1", [rid], |r| r.get(0)).unwrap() };
    check_all(&Scenario {
        action: "change_item",
        grade: "standard",
        card: serde_json::json!({ "tool": "forget_memory", "memory": "mem-aaaaaaaa", "text": q("Likes oat milk") }),
        call: &|variant, effect| {
            conn.execute("DELETE FROM memories", []).unwrap();
            let text = match variant {
                0 => "Likes oat milk".to_string(),
                1 => "Dislikes pears".to_string(),
                _ => "long ".repeat(30),
            };
            insert("mem-aaaaaaaaaaaa", &text);
            match asks_memory::forget_one(&conn, "mem-aaaaaaaaaaaa") {
                Ok(changed) => {
                    assert_eq!((changed, status("mem-aaaaaaaaaaaa").as_str()), (1, "tombstoned"));
                    effect()
                }
                Err(why) => {
                    assert_eq!(status("mem-aaaaaaaaaaaa"), "active", "the memory stays");
                    Err(why)
                }
            }
        },
    });
}
