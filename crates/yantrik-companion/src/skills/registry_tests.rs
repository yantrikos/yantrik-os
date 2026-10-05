//! A skill's switch shows what the database holds, never only what memory hoped.
use super::*;

fn registry() -> (Connection, SkillRegistry) {
    let conn = Connection::open_in_memory().expect("an in-memory database");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    let reg = SkillRegistry::init(&conn, &dir);
    assert!(reg.get("calendar").is_some(), "the shipped calendar skill loads");
    (conn, reg)
}

/// The database refuses the write (read-only, full, locked): the switch stays where it was and
/// the refusal is the caller's to show. It used to flip in memory and drop the error, so the
/// Skills screen said "on" for a skill that came back off at the next start.
#[test]
fn a_toggle_the_database_refuses_leaves_the_skill_as_it_was() {
    let (conn, mut reg) = registry();
    conn.execute_batch("PRAGMA query_only = ON").unwrap();
    let before = reg.get("calendar").unwrap().enabled;
    let refused = reg.toggle(&conn, "calendar").expect_err("a read-only database refuses the write");
    assert!(refused.contains("did not take it"), "{refused}");
    assert_eq!(reg.get("calendar").unwrap().enabled, before, "a refused write changed the switch");
}

#[test]
fn a_toggle_the_database_takes_is_what_the_next_start_reads() {
    let (conn, mut reg) = registry();
    let before = reg.get("calendar").unwrap().enabled;
    let (now, _) = reg.toggle(&conn, "calendar").expect("the database takes it");
    assert_eq!(now, !before);
    assert_eq!(reg.get("calendar").unwrap().enabled, !before);
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    let restarted = SkillRegistry::init(&conn, &dir);
    assert_eq!(restarted.get("calendar").unwrap().enabled, !before, "the database holds what the switch shows");
}
