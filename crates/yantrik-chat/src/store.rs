//! SQLite-backed conversation store — transcripts, dedupe, policies.
//!
//! All chat data flows through here for persistence and searchability.
//! The brain can query this store to detect patterns across all platforms.

use rusqlite::{params, Connection};
use crate::model::*;
use crate::policy::{ConversationPolicy, ReplyMode};

/// Initialize chat tables. Safe to call multiple times.
pub fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS chat_conversations (
            id          INTEGER PRIMARY KEY,
            provider    TEXT NOT NULL,
            conv_id     TEXT NOT NULL,
            kind        TEXT NOT NULL DEFAULT 'direct',
            title       TEXT,
            policy_json TEXT NOT NULL DEFAULT '{\"mode\":\"MentionOnly\",\"allow_tool_use\":true,\"trusted\":false}',
            last_message_at INTEGER,
            last_ai_reply_at INTEGER,
            created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
            UNIQUE(provider, conv_id)
        );

        CREATE TABLE IF NOT EXISTS chat_messages (
            id              INTEGER PRIMARY KEY,
            conversation_id INTEGER NOT NULL REFERENCES chat_conversations(id),
            message_id      TEXT NOT NULL,
            sender_id       TEXT NOT NULL,
            sender_name     TEXT,
            content_text    TEXT,
            content_type    TEXT NOT NULL DEFAULT 'text',
            is_from_ai      INTEGER NOT NULL DEFAULT 0,
            timestamp_ms    INTEGER NOT NULL,
            raw_json        TEXT,
            UNIQUE(conversation_id, message_id)
        );

        CREATE TABLE IF NOT EXISTS chat_seen_events (
            provider    TEXT NOT NULL,
            event_id    TEXT NOT NULL,
            processed_at INTEGER NOT NULL DEFAULT (unixepoch()),
            PRIMARY KEY(provider, event_id)
        );

        CREATE INDEX IF NOT EXISTS idx_chat_msg_sender
            ON chat_messages(sender_id, timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_chat_msg_conv
            ON chat_messages(conversation_id, timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_chat_msg_ts
            ON chat_messages(timestamp_ms);
    ")
}

/// Check if an event has already been processed (dedupe).
pub fn is_event_seen(conn: &Connection, provider: &str, event_id: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM chat_seen_events WHERE provider = ?1 AND event_id = ?2",
        params![provider, event_id],
        |_| Ok(()),
    ).is_ok()
}

/// Mark an event as processed.
pub fn mark_event_seen(conn: &Connection, provider: &str, event_id: &str) {
    let _ = conn.execute(
        "INSERT OR IGNORE INTO chat_seen_events (provider, event_id) VALUES (?1, ?2)",
        params![provider, event_id],
    );
}

/// Get or create a conversation record. Returns (rowid, policy).
pub fn get_or_create_conversation(
    conn: &Connection,
    conv: &ConversationRef,
) -> rusqlite::Result<(i64, ConversationPolicy)> {
    let kind_str = format!("{:?}", conv.kind).to_lowercase();

    // Try to find existing
    let existing = conn.query_row(
        "SELECT id, policy_json FROM chat_conversations WHERE provider = ?1 AND conv_id = ?2",
        params![conv.provider, conv.id],
        |row| {
            let id: i64 = row.get(0)?;
            let policy_json: String = row.get(1)?;
            Ok((id, policy_json))
        },
    );

    match existing {
        Ok((id, policy_json)) => {
            let policy: ConversationPolicy = serde_json::from_str(&policy_json)
                .unwrap_or_default();
            Ok((id, policy))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            // Create new conversation with default policy based on kind
            let default_policy = match conv.kind {
                ConversationKind::Direct => ConversationPolicy {
                    mode: ReplyMode::AutoReply,
                    allow_tool_use: true,
                    trusted: true,
                    ..Default::default()
                },
                _ => ConversationPolicy::default(), // MentionOnly for groups/channels
            };

            let policy_json = serde_json::to_string(&default_policy)
                .unwrap_or_else(|_| "{}".into());

            conn.execute(
                "INSERT INTO chat_conversations (provider, conv_id, kind, title, policy_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![conv.provider, conv.id, kind_str, conv.title, policy_json],
            )?;

            let id = conn.last_insert_rowid();
            Ok((id, default_policy))
        }
        Err(e) => Err(e),
    }
}

/// Store an inbound message in the transcript.
pub fn store_message(
    conn: &Connection,
    conversation_rowid: i64,
    msg: &InboundMessage,
) {
    let content_text = msg.content.text().map(|s| s.to_string());
    let content_type = msg.content.kind_str();

    let _ = conn.execute(
        "INSERT OR IGNORE INTO chat_messages
         (conversation_id, message_id, sender_id, sender_name, content_text, content_type, is_from_ai, timestamp_ms, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)",
        params![
            conversation_rowid,
            msg.message.id,
            msg.sender.id,
            msg.sender.display_name,
            content_text,
            content_type,
            msg.timestamp_ms,
            msg.raw,
        ],
    );

    // Update conversation last_message_at
    let _ = conn.execute(
        "UPDATE chat_conversations SET last_message_at = ?1 WHERE id = ?2",
        params![msg.timestamp_ms, conversation_rowid],
    );
}

/// Store an outbound AI response in the transcript.
pub fn store_ai_response(
    conn: &Connection,
    conversation_rowid: i64,
    message_id: &str,
    text: &str,
    timestamp_ms: i64,
) {
    let _ = conn.execute(
        "INSERT OR IGNORE INTO chat_messages
         (conversation_id, message_id, sender_id, sender_name, content_text, content_type, is_from_ai, timestamp_ms)
         VALUES (?1, ?2, 'yantrik', 'Yantrik', ?3, 'text', 1, ?4)",
        params![conversation_rowid, message_id, text, timestamp_ms],
    );

    let _ = conn.execute(
        "UPDATE chat_conversations SET last_ai_reply_at = ?1 WHERE id = ?2",
        params![timestamp_ms, conversation_rowid],
    );
}

/// Update the policy for a conversation.
pub fn update_policy(
    conn: &Connection,
    provider: &str,
    conv_id: &str,
    policy: &ConversationPolicy,
) -> rusqlite::Result<()> {
    let json = serde_json::to_string(policy).unwrap_or_else(|_| "{}".into());
    conn.execute(
        "UPDATE chat_conversations SET policy_json = ?1 WHERE provider = ?2 AND conv_id = ?3",
        params![json, provider, conv_id],
    )?;
    Ok(())
}

/// Get recent messages for a conversation (for context window).
pub fn recent_messages(
    conn: &Connection,
    conversation_rowid: i64,
    limit: usize,
) -> Vec<(String, String, bool, i64)> {
    let mut stmt = conn.prepare(
        "SELECT sender_name, content_text, is_from_ai, timestamp_ms
         FROM chat_messages
         WHERE conversation_id = ?1 AND content_text IS NOT NULL
         ORDER BY timestamp_ms DESC
         LIMIT ?2"
    ).unwrap();

    let rows = stmt.query_map(params![conversation_rowid, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, i64>(3)?,
        ))
    }).unwrap();

    let mut result: Vec<_> = rows.filter_map(|r| r.ok()).collect();
    result.reverse(); // oldest first
    result
}

/// List all conversations with their policies (for UI display).
pub fn list_conversations(conn: &Connection) -> Vec<(i64, String, String, String, ConversationPolicy, Option<i64>)> {
    let mut stmt = conn.prepare(
        "SELECT id, provider, conv_id, kind, policy_json, last_message_at
         FROM chat_conversations
         ORDER BY last_message_at DESC NULLS LAST"
    ).unwrap();

    stmt.query_map([], |row| {
        let policy_json: String = row.get(4)?;
        let policy: ConversationPolicy = serde_json::from_str(&policy_json)
            .unwrap_or_default();
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            policy,
            row.get(5)?,
        ))
    }).unwrap().filter_map(|r| r.ok()).collect()
}

/// Prune old seen events (keep last 7 days).
pub fn prune_seen_events(conn: &Connection) {
    let cutoff = chrono::Utc::now().timestamp() - 7 * 86400;
    let _ = conn.execute(
        "DELETE FROM chat_seen_events WHERE processed_at < ?1",
        params![cutoff],
    );
}

/// Count messages per sender across all providers (for brain pattern analysis).
pub fn message_counts_by_sender(
    conn: &Connection,
    since_ms: i64,
) -> Vec<(String, String, i64)> {
    let mut stmt = conn.prepare(
        "SELECT m.sender_id, m.sender_name, COUNT(*)
         FROM chat_messages m
         WHERE m.timestamp_ms > ?1 AND m.is_from_ai = 0
         GROUP BY m.sender_id
         ORDER BY COUNT(*) DESC
         LIMIT 100"
    ).unwrap();

    stmt.query_map(params![since_ms], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).unwrap().filter_map(|r| r.ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        conn
    }

    #[test]
    fn dedupe_works() {
        let conn = test_db();
        assert!(!is_event_seen(&conn, "telegram", "123"));
        mark_event_seen(&conn, "telegram", "123");
        assert!(is_event_seen(&conn, "telegram", "123"));
        assert!(!is_event_seen(&conn, "telegram", "456"));
    }

    #[test]
    fn conversation_create_and_policy() {
        let conn = test_db();
        let conv = ConversationRef::direct("telegram", "chat_123");
        let (id, policy) = get_or_create_conversation(&conn, &conv).unwrap();
        assert!(id > 0);
        assert_eq!(policy.mode, ReplyMode::AutoReply); // DMs get AutoReply

        // Second call returns same id
        let (id2, _) = get_or_create_conversation(&conn, &conv).unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn store_and_retrieve_messages() {
        let conn = test_db();
        let conv = ConversationRef::direct("telegram", "chat_1");
        let (conv_id, _) = get_or_create_conversation(&conn, &conv).unwrap();

        let msg = InboundMessage {
            event_id: "e1".into(),
            conversation: conv.clone(),
            message: MessageRef { provider: "telegram".into(), id: "m1".into() },
            sender: ActorRef { id: "u1".into(), display_name: "Alice".into(), is_bot: false },
            timestamp_ms: 1000,
            content: MessageContent::Text { text: "hello".into() },
            reply_to: None,
            mentions_ai: false,
            raw: None,
        };

        store_message(&conn, conv_id, &msg);
        store_ai_response(&conn, conv_id, "m2", "hi there!", 1001);

        let recent = recent_messages(&conn, conv_id, 10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].1, "hello");
        assert_eq!(recent[1].1, "hi there!");
        assert!(recent[1].2); // is_from_ai
    }
}
