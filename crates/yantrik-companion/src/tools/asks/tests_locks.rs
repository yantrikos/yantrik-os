//! A card that waits does not hold the database (#1313, M2), and `forget_topic` cancels the
//! schedules it listed and no others (L5).
//!
//! While a card is up the fake shell tries to take the database's lock on another thread; if the
//! tool still held it, that would wait past the probe's patience and the test would say so.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use yantrik_companion_tools::fake_shell::{Answer, FakeShell, First, Probe};

use super::tests_support::{ctx, db};
use crate::recipe::RecipeStore;
use crate::tools::memory::ForgetTopicTool;
use crate::tools::{Tool, ToolRegistry};

/// A probe that notes whether the database could be taken while a card was up.
fn lock_probe(db: &Arc<yantrikdb_core::YantrikDB>) -> (Probe, Arc<AtomicBool>) {
    let free = Arc::new(AtomicBool::new(false));
    let (shared, noted) = (db.clone(), free.clone());
    let probe: Probe = Arc::new(move || {
        let (tx, rx) = channel();
        let db = shared.clone();
        std::thread::spawn(move || {
            let _held_briefly = db.conn();
            let _ = tx.send(());
        });
        noted.store(rx.recv_timeout(Duration::from_millis(100)).is_ok(), Ordering::SeqCst);
    });
    (probe, free)
}

fn run(register: fn(&mut ToolRegistry), db: &yantrikdb_core::YantrikDB, tool: &str, args: serde_json::Value) -> String {
    let mut reg = ToolRegistry::new();
    register(&mut reg);
    reg.execute(&ctx(db), tool, &args)
}

#[test]
fn run_recipe_does_not_hold_the_database_while_the_card_waits() {
    let db = Arc::new(db());
    RecipeStore::create(&db.conn(), "Digest", "", &[crate::recipe::RecipeStep::Notify { message: "hi".into() }], None);
    let (probe, free) = lock_probe(&db);
    let shell = FakeShell::session_probing(First::Pending, Answer::Granted, Some(probe));
    let said = run(crate::tools::recipe::register, &db, "run_recipe", json!({"recipe_id": "Digest"}));
    assert!(said.contains("queued"), "{said}");
    assert_eq!(shell.cards().len(), 1, "a card was up");
    assert!(free.load(Ordering::SeqCst), "the database was taken by another user while the card waited");
}

#[test]
fn forget_memory_does_not_hold_the_database_while_the_card_waits() {
    let db = Arc::new(db());
    super::tests_forget::remember(&db, "Likes oat milk");
    let (probe, free) = lock_probe(&db);
    let shell = FakeShell::session_probing(First::Pending, Answer::Denied, Some(probe));
    let said = run(yantrik_companion_tools::memory_hygiene::register, &db, "forget_memory", json!({"rid": "mem-"}));
    assert!(said.contains("said no"), "{said}");
    assert_eq!(shell.cards().len(), 1, "a card was up");
    assert!(free.load(Ordering::SeqCst), "the database was taken by another user while the card waited");
}

#[test]
fn forget_topic_does_not_hold_the_database_while_the_card_waits() {
    let db = Arc::new(db());
    super::tests_forget::remember(&db, "Plays tennis on Sundays");
    let (probe, free) = lock_probe(&db);
    let shell = FakeShell::session_probing(First::Pending, Answer::Denied, Some(probe));
    let said = ForgetTopicTool.execute(&ctx(&db), &json!({"topic": "tennis"}));
    assert!(said.contains("said no"), "{said}");
    assert_eq!(shell.cards().len(), 1, "a card was up");
    assert!(free.load(Ordering::SeqCst), "the database was taken by another user while the card waited");
}

fn schedule(db: &yantrikdb_core::YantrikDB, id: &str, label: &str) {
    db.conn()
        .execute(
            "INSERT INTO scheduled_tasks (task_id, label, description, schedule_type, status, created_at, updated_at) VALUES (?1, ?2, '', 'interval', 'active', 0, 0)",
            rusqlite::params![id, label],
        )
        .unwrap();
}

fn status(db: &yantrikdb_core::YantrikDB, id: &str) -> String {
    db.conn().query_row("SELECT status FROM scheduled_tasks WHERE task_id = ?1", [id], |r| r.get(0)).unwrap()
}

/// L5: the schedules cancelled are the ones the card listed, by id: two with one label are both
/// on the card, and one made while the person was deciding is not cancelled unseen.
#[test]
fn forgetting_a_topic_cancels_the_schedules_on_the_card_and_no_others() {
    let db = Arc::new(db());
    schedule(&db, "t1", "tennis practice");
    schedule(&db, "t2", "tennis practice");
    schedule(&db, "c1", "chess club");
    let during = {
        let db = db.clone();
        Some(Arc::new(move || schedule(&db, "t3", "tennis lessons")) as Probe)
    };
    let shell = FakeShell::session_probing(First::Pending, Answer::Granted, during);
    let said = ForgetTopicTool.execute(&ctx(&db), &json!({"topic": "tennis"}));
    assert!(said.starts_with("Forgot topic") && said.contains("cancelled 2 scheduled tasks"), "{said}");
    let listed = shell.cards()[0]["args_json"]["schedules"].as_str().unwrap().to_string();
    assert!(listed.starts_with("2 cancelled: ") && listed.matches("tennis practice").count() == 2, "both are on the card: {listed}");
    assert_eq!((status(&db, "t1").as_str(), status(&db, "t2").as_str()), ("cancelled", "cancelled"));
    assert_eq!(status(&db, "t3"), "active", "made during the wait, never shown, not cancelled");
    assert_eq!(status(&db, "c1"), "active");
}
