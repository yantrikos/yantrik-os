//! What the person pressed on a brief's card, written down at once.
//!
//! The worker records it too, but the worker serves one command at a time, so a press queued
//! behind a long turn, or a shell restarted before the worker reached it, would be lost: a
//! dismissed card would come back. So the press is also written here, on a short connection of
//! the shell's own, off the UI thread. Both writes say the same thing and either may come first.
//! The store may keep its text encrypted, which only the engine can open; then only the worker
//! writes.

use yantrik_companion::meeting_brief::{press, Press};

/// How long the write waits on the worker's lock before giving up.
const BUSY_MS: u64 = 2_000;

/// Now, unix seconds, as the table keeps it.
pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn write(db_path: &str, id: &str, what: Press) -> Result<(), String> {
    let conn = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_MS)).map_err(|e| e.to_string())?;
    press(&conn, id, what, now());
    Ok(())
}

/// Write the press now, on a thread of its own, when the store can be written without the engine.
pub fn write_now(db_path: &str, id: &str, what: Press) {
    if !crate::memory_reader::is_plain() {
        return;
    }
    let (path, id) = (db_path.to_string(), id.to_string());
    let spawned = std::thread::Builder::new().name("yos-meeting-brief-press".into()).spawn(move || {
        if let Err(why) = write(&path, &id, what) {
            tracing::debug!(error = %why, "a meeting brief press could not be written directly; the worker will");
        }
    });
    if let Err(e) = spawned {
        tracing::debug!(error = %e, "could not start the meeting brief press writer");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion::anticipation::{AnticipationEngine, PrepOutcome, PreparationTrigger, PreparedItem, PreparedKind};

    #[test]
    fn a_press_is_written_on_a_connection_of_its_own() {
        let path = std::env::temp_dir().join(format!("brief-press-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        AnticipationEngine::ensure_table(&conn).unwrap();
        AnticipationEngine::store(
            &conn,
            &PreparedItem {
                id: "meeting:e1:100".into(),
                trigger: PreparationTrigger::UserRequested,
                kind: PreparedKind::MeetingBrief,
                description: "Brief".into(),
                data: serde_json::json!({ "title": "SECRET" }),
                relevant_at: 0.0,
                expires_at: now() + 3600.0,
                surfaced: true,
                outcome: None,
                created_at: now(),
            },
        );
        write(path.to_str().unwrap(), "meeting:e1:100", Press::Dismissed).unwrap();
        let item = AnticipationEngine::get(&conn, "meeting:e1:100").unwrap();
        assert_eq!(item.outcome, Some(PrepOutcome::Dismissed));
        assert_eq!(item.data, serde_json::json!({}), "what the card said is gone");
        let _ = std::fs::remove_file(path);
    }
}
