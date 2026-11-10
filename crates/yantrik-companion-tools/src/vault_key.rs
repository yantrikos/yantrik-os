//! The vault's data key, created once however many connections ask for it first.
//!
//! The engine's `vault_encryption` reads the key row and, when there is none, generates a key and
//! writes it with `INSERT OR REPLACE`. On one connection that is atomic enough; on two it is not,
//! and since the vault got its own thread and connection (`yantrik-ui`'s `vault_lane`) two
//! connections can be first at the same moment. Each mints a key, the second replaces the first,
//! and whatever the first sealed can never be read again. Every provider and mail key read then
//! fails (`provider_keys::load_all` fails on one row it cannot open).
//!
//! The fix is that nobody reaches `vault_encryption` without the row existing, and the row is
//! made by one statement: `INSERT OR IGNORE` of a fresh key, guarded by `NOT EXISTS` on both key
//! rows, so the first committed key wins, a loser's key is discarded, and a vault that already
//! has a passphrase (its key wrapped, the clear row deleted) is never given a clear key beside it.
//! SQLite runs the statement under the write lock, so it cannot interleave with `set_passphrase`
//! when that runs in a transaction (`vault_lane` does).

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rusqlite::Connection;
use yantrikdb_core::encryption::EncryptionProvider;

/// Make sure the vault has a data key to read, atomically. Errors are left for the caller's next
/// engine call to report.
pub fn ensure(conn: &Connection) {
    let fresh = B64.encode(yantrikdb_core::encryption::generate_key());
    let _ = conn.execute(
        "INSERT OR IGNORE INTO vault_security (key, value) \
         SELECT 'vault_dek', ?1 \
         WHERE NOT EXISTS (SELECT 1 FROM vault_security WHERE key IN ('vault_dek', 'dek_wrapped'))",
        [fresh],
    );
}

/// The vault's encryption, with the key made first if there is none. Use this, not the engine's
/// `vault_encryption`, everywhere in the OS.
pub fn encryption(conn: &Connection) -> Result<EncryptionProvider, yantrikdb_core::YantrikDbError> {
    ensure(conn);
    yantrikdb_core::vault::vault_encryption(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn store_file(tag: &str, round: usize) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("yantrik-vkey-{}-{nanos}-{tag}-{round}.db", std::process::id()));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        yantrikdb_core::vault::init_tables(&conn);
        path
    }

    fn remove(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// Two connections making the very first access at the same moment: one key results, and a
    /// credential each of them sealed reads back on both.
    #[test]
    fn two_connections_racing_first_access_end_with_one_key_both_read() {
        for round in 0..25 {
            let path = store_file("race", round);
            let barrier = Arc::new(Barrier::new(2));
            let sealers: Vec<_> = ["one", "two"]
                .into_iter()
                .map(|who| {
                    let (path, barrier) = (path.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        let conn = Connection::open(&path).unwrap();
                        conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
                        barrier.wait();
                        let enc = encryption(&conn).expect("a key");
                        yantrikdb_core::vault::store(&conn, &enc, who, "u", &format!("fake-secret-{who}"), None, None, None).expect("stored");
                    })
                })
                .collect();
            for s in sealers {
                s.join().unwrap();
            }
            let conn = Connection::open(&path).unwrap();
            let keys: i64 = conn.query_row("SELECT count(*) FROM vault_security WHERE key = 'vault_dek'", [], |r| r.get(0)).unwrap();
            assert_eq!(keys, 1, "round {round}: one key");
            let enc = encryption(&conn).unwrap();
            for who in ["one", "two"] {
                let got = yantrikdb_core::vault::get(&conn, &enc, who).expect("readable");
                assert_eq!(got.first().map(|e| e.password.clone()), Some(format!("fake-secret-{who}")), "round {round}: {who}");
            }
            remove(&path);
        }
    }

    /// A vault that has a passphrase is never given a clear key beside its wrapped one.
    #[test]
    fn a_protected_vault_gets_no_clear_key() {
        let path = store_file("protected", 0);
        let conn = Connection::open(&path).unwrap();
        conn.execute("INSERT INTO vault_security (key, value) VALUES ('dek_wrapped', 'x')", []).unwrap();
        ensure(&conn);
        let clear: i64 = conn.query_row("SELECT count(*) FROM vault_security WHERE key = 'vault_dek'", [], |r| r.get(0)).unwrap();
        assert_eq!(clear, 0);
        remove(&path);
    }
}
