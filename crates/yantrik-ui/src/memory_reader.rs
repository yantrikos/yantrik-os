//! The newest memories, read straight from the memory database, never through the companion.
//!
//! VM 520, 4 October, OS ace075fd: the Memory screen opened on "1049 stored · searching" over an
//! empty body and stayed that way. The newest memories were asked of the companion worker
//! (`CompanionCommand::RecentMemories`), and that worker serves one command at a time
//! (bridge.rs, `worker_loop`): behind its own background turns ("Review my memory graph…", a
//! QueryPlanner call) the listing waited its turn, and the screen with it. A listing of stored
//! rows has nothing to do with the language model, so it must never queue behind it.
//!
//! So this opens its OWN connection: read-only (`SQLITE_OPEN_READ_ONLY` plus `query_only`), the
//! way yantrikdb itself opens its read pool, beside the worker's writer, under WAL. It reads only
//! when the worker has said the store keeps its text in the clear (`set_plain`): an encrypted
//! store's rows are only readable through the engine that holds the key, and then the caller
//! falls back to the worker and its timeout.
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bridge::MemoryResult;

/// Whether the store's text may be read without the engine: set by the worker once it has opened
/// the store (`YantrikDB::is_encrypted`). False until then, which sends every read to the worker.
static PLAIN: AtomicBool = AtomicBool::new(false);

/// Said by the worker, once, after it has opened the store.
pub fn set_plain(plain: bool) {
    PLAIN.store(plain, Ordering::Release);
}

/// How long a read waits on the writer's lock before giving up. WAL readers rarely wait at all.
const BUSY_MS: u64 = 500;

/// The newest `limit` memories at `db_path`, by recall's own domain rule
/// (`crate::recent_memories::collect`). `Err` when this path cannot answer — the store is
/// encrypted, not there yet, or unreadable — and the caller should ask the worker instead.
pub fn newest(db_path: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
    if !PLAIN.load(Ordering::Acquire) {
        return Err("the memory store is only readable through the companion".into());
    }
    newest_from(db_path, limit)
}

/// [`newest`] without the encryption gate, for the tests' own plain stores.
fn newest_from(db_path: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
    use rusqlite::{Connection, OpenFlags};
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("cannot open {db_path} to read: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_MS)).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "query_only", true).map_err(|e| e.to_string())?;

    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM memories WHERE consolidation_status = 'active'", [], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())? as usize;
    let mut page = conn
        .prepare(
            "SELECT rid, type, text, created_at, importance, valence, domain FROM memories \
             WHERE consolidation_status = 'active' ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| e.to_string())?;
    crate::recent_memories::collect(
        limit,
        |m: &MemoryResult| m.domain.as_str(),
        |offset, size| {
            let rows = page
                .query_map(rusqlite::params![size as i64, offset as i64], |r| {
                    Ok(MemoryResult {
                        rid: r.get(0)?,
                        memory_type: r.get(1)?,
                        text: r.get(2)?,
                        created_at: r.get(3)?,
                        importance: r.get(4)?,
                        valence: r.get(5)?,
                        score: 0.0,
                        domain: r.get(6)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok((rows, total))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store with yantrikdb's `memories` columns this reads, and the rows given.
    fn store(rows: &[(&str, &str, f64, &str, &str)]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "yantrik-memory-reader-{}-{}.db",
            std::process::id(),
            rows.len()
        ));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (rid TEXT, type TEXT, text TEXT, created_at REAL, importance REAL, \
             valence REAL, domain TEXT, consolidation_status TEXT);",
        )
        .unwrap();
        for (rid, text, at, domain, status) in rows {
            conn.execute(
                "INSERT INTO memories VALUES (?1, 'semantic', ?2, ?3, 0.5, 0.0, ?4, ?5)",
                rusqlite::params![rid, text, at, domain, status],
            )
            .unwrap();
        }
        path
    }

    #[test]
    fn the_newest_are_read_without_the_companion_by_recalls_rule() {
        let path = store(&[
            ("m1", "oldest", 1.0, "general", "active"),
            ("m2", "a tool ran", 4.0, "audit/tools", "active"),
            ("m3", "newest", 5.0, "people", "active"),
            ("m4", "merged away", 6.0, "general", "consolidated"),
            ("m5", "middle", 3.0, "work", "active"),
        ]);
        let got = newest_from(path.to_str().unwrap(), 20).unwrap();
        let texts: Vec<&str> = got.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["newest", "middle", "oldest"], "newest first, audit lines and merged rows left out");
        assert!(got.iter().all(|m| m.score == 0.0), "a listing, not a search");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_encrypted_or_unopened_store_is_left_to_the_companion() {
        // Until the worker has said the store is in the clear, nothing is read here.
        set_plain(false);
        assert!(newest("/nonexistent/memory.db", 20).is_err());
        // And a store that is not there is an error, never an empty page passed off as one.
        assert!(newest_from("/nonexistent/memory.db", 20).is_err());
    }
}
