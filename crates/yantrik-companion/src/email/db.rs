//! Email SQLite cache — stores fetched emails for offline access.

use rusqlite::Connection;

/// Initialize email tables in the database.
pub fn init_tables(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS email_accounts (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT NOT NULL UNIQUE,
            provider TEXT NOT NULL,
            last_sync_uid INTEGER DEFAULT 0,
            last_sync_ts REAL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS emails (
            id INTEGER PRIMARY KEY,
            account_id INTEGER REFERENCES email_accounts(id),
            uid INTEGER NOT NULL,
            folder TEXT NOT NULL DEFAULT 'INBOX',
            from_addr TEXT NOT NULL,
            from_name TEXT,
            to_addr TEXT,
            subject TEXT,
            date_ts REAL NOT NULL,
            body_preview TEXT,
            body_full TEXT,
            is_read INTEGER DEFAULT 0,
            is_flagged INTEGER DEFAULT 0,
            is_replied INTEGER DEFAULT 0,
            message_id TEXT,
            in_reply_to TEXT,
            ai_summary TEXT,
            importance TEXT DEFAULT 'normal',
            UNIQUE(account_id, uid, folder)
        );
        CREATE INDEX IF NOT EXISTS idx_emails_date ON emails(date_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_emails_folder ON emails(account_id, folder, date_ts DESC);"
    ).unwrap_or_else(|e| tracing::error!("Email DB init failed: {}", e));
}

/// Cached email row.
#[derive(Debug, Clone)]
pub struct CachedEmail {
    pub id: i64,
    pub account_id: i64,
    pub uid: u32,
    pub folder: String,
    pub from_addr: String,
    pub from_name: String,
    pub to_addr: String,
    pub subject: String,
    pub date_ts: f64,
    pub body_preview: String,
    pub body_full: String,
    pub is_read: bool,
    pub is_flagged: bool,
    pub message_id: String,
    pub in_reply_to: String,
    pub ai_summary: String,
    pub importance: String,
}

/// Ensure an account record exists, return its ID.
pub fn ensure_account(conn: &Connection, name: &str, email: &str, provider: &str) -> i64 {
    conn.execute(
        "INSERT OR IGNORE INTO email_accounts (name, email, provider) VALUES (?1, ?2, ?3)",
        rusqlite::params![name, email, provider],
    ).unwrap_or(0);

    conn.query_row(
        "SELECT id FROM email_accounts WHERE email = ?1",
        rusqlite::params![email],
        |row| row.get(0),
    ).unwrap_or(0)
}

/// Get last synced UID for an account.
pub fn get_last_sync_uid(conn: &Connection, account_id: i64) -> u32 {
    conn.query_row(
        "SELECT last_sync_uid FROM email_accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| row.get::<_, i64>(0),
    ).unwrap_or(0) as u32
}

/// Update last synced UID for an account.
pub fn update_last_sync_uid(conn: &Connection, account_id: i64, uid: u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    conn.execute(
        "UPDATE email_accounts SET last_sync_uid = ?1, last_sync_ts = ?2 WHERE id = ?3",
        rusqlite::params![uid as i64, now, account_id],
    ).unwrap_or(0);
}

/// Insert or update an email in the cache.
pub fn upsert_email(
    conn: &Connection,
    account_id: i64,
    uid: u32,
    folder: &str,
    from_addr: &str,
    from_name: &str,
    to_addr: &str,
    subject: &str,
    date_ts: f64,
    body_preview: &str,
    is_read: bool,
    message_id: &str,
    in_reply_to: &str,
) -> i64 {
    conn.execute(
        "INSERT OR REPLACE INTO emails
         (account_id, uid, folder, from_addr, from_name, to_addr, subject, date_ts, body_preview, is_read, message_id, in_reply_to)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            account_id, uid as i64, folder, from_addr, from_name, to_addr,
            subject, date_ts, body_preview, is_read as i32, message_id, in_reply_to
        ],
    ).unwrap_or(0);

    conn.last_insert_rowid()
}

/// List emails in a folder, sorted by date descending.
pub fn list_emails(conn: &Connection, account_id: i64, folder: &str, limit: usize) -> Vec<CachedEmail> {
    let mut stmt = match conn.prepare(
        "SELECT id, account_id, uid, folder, from_addr, from_name, to_addr, subject,
                date_ts, body_preview, COALESCE(body_full, ''), is_read, is_flagged,
                COALESCE(message_id, ''), COALESCE(in_reply_to, ''), COALESCE(ai_summary, ''),
                COALESCE(importance, 'normal')
         FROM emails WHERE account_id = ?1 AND folder = ?2
         ORDER BY date_ts DESC LIMIT ?3"
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("list_emails prepare failed: {}", e);
            return Vec::new();
        }
    };

    let rows = stmt.query_map(rusqlite::params![account_id, folder, limit as i64], |row| {
        Ok(CachedEmail {
            id: row.get(0)?,
            account_id: row.get(1)?,
            uid: row.get::<_, i64>(2)? as u32,
            folder: row.get(3)?,
            from_addr: row.get(4)?,
            from_name: row.get(5)?,
            to_addr: row.get(6)?,
            subject: row.get(7)?,
            date_ts: row.get(8)?,
            body_preview: row.get(9)?,
            body_full: row.get(10)?,
            is_read: row.get::<_, i32>(11)? != 0,
            is_flagged: row.get::<_, i32>(12)? != 0,
            message_id: row.get(13)?,
            in_reply_to: row.get(14)?,
            ai_summary: row.get(15)?,
            importance: row.get(16)?,
        })
    });
    match rows {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            tracing::error!("list_emails query failed: {}", e);
            Vec::new()
        }
    }
}

/// Get a single email by ID with full body.
pub fn get_email(conn: &Connection, email_id: i64) -> Option<CachedEmail> {
    conn.query_row(
        "SELECT id, account_id, uid, folder, from_addr, from_name, to_addr, subject,
                date_ts, body_preview, COALESCE(body_full, ''), is_read, is_flagged,
                COALESCE(message_id, ''), COALESCE(in_reply_to, ''), COALESCE(ai_summary, ''),
                COALESCE(importance, 'normal')
         FROM emails WHERE id = ?1",
        rusqlite::params![email_id],
        |row| Ok(CachedEmail {
            id: row.get(0)?,
            account_id: row.get(1)?,
            uid: row.get::<_, i64>(2)? as u32,
            folder: row.get(3)?,
            from_addr: row.get(4)?,
            from_name: row.get(5)?,
            to_addr: row.get(6)?,
            subject: row.get(7)?,
            date_ts: row.get(8)?,
            body_preview: row.get(9)?,
            body_full: row.get(10)?,
            is_read: row.get::<_, i32>(11)? != 0,
            is_flagged: row.get::<_, i32>(12)? != 0,
            message_id: row.get(13)?,
            in_reply_to: row.get(14)?,
            ai_summary: row.get(15)?,
            importance: row.get(16)?,
        })
    ).ok()
}

/// Update email body (after full fetch).
pub fn update_email_body(conn: &Connection, email_id: i64, body: &str) {
    conn.execute(
        "UPDATE emails SET body_full = ?1 WHERE id = ?2",
        rusqlite::params![body, email_id],
    ).unwrap_or(0);
}

/// Update AI summary for an email.
pub fn update_ai_summary(conn: &Connection, email_id: i64, summary: &str) {
    conn.execute(
        "UPDATE emails SET ai_summary = ?1 WHERE id = ?2",
        rusqlite::params![summary, email_id],
    ).unwrap_or(0);
}

/// Search emails by subject/from.
pub fn search_emails(conn: &Connection, account_id: i64, query: &str, limit: usize) -> Vec<CachedEmail> {
    let pattern = format!("%{}%", query);
    let mut stmt = match conn.prepare(
        "SELECT id, account_id, uid, folder, from_addr, from_name, to_addr, subject,
                date_ts, body_preview, COALESCE(body_full, ''), is_read, is_flagged,
                COALESCE(message_id, ''), COALESCE(in_reply_to, ''), COALESCE(ai_summary, ''),
                COALESCE(importance, 'normal')
         FROM emails WHERE account_id = ?1 AND (subject LIKE ?2 OR from_name LIKE ?2 OR from_addr LIKE ?2)
         ORDER BY date_ts DESC LIMIT ?3"
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("search_emails prepare failed: {}", e);
            return Vec::new();
        }
    };

    let rows = stmt.query_map(rusqlite::params![account_id, pattern, limit as i64], |row| {
        Ok(CachedEmail {
            id: row.get(0)?,
            account_id: row.get(1)?,
            uid: row.get::<_, i64>(2)? as u32,
            folder: row.get(3)?,
            from_addr: row.get(4)?,
            from_name: row.get(5)?,
            to_addr: row.get(6)?,
            subject: row.get(7)?,
            date_ts: row.get(8)?,
            body_preview: row.get(9)?,
            body_full: row.get(10)?,
            is_read: row.get::<_, i32>(11)? != 0,
            is_flagged: row.get::<_, i32>(12)? != 0,
            message_id: row.get(13)?,
            in_reply_to: row.get(14)?,
            ai_summary: row.get(15)?,
            importance: row.get(16)?,
        })
    });
    match rows {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            tracing::error!("search_emails query failed: {}", e);
            Vec::new()
        }
    }
}

/// Mark an email as read in the cache.
pub fn mark_read(conn: &Connection, email_id: i64) {
    conn.execute(
        "UPDATE emails SET is_read = 1 WHERE id = ?1",
        rusqlite::params![email_id],
    ).unwrap_or(0);
}

/// Mark an email as unread in the cache.
pub fn mark_unread(conn: &Connection, email_id: i64) {
    conn.execute(
        "UPDATE emails SET is_read = 0 WHERE id = ?1",
        rusqlite::params![email_id],
    ).unwrap_or(0);
}

/// Mark all emails in a folder as read in the cache.
pub fn mark_all_read(conn: &Connection, account_id: i64, folder: &str) -> usize {
    conn.execute(
        "UPDATE emails SET is_read = 1 WHERE account_id = ?1 AND folder = ?2 AND is_read = 0",
        rusqlite::params![account_id, folder],
    ).unwrap_or(0)
}

/// Set flagged status on an email in the cache.
pub fn set_flagged(conn: &Connection, email_id: i64, flagged: bool) {
    conn.execute(
        "UPDATE emails SET is_flagged = ?1 WHERE id = ?2",
        rusqlite::params![flagged as i32, email_id],
    ).unwrap_or(0);
}

/// Delete an email from the cache.
pub fn delete_email(conn: &Connection, email_id: i64) {
    conn.execute(
        "DELETE FROM emails WHERE id = ?1",
        rusqlite::params![email_id],
    ).unwrap_or(0);
}

/// Get all unread email UIDs in a folder (for bulk IMAP operations).
pub fn get_unread_uids(conn: &Connection, account_id: i64, folder: &str) -> Vec<u32> {
    let mut stmt = match conn.prepare(
        "SELECT uid FROM emails WHERE account_id = ?1 AND folder = ?2 AND is_read = 0"
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![account_id, folder], |row| {
        row.get::<_, i64>(0).map(|u| u as u32)
    });
    match rows {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Count unread emails in a folder.
pub fn count_unread(conn: &Connection, account_id: i64, folder: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM emails WHERE account_id = ?1 AND folder = ?2 AND is_read = 0",
        rusqlite::params![account_id, folder],
        |row| row.get(0),
    ).unwrap_or(0)
}
