//! `forget_topic` asks before it tombstones, deletes or cancels anything, and shows what goes
//! (#1311): "Forget everything about <topic>" shows the memories, Deny keeps them.

use yantrik_companion_tools::asks_scenarios::{check_all, Scenario};
use yantrik_companion_tools::fake_shell::{Answer, FakeShell, First};

use super::tests_support::{ctx, db, q};
use crate::tools::memory::ForgetTopicTool;
use crate::tools::Tool;

fn remember(db: &yantrikdb_core::YantrikDB, text: &str) {
    // Straight into the table: a test database has no embedder, so `record_text` would refuse.
    let rid = format!("mem-{}", text.len());
    db.conn()
        .execute("INSERT INTO memories (rid, text, consolidation_status) VALUES (?1, ?2, 'active')", rusqlite::params![rid, text])
        .expect("a memory");
}

fn tombstoned(db: &yantrikdb_core::YantrikDB) -> i64 {
    db.conn().query_row("SELECT COUNT(*) FROM memories WHERE consolidation_status = 'tombstoned'", [], |r| r.get(0)).unwrap_or(0)
}

#[test]
fn forgetting_a_topic_asks_with_the_text_of_every_memory_that_goes() {
    check_all(&Scenario {
        action: "delete_for_good",
        grade: "sensitive",
        card: serde_json::json!({
            "tool": "forget_topic", "topic": q("tennis"),
            "memories": format!("1: {}", q("Plays tennis on Sundays").as_str().unwrap()),
            "also": "adds a rule not to bring the topic up again",
        }),
        call: &|variant, effect| {
            let db = db();
            let (topic, text) = match variant {
                0 => ("tennis", "Plays tennis on Sundays".to_string()),
                1 => ("chess", "Plays chess online".to_string()),
                // A memory too long for the card to show whole: the topic is not asked about.
                _ => ("tennis", format!("tennis {}", "x".repeat(300))),
            };
            remember(&db, &text);
            let said = ForgetTopicTool.execute(&ctx(&db), &serde_json::json!({"topic": topic}));
            if tombstoned(&db) == 1 {
                assert!(said.starts_with("Forgot topic"), "{said}");
                return effect();
            }
            assert_eq!(tombstoned(&db), 0, "the memory is kept: {said}");
            Err(said)
        },
    });
}

#[test]
fn a_denied_topic_keeps_its_memories_and_adds_no_rule() {
    let shell = FakeShell::session(First::Pending, Answer::Denied);
    let db = db();
    remember(&db, "Plays tennis on Sundays");
    let said = ForgetTopicTool.execute(&ctx(&db), &serde_json::json!({"topic": "tennis"}));
    assert!(said.contains("said no") && said.contains("Nothing was changed"), "{said}");
    assert_eq!(tombstoned(&db), 0);
    let rules: i64 = db.conn().query_row("SELECT COUNT(*) FROM memories WHERE text LIKE 'USER PREFERENCE%'", [], |r| r.get(0)).unwrap_or(-1);
    assert_eq!(rules, 0, "no suppression rule was written");
    assert_eq!(shell.cards().len(), 1);
}
