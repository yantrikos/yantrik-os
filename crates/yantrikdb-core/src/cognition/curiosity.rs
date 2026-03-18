//! Curiosity Engine — information-seeking driven by homeostatic hunger.
//!
//! When the brain's information_hunger or novelty_hunger drives are high, the
//! curiosity engine determines WHAT to fetch and prioritizes requests. It doesn't
//! perform the fetches itself (that's the companion's job via tools) — it emits
//! BrainCandidates with `CandidateSource::ExternalFetch` and appropriate context
//! for the companion to execute.
//!
//! # Two Modes
//!
//! **Diversive curiosity** (novelty_hunger > 0.6): "I'm bored, show me something new"
//! - Refresh RSS feeds for tracked topics
//! - Fetch trending news in user's interest areas
//! - Check local events calendar
//! - Explore adjacent topics to user interests
//!
//! **Specific curiosity** (Uncertainty signals with high salience): "I need THIS data"
//! - Refresh weather before outdoor events
//! - Check package tracking status
//! - Update financial data for watched assets
//! - Fetch traffic before commute windows
//!
//! # Fetch Budget
//!
//! Each fetch has a `cost` (network overhead + processing time). The engine
//! has a per-tick budget to prevent spamming external APIs. Sources that yield
//! low-value results get backed off exponentially.

use rusqlite::Connection;

use serde::{Deserialize, Serialize};

use super::brain::{BrainCandidate, CandidateSource, Homeostasis, SignalType};

// ══════════════════════════════════════════════════════════════════════════════
// § 1  Schema
// ══════════════════════════════════════════════════════════════════════════════

/// Ensure curiosity tables exist.
pub fn ensure_curiosity_tables(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS curiosity_sources (
            source_id       TEXT PRIMARY KEY,
            source_type     TEXT NOT NULL,
            label           TEXT NOT NULL,
            url_or_key      TEXT NOT NULL DEFAULT '',
            ttl_secs        REAL NOT NULL DEFAULT 3600.0,
            cost            REAL NOT NULL DEFAULT 1.0,
            last_fetched_at REAL NOT NULL DEFAULT 0.0,
            last_yield      REAL NOT NULL DEFAULT 0.5,
            backoff_until   REAL NOT NULL DEFAULT 0.0,
            fetch_count     INTEGER NOT NULL DEFAULT 0,
            yield_ema       REAL NOT NULL DEFAULT 0.5,
            enabled         INTEGER NOT NULL DEFAULT 1,
            created_at      REAL NOT NULL,
            updated_at      REAL NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_curiosity_type
            ON curiosity_sources(source_type);
        ",
    )
    .ok();
}

// ══════════════════════════════════════════════════════════════════════════════
// § 2  Source Types
// ══════════════════════════════════════════════════════════════════════════════

/// Types of external data sources the curiosity engine can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchType {
    /// Weather forecast (uses weather tool or API).
    Weather,
    /// RSS/Atom feed.
    RssFeed,
    /// News headlines for a topic.
    NewsHeadlines,
    /// Local events (community calendar, meetups).
    LocalEvents,
    /// Package tracking status.
    PackageTracking,
    /// Financial data (stock price, crypto, etc.).
    Financial,
    /// Traffic/commute conditions.
    Traffic,
    /// Generic URL content refresh.
    WebContent,
}

impl FetchType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weather => "weather",
            Self::RssFeed => "rss_feed",
            Self::NewsHeadlines => "news",
            Self::LocalEvents => "local_events",
            Self::PackageTracking => "package_tracking",
            Self::Financial => "financial",
            Self::Traffic => "traffic",
            Self::WebContent => "web_content",
        }
    }

    /// Default TTL for each source type.
    pub fn default_ttl_secs(self) -> f64 {
        match self {
            Self::Weather => 3600.0,         // 1 hour
            Self::RssFeed => 1800.0,         // 30 minutes
            Self::NewsHeadlines => 3600.0,   // 1 hour
            Self::LocalEvents => 21600.0,    // 6 hours
            Self::PackageTracking => 7200.0, // 2 hours
            Self::Financial => 900.0,        // 15 minutes
            Self::Traffic => 600.0,          // 10 minutes
            Self::WebContent => 7200.0,      // 2 hours
        }
    }

    /// Base cost of a fetch (higher = more expensive).
    pub fn base_cost(self) -> f64 {
        match self {
            Self::Weather => 1.0,
            Self::RssFeed => 0.5,
            Self::NewsHeadlines => 1.5,
            Self::LocalEvents => 2.0,
            Self::PackageTracking => 0.5,
            Self::Financial => 0.5,
            Self::Traffic => 1.0,
            Self::WebContent => 2.0,
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 3  Curiosity Planner
// ══════════════════════════════════════════════════════════════════════════════

/// A planned fetch request — what the companion should execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRequest {
    pub source_id: String,
    pub fetch_type: String,
    pub label: String,
    pub url_or_key: String,
    pub priority: f64,
}

/// Plan fetch requests based on current drives and source staleness.
/// Returns BrainCandidates with ExternalFetch source for the brain to score.
pub fn plan_fetches(
    conn: &Connection,
    drives: &Homeostasis,
    now: f64,
    budget: f64,
) -> Vec<BrainCandidate> {
    let mut candidates = Vec::new();

    // Only plan fetches when drives warrant it
    let fetch_drive = drives.information_hunger.max(drives.novelty_hunger * 0.7);
    if fetch_drive < 0.35 {
        return candidates;
    }

    let mut stmt = match conn.prepare(
        "SELECT source_id, source_type, label, url_or_key, ttl_secs, cost,
                last_fetched_at, yield_ema, backoff_until
         FROM curiosity_sources
         WHERE enabled = 1
           AND backoff_until < ?1
         ORDER BY yield_ema DESC, last_fetched_at ASC
         LIMIT 50"
    ) {
        Ok(s) => s,
        Err(_) => return candidates,
    };

    struct SourceRow {
        source_id: String,
        source_type: String,
        label: String,
        url_or_key: String,
        ttl_secs: f64,
        cost: f64,
        last_fetched_at: f64,
        yield_ema: f64,
    }

    let rows: Vec<SourceRow> = stmt
        .query_map(rusqlite::params![now], |row| {
            Ok(SourceRow {
                source_id: row.get(0)?,
                source_type: row.get(1)?,
                label: row.get(2)?,
                url_or_key: row.get(3)?,
                ttl_secs: row.get(4)?,
                cost: row.get(5)?,
                last_fetched_at: row.get(6)?,
                yield_ema: row.get(7)?,
            })
        })
        .ok()
        .map(|r| r.flatten().collect())
        .unwrap_or_default();

    let mut remaining_budget = budget;

    for row in &rows {
        if remaining_budget <= 0.0 {
            break;
        }

        let staleness = now - row.last_fetched_at;
        if staleness < row.ttl_secs {
            continue; // not stale yet
        }

        // Priority = (staleness / ttl) × yield_ema × fetch_drive / cost
        let staleness_factor = (staleness / row.ttl_secs).min(5.0);
        let priority = staleness_factor * row.yield_ema * fetch_drive / row.cost.max(0.1);

        if priority < 0.2 {
            continue;
        }

        remaining_budget -= row.cost;

        let is_diversive = drives.novelty_hunger > drives.information_hunger;
        let signal_type = if is_diversive {
            SignalType::Uncertainty // "I want to know" → triggers fetch
        } else {
            SignalType::Uncertainty // specific data need
        };

        candidates.push(BrainCandidate {
            candidate_id: format!("curiosity:{}:{:.0}", row.source_id, now),
            source: CandidateSource::ExternalFetch {
                fetch_type: row.source_type.clone(),
            },
            signal_type,
            raw_urgency: priority.clamp(0.0, 1.0),
            brain_score: 0.0,
            reason: format!(
                "Data stale: {} last fetched {} ago (TTL: {})",
                row.label,
                humanize_secs(staleness),
                humanize_secs(row.ttl_secs),
            ),
            suggested_message: format!("Checking for updates on {}", row.label),
            action: Some(format!(
                "fetch:{}:{}",
                row.source_type, row.source_id,
            )),
            context: serde_json::json!({
                "fetch_request": {
                    "source_id": row.source_id,
                    "fetch_type": row.source_type,
                    "label": row.label,
                    "url_or_key": row.url_or_key,
                    "priority": priority,
                },
                "staleness_secs": staleness,
                "yield_ema": row.yield_ema,
                "diversive": is_diversive,
            }),
            cooldown_key: format!("fetch:{}", row.source_id),
            orientation: None,
            created_at: now,
        });
    }

    candidates
}

// ══════════════════════════════════════════════════════════════════════════════
// § 4  Source Management
// ══════════════════════════════════════════════════════════════════════════════

/// Register a new curiosity source.
pub fn register_source(
    conn: &Connection,
    source_id: &str,
    source_type: &str,
    label: &str,
    url_or_key: &str,
    ttl_secs: f64,
    cost: f64,
) {
    let now = now_ts();
    conn.execute(
        "INSERT INTO curiosity_sources
         (source_id, source_type, label, url_or_key, ttl_secs, cost, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(source_id) DO UPDATE SET
           label = ?3, url_or_key = ?4, ttl_secs = ?5, cost = ?6, updated_at = ?7",
        rusqlite::params![source_id, source_type, label, url_or_key, ttl_secs, cost, now],
    )
    .ok();
}

/// Record the result of a fetch. Updates yield EMA and backoff.
///
/// - `yield_score`: 0.0 (nothing useful) to 1.0 (highly valuable data returned)
pub fn record_fetch_result(
    conn: &Connection,
    source_id: &str,
    yield_score: f64,
) {
    let now = now_ts();
    let alpha = 0.2; // EMA smoothing

    // Get current EMA
    let current_ema: f64 = conn
        .query_row(
            "SELECT yield_ema FROM curiosity_sources WHERE source_id = ?1",
            rusqlite::params![source_id],
            |row| row.get(0),
        )
        .unwrap_or(0.5);

    let new_ema = current_ema * (1.0 - alpha) + yield_score * alpha;

    // Backoff: if yield is consistently low, add exponential backoff
    let backoff_until = if new_ema < 0.15 {
        now + 86400.0 // 24h backoff for very low yield
    } else if new_ema < 0.25 {
        now + 7200.0 // 2h backoff for low yield
    } else {
        0.0 // no backoff
    };

    conn.execute(
        "UPDATE curiosity_sources
         SET last_fetched_at = ?1, yield_ema = ?2, backoff_until = ?3,
             last_yield = ?4, fetch_count = fetch_count + 1, updated_at = ?1
         WHERE source_id = ?5",
        rusqlite::params![now, new_ema, backoff_until, yield_score, source_id],
    )
    .ok();
}

/// Seed default curiosity sources based on user interests and location.
/// Called once during setup or when interests change.
pub fn seed_default_sources(
    conn: &Connection,
    user_interests: &[String],
    user_location: &str,
) {
    // Weather for user's location
    if !user_location.is_empty() {
        register_source(
            conn,
            &format!("weather:{}", user_location),
            "weather",
            &format!("Weather in {}", user_location),
            user_location,
            3600.0,
            1.0,
        );
    }

    // News for each interest
    for interest in user_interests {
        let id = format!("news:{}", interest.to_lowercase().replace(' ', "_"));
        register_source(
            conn,
            &id,
            "news",
            &format!("{} news", interest),
            interest,
            3600.0,
            1.5,
        );
    }
}

/// Get curiosity engine statistics.
pub fn curiosity_stats(conn: &Connection) -> serde_json::Value {
    let total_sources: i64 = conn
        .query_row("SELECT COUNT(*) FROM curiosity_sources WHERE enabled = 1", [], |r| r.get(0))
        .unwrap_or(0);

    let total_fetches: i64 = conn
        .query_row("SELECT COALESCE(SUM(fetch_count), 0) FROM curiosity_sources", [], |r| r.get(0))
        .unwrap_or(0);

    let avg_yield: f64 = conn
        .query_row("SELECT COALESCE(AVG(yield_ema), 0) FROM curiosity_sources WHERE enabled = 1", [], |r| r.get(0))
        .unwrap_or(0.0);

    let backed_off: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM curiosity_sources WHERE backoff_until > ?1",
            rusqlite::params![now_ts()],
            |r| r.get(0),
        )
        .unwrap_or(0);

    serde_json::json!({
        "total_sources": total_sources,
        "total_fetches": total_fetches,
        "avg_yield_ema": avg_yield,
        "backed_off_sources": backed_off,
    })
}

// ══════════════════════════════════════════════════════════════════════════════
// § 5  Helpers
// ══════════════════════════════════════════════════════════════════════════════

fn humanize_secs(secs: f64) -> String {
    if secs < 60.0 {
        format!("{:.0}s", secs)
    } else if secs < 3600.0 {
        format!("{:.0}m", secs / 60.0)
    } else if secs < 86400.0 {
        format!("{:.1}h", secs / 3600.0)
    } else {
        format!("{:.1}d", secs / 86400.0)
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ══════════════════════════════════════════════════════════════════════════════
// § 6  Tests
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_curiosity_tables(&conn);
        conn
    }

    #[test]
    fn test_register_source() {
        let conn = setup_db();
        register_source(&conn, "weather:london", "weather", "London Weather", "London", 3600.0, 1.0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM curiosity_sources", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_no_fetch_when_drives_low() {
        let conn = setup_db();
        register_source(&conn, "weather:london", "weather", "London Weather", "London", 3600.0, 1.0);

        let drives = Homeostasis {
            information_hunger: 0.1,
            novelty_hunger: 0.1,
            ..Default::default()
        };

        let candidates = plan_fetches(&conn, &drives, 1_000_000.0, 10.0);
        assert!(candidates.is_empty(), "Low drives should not trigger fetches");
    }

    #[test]
    fn test_fetch_when_hungry_and_stale() {
        let conn = setup_db();
        register_source(&conn, "weather:london", "weather", "London Weather", "London", 3600.0, 1.0);

        // Make it stale: last fetched 2 hours ago, TTL is 1 hour
        let now = 1_000_000.0;
        conn.execute(
            "UPDATE curiosity_sources SET last_fetched_at = ?1",
            rusqlite::params![now - 7200.0],
        ).unwrap();

        let drives = Homeostasis {
            information_hunger: 0.8,
            novelty_hunger: 0.5,
            ..Default::default()
        };

        let candidates = plan_fetches(&conn, &drives, now, 10.0);
        assert!(!candidates.is_empty(), "High hunger + stale source should trigger fetch");
    }

    #[test]
    fn test_fetch_result_backoff() {
        let conn = setup_db();
        register_source(&conn, "bad_source", "news", "Bad Source", "nothing", 3600.0, 1.0);

        // Record many low-yield results
        for _ in 0..10 {
            record_fetch_result(&conn, "bad_source", 0.0);
        }

        let ema: f64 = conn
            .query_row(
                "SELECT yield_ema FROM curiosity_sources WHERE source_id = 'bad_source'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(ema < 0.15, "EMA should be very low after many zero yields: {ema}");

        let backoff: f64 = conn
            .query_row(
                "SELECT backoff_until FROM curiosity_sources WHERE source_id = 'bad_source'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(backoff > 0.0, "Should be backed off");
    }

    #[test]
    fn test_budget_limiting() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Register 10 sources, all stale
        for i in 0..10 {
            register_source(&conn, &format!("src_{i}"), "news", &format!("Source {i}"), "", 3600.0, 3.0);
        }
        conn.execute(
            "UPDATE curiosity_sources SET last_fetched_at = ?1",
            rusqlite::params![now - 7200.0],
        ).unwrap();

        let drives = Homeostasis {
            information_hunger: 0.9,
            ..Default::default()
        };

        // Budget of 5.0 with cost 3.0 each → max ~1 fetch
        let candidates = plan_fetches(&conn, &drives, now, 5.0);
        assert!(candidates.len() <= 2, "Budget should limit fetches: got {}", candidates.len());
    }

    #[test]
    fn test_seed_defaults() {
        let conn = setup_db();
        seed_default_sources(&conn, &["Rust programming".to_string(), "AI".to_string()], "Hyderabad");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM curiosity_sources", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3); // 1 weather + 2 news
    }
}
