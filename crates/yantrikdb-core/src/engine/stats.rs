use rusqlite::params;
use tracing::{debug, info};

use crate::error::Result;
use crate::types::Stats;

use super::{now, YantrikDB};

/// Result of a retention/cleanup run.
#[derive(Debug, Clone)]
pub struct RetentionResult {
    pub oplog_deleted: usize,
    pub trigger_log_deleted: usize,
    pub consolidation_log_deleted: usize,
    pub tombstoned_memories_purged: usize,
    pub vacuumed: bool,
}

impl YantrikDB {
    /// Get engine statistics. Optionally filter memory counts by namespace.
    pub fn stats(&self, namespace: Option<&str>) -> Result<Stats> {
        let ns_filter = namespace.map(|ns| format!(" AND namespace = '{}'", ns.replace('\'', "''"))).unwrap_or_default();
        let active = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE consolidation_status = 'active'{}", ns_filter),
            [], |row| row.get(0),
        )?;
        let consolidated = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE consolidation_status = 'consolidated'{}", ns_filter),
            [], |row| row.get(0),
        )?;
        let tombstoned = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE consolidation_status = 'tombstoned'{}", ns_filter),
            [], |row| row.get(0),
        )?;
        let archived = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE storage_tier = 'cold'{}", ns_filter),
            [], |row| row.get(0),
        )?;
        let edges = self.conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE tombstoned = 0",
            [], |row| row.get(0),
        )?;
        let entities = self.conn.query_row(
            "SELECT COUNT(*) FROM entities",
            [], |row| row.get(0),
        )?;
        let operations = self.conn.query_row(
            "SELECT COUNT(*) FROM oplog",
            [], |row| row.get(0),
        )?;
        let open_conflicts = self.conn.query_row(
            "SELECT COUNT(*) FROM conflicts WHERE status = 'open'",
            [], |row| row.get(0),
        )?;
        let resolved_conflicts = self.conn.query_row(
            "SELECT COUNT(*) FROM conflicts WHERE status IN ('resolved', 'dismissed')",
            [], |row| row.get(0),
        )?;
        let pending_triggers = self.conn.query_row(
            "SELECT COUNT(*) FROM trigger_log WHERE status = 'pending'",
            [], |row| row.get(0),
        )?;
        let active_patterns = self.conn.query_row(
            "SELECT COUNT(*) FROM patterns WHERE status = 'active'",
            [], |row| row.get(0),
        )?;

        Ok(Stats {
            active_memories: active,
            consolidated_memories: consolidated,
            tombstoned_memories: tombstoned,
            archived_memories: archived,
            edges,
            entities,
            operations,
            open_conflicts,
            resolved_conflicts,
            pending_triggers,
            active_patterns,
            scoring_cache_entries: self.scoring_cache.borrow().len(),
            vec_index_entries: self.vec_index.borrow().len(),
            graph_index_entities: self.graph_index.borrow().entity_count(),
            graph_index_edges: self.graph_index.borrow().edge_count(),
        })
    }

    /// Append an operation to the oplog with HLC and optional embedding hash.
    ///
    /// If sync is disabled (no peers registered), the oplog write is skipped entirely
    /// to avoid unbounded growth of data that will never be consumed.
    pub fn log_op(
        &self,
        op_type: &str,
        target_rid: Option<&str>,
        payload: &serde_json::Value,
        emb_hash: Option<&[u8]>,
    ) -> Result<String> {
        // Skip oplog write when no sync peers exist — the data would never be consumed.
        let peer_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sync_peers", [], |row| row.get(0),
        ).unwrap_or(0);

        if peer_count == 0 {
            // Still return a valid op_id and tick HLC for local consistency,
            // but don't persist to the oplog table.
            let op_id = uuid7::uuid7().to_string();
            let _hlc_ts = self.tick_hlc();
            return Ok(op_id);
        }

        let op_id = uuid7::uuid7().to_string();
        let hlc_ts = self.tick_hlc();
        let hlc_bytes = hlc_ts.to_bytes().to_vec();
        let payload_str = serde_json::to_string(payload)?;

        self.conn.execute(
            "INSERT INTO oplog (op_id, op_type, timestamp, target_rid, payload, \
             actor_id, hlc, embedding_hash, origin_actor, applied) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)",
            params![
                op_id,
                op_type,
                now(),
                target_rid,
                payload_str,
                self.actor_id,
                hlc_bytes,
                emb_hash,
                self.actor_id,
            ],
        )?;
        Ok(op_id)
    }

    /// Run retention cleanup on oplog, trigger_log, and other accumulated data.
    ///
    /// - `oplog_keep_days`: delete oplog entries older than this (only entries already
    ///   synced to all peers, or all entries if no peers exist). Default: 7 days.
    /// - `trigger_keep_days`: delete non-pending trigger_log entries older than this.
    ///   Default: 7 days.
    /// - `tombstone_keep_days`: permanently purge tombstoned memories older than this.
    ///   Default: 30 days.
    /// - `vacuum`: run VACUUM after deletion to reclaim disk space.
    pub fn run_retention(
        &self,
        oplog_keep_days: Option<f64>,
        trigger_keep_days: Option<f64>,
        tombstone_keep_days: Option<f64>,
        vacuum: bool,
    ) -> Result<RetentionResult> {
        let ts = now();
        let oplog_cutoff = ts - (oplog_keep_days.unwrap_or(7.0) * 86400.0);
        let trigger_cutoff = ts - (trigger_keep_days.unwrap_or(7.0) * 86400.0);
        let tombstone_cutoff = ts - (tombstone_keep_days.unwrap_or(30.0) * 86400.0);

        // ── Oplog retention ──
        // If no sync peers, all entries are safe to delete (they'll never be consumed).
        // If peers exist, only delete entries older than cutoff AND already synced to all peers.
        let peer_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sync_peers", [], |row| row.get(0),
        ).unwrap_or(0);

        let oplog_deleted = if peer_count == 0 {
            // No peers — delete everything older than cutoff
            self.conn.execute(
                "DELETE FROM oplog WHERE timestamp < ?1",
                params![oplog_cutoff],
            )?
        } else {
            // With peers — only delete entries that all peers have already synced past.
            // An entry is safe to delete if its HLC is <= the minimum last_synced_hlc across all peers.
            self.conn.execute(
                "DELETE FROM oplog WHERE timestamp < ?1 \
                 AND hlc <= (SELECT MIN(last_synced_hlc) FROM sync_peers)",
                params![oplog_cutoff],
            )?
        };

        // ── Trigger log retention ──
        // Delete non-pending (delivered, acknowledged, acted, dismissed, expired) entries older than cutoff.
        let trigger_log_deleted = self.conn.execute(
            "DELETE FROM trigger_log WHERE status != 'pending' AND created_at < ?1",
            params![trigger_cutoff],
        )?;

        // Also clean up trigger_source_rids for deleted triggers
        self.conn.execute(
            "DELETE FROM trigger_source_rids WHERE trigger_id NOT IN (SELECT trigger_id FROM trigger_log)",
            [],
        )?;

        // ── Consolidation log retention ──
        // Old consolidation membership records serve no purpose after the consolidation is complete.
        let consolidation_log_deleted = self.conn.execute(
            "DELETE FROM consolidation_log WHERE timestamp < ?1",
            params![oplog_cutoff],
        )?;
        self.conn.execute(
            "DELETE FROM consolidation_members WHERE group_id NOT IN (SELECT id FROM consolidation_log)",
            [],
        )?;

        // ── Tombstoned memory purge ──
        // Memories marked tombstoned for longer than tombstone_keep_days get permanently deleted.
        let tombstoned_memories_purged = self.conn.execute(
            "DELETE FROM memories WHERE consolidation_status = 'tombstoned' AND updated_at < ?1",
            params![tombstone_cutoff],
        )?;

        // Clean up orphaned edges and entities pointing to purged memories
        if tombstoned_memories_purged > 0 {
            self.conn.execute(
                "DELETE FROM edges WHERE source_rid NOT IN (SELECT rid FROM memories) \
                 OR target_rid NOT IN (SELECT rid FROM memories)",
                [],
            )?;
            self.conn.execute(
                "DELETE FROM memory_entities WHERE memory_rid NOT IN (SELECT rid FROM memories)",
                [],
            )?;
        }

        info!(
            oplog = oplog_deleted,
            triggers = trigger_log_deleted,
            consolidation = consolidation_log_deleted,
            tombstones = tombstoned_memories_purged,
            "retention cleanup complete"
        );

        // ── VACUUM ──
        let vacuumed = if vacuum && (oplog_deleted + trigger_log_deleted + tombstoned_memories_purged) > 0 {
            debug!("running VACUUM to reclaim disk space");
            self.conn.execute_batch("VACUUM")?;
            true
        } else {
            false
        };

        Ok(RetentionResult {
            oplog_deleted,
            trigger_log_deleted,
            consolidation_log_deleted,
            tombstoned_memories_purged,
            vacuumed,
        })
    }

    /// Get database size statistics for monitoring.
    pub fn db_size_stats(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, SUM(pgsize) as size_bytes FROM dbstat GROUP BY name ORDER BY size_bytes DESC LIMIT 20"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}
