//! Memory hygiene tools — self-curation for the companion.
//!
//! Lets the companion review, clean, re-tier, and resolve conflicts
//! in its own memory. These tools enable the companion to take ownership
//! of its memory health rather than relying on automated heuristics alone.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(MemoryStatsTool));
    reg.register(Box::new(ReviewMemoriesTool));
    reg.register(Box::new(ForgetMemoryTool));
    reg.register(Box::new(UpdateMemoryTool));
    reg.register(Box::new(ResolveConflictsTool));
    reg.register(Box::new(PurgeSystemNoiseTool));
}

// ── Memory Stats ──

struct MemoryStatsTool;

impl Tool for MemoryStatsTool {
    fn name(&self) -> &'static str { "memory_stats" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "memory_hygiene" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "memory_stats",
                "description": "Show memory counts and storage stats only",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let conn = ctx.db.conn();

        let mut out = String::from("=== Memory Health Report ===\n\n");

        // Total count
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE consolidation_status IS NULL OR consolidation_status = 'active'",
            [], |r| r.get(0),
        ).unwrap_or(0);
        out.push_str(&format!("Total active memories: {}\n\n", total));

        // By domain
        out.push_str("By domain:\n");
        if let Ok(mut stmt) = conn.prepare(
            "SELECT domain, COUNT(*) as cnt FROM memories \
             WHERE consolidation_status IS NULL OR consolidation_status = 'active' \
             GROUP BY domain ORDER BY cnt DESC LIMIT 15"
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            }) {
                for row in rows.flatten() {
                    out.push_str(&format!("  {:25} {}\n", row.0, row.1));
                }
            }
        }

        // By source
        out.push_str("\nBy source:\n");
        if let Ok(mut stmt) = conn.prepare(
            "SELECT source, COUNT(*) as cnt FROM memories \
             WHERE consolidation_status IS NULL OR consolidation_status = 'active' \
             GROUP BY source ORDER BY cnt DESC"
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            }) {
                for row in rows.flatten() {
                    out.push_str(&format!("  {:25} {}\n", row.0, row.1));
                }
            }
        }

        // By storage tier
        out.push_str("\nBy storage tier:\n");
        if let Ok(mut stmt) = conn.prepare(
            "SELECT storage_tier, COUNT(*) as cnt FROM memories \
             WHERE consolidation_status IS NULL OR consolidation_status = 'active' \
             GROUP BY storage_tier ORDER BY cnt DESC"
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            }) {
                for row in rows.flatten() {
                    out.push_str(&format!("  {:25} {}\n", row.0, row.1));
                }
            }
        }

        // Open conflicts
        let conflicts: i64 = conn.query_row(
            "SELECT COUNT(*) FROM conflicts WHERE status = 'open'",
            [], |r| r.get(0),
        ).unwrap_or(0);
        out.push_str(&format!("\nOpen conflicts: {}\n", conflicts));

        // Patterns
        let patterns: i64 = conn.query_row(
            "SELECT COUNT(*) FROM patterns", [], |r| r.get(0),
        ).unwrap_or(0);
        out.push_str(&format!("Patterns: {}\n", patterns));

        out
    }
}

// ── Review Memories ──

struct ReviewMemoriesTool;

impl Tool for ReviewMemoriesTool {
    fn name(&self) -> &'static str { "review_memories" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "memory_hygiene" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "review_memories",
                "description": "List recent memories for manual review",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "domain": {
                            "type": "string",
                            "description": "Filter by domain (e.g. 'general', 'system/process', 'work', 'identity', 'preference')"
                        },
                        "source": {
                            "type": "string",
                            "description": "Filter by source: 'user', 'companion', 'self', 'system'"
                        },
                        "min_importance": {
                            "type": "number",
                            "description": "Only show memories with importance >= this value (0.0-1.0)"
                        },
                        "max_importance": {
                            "type": "number",
                            "description": "Only show memories with importance <= this value (0.0-1.0)"
                        },
                        "page": {
                            "type": "integer",
                            "description": "Page number (default 1, 20 per page)"
                        },
                        "sort": {
                            "type": "string",
                            "enum": ["newest", "oldest", "importance_desc", "importance_asc"],
                            "description": "Sort order (default newest)"
                        }
                    },
                    "required": []
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let domain = args.get("domain").and_then(|v| v.as_str());
        let source = args.get("source").and_then(|v| v.as_str());
        let min_imp = args.get("min_importance").and_then(|v| v.as_f64());
        let max_imp = args.get("max_importance").and_then(|v| v.as_f64());
        let page = args.get("page").and_then(|v| v.as_u64()).unwrap_or(1).max(1);
        let sort = args.get("sort").and_then(|v| v.as_str()).unwrap_or("newest");
        let per_page = 20u64;
        let offset = (page - 1) * per_page;

        let conn = ctx.db.conn();

        // Build dynamic query
        let mut conditions = vec![
            "(consolidation_status IS NULL OR consolidation_status = 'active')".to_string(),
        ];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(d) = domain {
            conditions.push(format!("domain = ?{}", params.len() + 1));
            params.push(Box::new(d.to_string()));
        }
        if let Some(s) = source {
            conditions.push(format!("source = ?{}", params.len() + 1));
            params.push(Box::new(s.to_string()));
        }
        if let Some(mi) = min_imp {
            conditions.push(format!("importance >= ?{}", params.len() + 1));
            params.push(Box::new(mi));
        }
        if let Some(ma) = max_imp {
            conditions.push(format!("importance <= ?{}", params.len() + 1));
            params.push(Box::new(ma));
        }

        let order = match sort {
            "oldest" => "created_at ASC",
            "importance_desc" => "importance DESC",
            "importance_asc" => "importance ASC",
            _ => "created_at DESC",
        };

        let where_clause = conditions.join(" AND ");

        // Get total count for this filter
        let count_sql = format!("SELECT COUNT(*) FROM memories WHERE {}", where_clause);
        let total: i64 = {
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            conn.query_row(&count_sql, param_refs.as_slice(), |r| r.get(0)).unwrap_or(0)
        };

        let total_pages = ((total as u64).max(1) + per_page - 1) / per_page;

        let sql = format!(
            "SELECT rid, text, domain, source, importance, storage_tier, created_at \
             FROM memories WHERE {} ORDER BY {} LIMIT {} OFFSET {}",
            where_clause, order, per_page, offset,
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let mut out = format!(
            "=== Memories (page {}/{}, {} total) ===\n\n",
            page, total_pages, total
        );

        match conn.prepare(&sql) {
            Ok(mut stmt) => {
                match stmt.query_map(param_refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,  // rid
                        row.get::<_, String>(1)?,  // text
                        row.get::<_, String>(2)?,  // domain
                        row.get::<_, String>(3)?,  // source
                        row.get::<_, f64>(4)?,     // importance
                        row.get::<_, String>(5)?,  // storage_tier
                        row.get::<_, f64>(6)?,     // created_at
                    ))
                }) {
                    Ok(rows) => {
                        for row in rows.flatten() {
                            let (rid, text, domain, source, imp, tier, _ts) = row;
                            // Truncate text for readability
                            let display = if text.len() > 100 {
                                format!("{}...", &text[..text.floor_char_boundary(97)])
                            } else {
                                text
                            };
                            // Short rid (first 8 chars)
                            let short_rid = if rid.len() > 12 { &rid[..rid.floor_char_boundary(12)] } else { &rid };
                            out.push_str(&format!(
                                "[{}] imp={:.1} tier={} src={} dom={}\n  {}\n\n",
                                short_rid, imp, tier, source, domain, display,
                            ));
                        }
                    }
                    Err(e) => {
                        out.push_str(&format!("Query error: {e}\n"));
                    }
                }
            }
            Err(e) => {
                out.push_str(&format!("Prepare error: {e}\n"));
            }
        }

        if page < total_pages {
            out.push_str(&format!("(Use page={} to see more)\n", page + 1));
        }

        out
    }
}

// ── Forget Memory ──

struct ForgetMemoryTool;

impl Tool for ForgetMemoryTool {
    fn name(&self) -> &'static str { "forget_memory" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory_hygiene" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "forget_memory",
                "description": "Delete one memory by ID; permanent",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "rid": {
                            "type": "string",
                            "description": "Memory ID (from review_memories). Can be the short 12-char prefix."
                        },
                        "reason": {
                            "type": "string",
                            "description": "Why you're forgetting this (logged for audit)"
                        }
                    },
                    "required": ["rid"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let rid_prefix = args.get("rid").and_then(|v| v.as_str()).unwrap_or_default();
        let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("self-curation");

        if rid_prefix.is_empty() {
            return "Error: rid is required".to_string();
        }

        let conn = ctx.db.conn();

        // Resolve short rid prefix to full rid
        let full_rid: Option<String> = conn.query_row(
            "SELECT rid FROM memories WHERE rid LIKE ?1 || '%' LIMIT 1",
            [rid_prefix],
            |r| r.get(0),
        ).ok();

        let full_rid = match full_rid {
            Some(r) => r,
            None => return format!("No memory found matching ID '{}'", rid_prefix),
        };

        // Soft-delete: set half_life to 1 second (tombstone pattern)
        let result = conn.execute(
            "UPDATE memories SET half_life = 1.0, consolidation_status = 'tombstoned' WHERE rid = ?1",
            [&full_rid],
        );

        match result {
            Ok(changed) if changed > 0 => {
                // Log the deletion
                let _ = conn.execute(
                    "INSERT INTO consolidation_log (original_rid, replacement_rid, consolidation_type, reason, created_at) \
                     VALUES (?1, '', 'forget', ?2, ?3)",
                    rusqlite::params![full_rid, reason, now_ts()],
                );
                format!("Forgotten: {} (reason: {})", &full_rid[..full_rid.floor_char_boundary(12.min(full_rid.len()))], reason)
            }
            _ => format!("Failed to forget memory '{}'", rid_prefix),
        }
    }
}

// ── Update Memory ──

struct UpdateMemoryTool;

impl Tool for UpdateMemoryTool {
    fn name(&self) -> &'static str { "update_memory" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory_hygiene" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "update_memory",
                "description": "Edit memory metadata, not content text",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "rid": {
                            "type": "string",
                            "description": "Memory ID (or short prefix from review_memories)"
                        },
                        "importance": {
                            "type": "number",
                            "description": "New importance (0.0-1.0)"
                        },
                        "domain": {
                            "type": "string",
                            "description": "New domain (e.g. 'general', 'work', 'identity', 'preference')"
                        },
                        "storage_tier": {
                            "type": "string",
                            "enum": ["hot", "warm", "cold"],
                            "description": "Storage tier: hot (always searchable), warm (slower), cold (archived)"
                        }
                    },
                    "required": ["rid"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let rid_prefix = args.get("rid").and_then(|v| v.as_str()).unwrap_or_default();
        if rid_prefix.is_empty() {
            return "Error: rid is required".to_string();
        }

        let conn = ctx.db.conn();

        // Resolve short rid
        let full_rid: Option<String> = conn.query_row(
            "SELECT rid FROM memories WHERE rid LIKE ?1 || '%' LIMIT 1",
            [rid_prefix],
            |r| r.get(0),
        ).ok();

        let full_rid = match full_rid {
            Some(r) => r,
            None => return format!("No memory found matching ID '{}'", rid_prefix),
        };

        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(imp) = args.get("importance").and_then(|v| v.as_f64()) {
            let imp = imp.clamp(0.0, 1.0);
            params.push(Box::new(imp));
            updates.push(format!("importance = ?{}", params.len()));
        }

        if let Some(domain) = args.get("domain").and_then(|v| v.as_str()) {
            params.push(Box::new(domain.to_string()));
            updates.push(format!("domain = ?{}", params.len()));
        }

        if let Some(tier) = args.get("storage_tier").and_then(|v| v.as_str()) {
            params.push(Box::new(tier.to_string()));
            updates.push(format!("storage_tier = ?{}", params.len()));
        }

        if updates.is_empty() {
            return "Nothing to update — specify importance, domain, or storage_tier".to_string();
        }

        params.push(Box::new(full_rid.clone()));
        let sql = format!(
            "UPDATE memories SET {} WHERE rid = ?{}",
            updates.join(", "),
            params.len(),
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        match conn.execute(&sql, param_refs.as_slice()) {
            Ok(1) => format!("Updated memory {}: {}", &full_rid[..full_rid.floor_char_boundary(12.min(full_rid.len()))], updates.join(", ")),
            Ok(_) => format!("No memory matched '{}'", rid_prefix),
            Err(e) => format!("Update failed: {e}"),
        }
    }
}

// ── Resolve Conflicts ──

struct ResolveConflictsTool;

impl Tool for ResolveConflictsTool {
    fn name(&self) -> &'static str { "resolve_conflicts" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory_hygiene" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "resolve_conflicts",
                "description": "Fix contradictory memories by merge or delete",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "resolve_all", "resolve_one"],
                            "description": "Action to take (default: list)"
                        },
                        "conflict_id": {
                            "type": "string",
                            "description": "Specific conflict ID to resolve (for resolve_one)"
                        }
                    },
                    "required": []
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let conn = ctx.db.conn();

        match action {
            "list" => {
                let mut out = String::from("=== Open Conflicts ===\n\n");
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT conflict_id, conflict_type, priority, detection_reason, \
                     memory_a, memory_b FROM conflicts \
                     WHERE status = 'open' ORDER BY detected_at DESC LIMIT 30"
                ) {
                    match stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    }) {
                        Ok(rows) => {
                            let mut count = 0;
                            for row in rows.flatten() {
                                let (id, ctype, priority, reason, mem_a, mem_b) = row;
                                let short_reason = if reason.len() > 80 {
                                    format!("{}...", &reason[..reason.floor_char_boundary(77)])
                                } else {
                                    reason
                                };
                                // Fetch memory texts for context
                                let text_a: String = conn.query_row(
                                    "SELECT COALESCE(substr(text,1,60), '?') FROM memories WHERE id = ?1",
                                    [&mem_a], |r| r.get(0),
                                ).unwrap_or_else(|_| "[deleted]".into());
                                let text_b: String = conn.query_row(
                                    "SELECT COALESCE(substr(text,1,60), '?') FROM memories WHERE id = ?1",
                                    [&mem_b], |r| r.get(0),
                                ).unwrap_or_else(|_| "[deleted]".into());
                                out.push_str(&format!(
                                    "[{}] type={} priority={}\n  {}\n  A: \"{}\"\n  B: \"{}\"\n\n",
                                    &id[..id.floor_char_boundary(8)], ctype, priority, short_reason, text_a, text_b,
                                ));
                                count += 1;
                            }
                            if count == 0 {
                                out.push_str("No open conflicts.\n");
                            }
                        }
                        Err(e) => out.push_str(&format!("Query error: {e}\n")),
                    }
                }
                out
            }
            "resolve_all" => {
                match conn.execute(
                    "UPDATE conflicts SET status = 'resolved' WHERE status = 'open'",
                    [],
                ) {
                    Ok(n) => format!("Resolved {} conflicts.", n),
                    Err(e) => format!("Failed to resolve conflicts: {e}"),
                }
            }
            "resolve_one" => {
                let conflict_id = args.get("conflict_id").and_then(|v| v.as_str()).unwrap_or_default();
                if conflict_id.is_empty() {
                    return "Error: conflict_id is required for resolve_one".to_string();
                }
                // Try matching by conflict_id prefix
                let result = conn.execute(
                    "UPDATE conflicts SET status = 'resolved' WHERE conflict_id LIKE ?1 || '%' AND status = 'open'",
                    [conflict_id],
                );

                match result {
                    Ok(n) if n > 0 => format!("Resolved conflict '{}'.", conflict_id),
                    _ => format!("No open conflict matching '{}'.", conflict_id),
                }
            }
            _ => "Unknown action. Use 'list', 'resolve_all', or 'resolve_one'.".to_string(),
        }
    }
}

// ── Purge System Noise ──

struct PurgeSystemNoiseTool;

impl Tool for PurgeSystemNoiseTool {
    fn name(&self) -> &'static str { "purge_system_noise" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "memory_hygiene" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "purge_system_noise",
                "description": "Remove low-value auto memories; not user facts",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "domains": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Domains to purge (default: ['system/process', 'system/cpu'])"
                        },
                        "older_than_hours": {
                            "type": "number",
                            "description": "Only purge memories older than this many hours (default: 24)"
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "If true, just count what would be purged without actually doing it (default: true)"
                        }
                    },
                    "required": []
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let domains: Vec<String> = args.get("domains")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_else(|| vec!["system/process".into(), "system/cpu".into()]);

        let older_hours = args.get("older_than_hours").and_then(|v| v.as_f64()).unwrap_or(24.0);
        let dry_run = args.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(true);

        let conn = ctx.db.conn();
        let cutoff_ts = now_ts() - (older_hours * 3600.0);

        let mut total = 0i64;
        let mut out = String::new();

        for domain in &domains {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE domain = ?1 AND created_at < ?2 \
                 AND (consolidation_status IS NULL OR consolidation_status = 'active')",
                rusqlite::params![domain, cutoff_ts],
                |r| r.get(0),
            ).unwrap_or(0);

            out.push_str(&format!("  {}: {} memories\n", domain, count));
            total += count;
        }

        if dry_run {
            return format!(
                "DRY RUN — would purge {} memories (older than {:.0}h):\n{}\nSet dry_run=false to execute.",
                total, older_hours, out,
            );
        }

        // Actually purge: tombstone them
        let mut purged = 0i64;
        for domain in &domains {
            let n = conn.execute(
                "UPDATE memories SET half_life = 1.0, consolidation_status = 'tombstoned' \
                 WHERE domain = ?1 AND created_at < ?2 \
                 AND (consolidation_status IS NULL OR consolidation_status = 'active')",
                rusqlite::params![domain, cutoff_ts],
            ).unwrap_or(0) as i64;
            purged += n;
        }

        // Log the purge
        let _ = conn.execute(
            "INSERT INTO consolidation_log (original_rid, replacement_rid, consolidation_type, reason, created_at) \
             VALUES ('batch_purge', '', 'purge', ?1, ?2)",
            rusqlite::params![
                format!("Self-curation: purged {} system noise memories from domains {:?}", purged, domains),
                now_ts(),
            ],
        );

        format!(
            "Purged {} memories (older than {:.0}h):\n{}",
            purged, older_hours, out,
        )
    }
}
