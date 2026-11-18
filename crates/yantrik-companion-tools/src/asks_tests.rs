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

/// The check at the moment of the write, for tests that write outside the home the real one guards.
fn allowed(path: &str) -> Result<String, String> {
    Ok(path.to_string())
}

#[test]
fn the_four_questions_are_named_and_graded_as_the_shell_publishes_them() {
    let table: Vec<_> = Question::ALL.iter().map(|q| (q.action(), q.grade(), q.shown_whole())).collect();
    assert_eq!(
        table,
        [
            ("delete_for_good", "sensitive", true),
            ("send_message", "sensitive", true),
            ("plan_work", "sensitive", true),
            ("change_item", "standard", true),
        ]
    );
}

#[test]
fn a_card_shows_each_value_whole_or_is_not_asked() {
    // Every question is drawn whole: a value up to 240 characters, twenty rows, 2400 together.
    let whole = |n: usize| Card::new("t").text("text", &"x".repeat(n)).args();
    for question in Question::ALL {
        assert!(question.fits(&whole(61)).is_ok(), "{question:?}: sixty-one characters is asked and drawn in full");
        assert!(question.fits(&whole(200)).is_ok());
        assert!(question.fits(&whole(300)).unwrap_err().contains("`text`"), "{question:?}");
    }
    let rows = |n: usize| crate::asks::fields().into_iter().skip(1).take(n).fold(Card::new("t"), |card, (key, _)| card.plain(&key, "v")).args();
    assert!(Question::PlanWork.fits(&rows(19)).is_ok(), "twenty rows with the tool");
    assert!(Question::PlanWork.fits(&rows(20)).unwrap_err().contains("fields"), "twenty-one is one past the card's twenty");
    let heavy = (0..12).fold(Card::new("t"), |card, i| card.text(&format!("step_{}", i + 1), &"y".repeat(230))).args();
    assert!(Question::PlanWork.fits(&heavy).unwrap_err().contains("a card shows"), "past the total");
    let thirteen = Card::new("t").plain("step_13", "x").args();
    assert!(Question::PlanWork.fits(&thirteen).unwrap_err().contains("at most 12 steps"));
    // Characters the card can only show as escapes count at their escaped length.
    let hidden = Card::new("t").text("text", &"\u{200b}".repeat(30)).args();
    assert!(Question::ChangeItem.fits(&hidden).is_err());
}

#[test]
fn a_press_is_told_from_the_mode_and_the_shell_must_say_which() {
    let shell = FakeShell::start(First::Pending, Answer::Granted);
    let card = Card::new("t").plain("change", "x");
    let pressed = crate::asks::ask_on(&shell.desk(), Question::PlanWork, &card).expect("allowed");
    assert!(pressed.pressed() && pressed.answered() == crate::asks::Answered::Pressed);
    let by_mode = FakeShell::start(First::NotNeeded, Answer::Granted);
    let unasked = crate::asks::ask_on(&by_mode.desk(), Question::PlanWork, &card).expect("the mode runs it");
    assert!(!unasked.pressed() && unasked.answered() == crate::asks::Answered::Mode);
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
    let original = "eggs
bread
butter
";
    check_all(&Scenario {
        action: "delete_for_good",
        grade: "sensitive",
        card: serde_json::json!({
            "tool": "write_file", "path": q(&path),
            "change": "replaces the whole file (was 3 lines, 18 characters; becomes 1 line, 4 characters); the first and last lines are shown",
            "first_line": q("milk"), "last_line": q("milk"),
        }),
        call: &|variant, effect| {
            // Too long to show whole is a path: the lines are a preview, cut and marked.
            let (target, content) = match variant {
                0 => (file.clone(), "milk"),
                1 => (file.clone(), "oats"),
                _ => (deep(&dir), "milk"),
            };
            std::fs::write(&target, original).unwrap();
            match asks_files::overwrite_with(&allowed, &target.to_string_lossy(), &target.to_string_lossy(), content) {
                Ok(()) => {
                    assert_eq!(std::fs::read_to_string(&target).unwrap(), content, "the file changed");
                    effect()
                }
                Err(why) => {
                    assert_eq!(std::fs::read_to_string(&target).unwrap(), original, "the file is byte-identical");
                    Err(why)
                }
            }
        },
    });
}

#[test]
fn a_long_replacement_is_shown_by_its_first_and_last_lines_with_the_cut_marked() {
    let dir = scratch("preview");
    let file = dir.join("a.txt");
    std::fs::write(&file, "old").unwrap();
    let shell = FakeShell::session(First::Pending, Answer::Denied);
    let content = format!("{}
middle
{}", "x".repeat(100), "y".repeat(100));
    asks_files::overwrite_with(&allowed, "a.txt", &file.to_string_lossy(), &content).unwrap_err();
    let card = &shell.cards()[0]["args_json"];
    let drawn = |key: &str| serde_json::from_str::<String>(card[key].as_str().unwrap()).unwrap();
    assert!(drawn("first_line").starts_with(&"x".repeat(40)) && drawn("first_line").ends_with("(100 characters in all)"));
    assert!(drawn("last_line").ends_with("(100 characters in all)"));
    assert!(!card.to_string().contains("middle"), "the middle of the text is not claimed to be shown");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
}

/// A path the card cannot show whole: three folders of ninety characters.
fn deep(dir: &std::path::Path) -> PathBuf {
    let deep = dir.join("a".repeat(90)).join("b".repeat(90)).join("c".repeat(90));
    std::fs::create_dir_all(&deep).unwrap();
    deep.join("groceries.txt")
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
            "change": "replaces 1 place(s) (the file was 3 lines, 17 characters; becomes 3 lines, 18 characters); only the first and last lines of what goes and what comes are shown",
            "removes_first": q("beta"), "removes_last": q("beta"),
            "first_line": q("delta"), "last_line": q("delta"),
        }),
        call: &|variant, effect| {
            let (target, new) = match variant {
                0 => (file.clone(), "delta"),
                1 => (file.clone(), "omega"),
                _ => (deep(&dir), "delta"),
            };
            std::fs::write(&target, original).unwrap();
            let after = original.replacen("beta", new, 1);
            let shown = target.to_string_lossy().to_string();
            match asks_files::edit_with(&allowed, &shown, &shown, 1, "beta", new, original, &after) {
                Ok(()) => {
                    assert_eq!(std::fs::read_to_string(&target).unwrap(), after);
                    effect()
                }
                Err(why) => {
                    assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
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
                _ => "long ".repeat(60),
            };
            insert("mem-aaaaaaaaaaaa", &text);
            match asks_memory::forget_one(|| &conn, "mem-aaaaaaaaaaaa") {
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
