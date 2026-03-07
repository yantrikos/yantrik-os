//! Adaptive User Model — learns each user's interaction patterns to dynamically
//! adjust proactive message frequency, category preferences, and timing.
//!
//! # Design
//!
//! Dual-timescale Beta engagement tracking:
//! - **Fast** (τ=2d): reacts to recent behavior (busy afternoon, active evening)
//! - **Slow** (τ=14d): captures stable baseline engagement level
//!
//! Per-category preferences with hierarchical shrinkage (k=3 pseudo-observations
//! blending toward global mean). Time-of-day receptivity in 6 × 4h slots.
//!
//! Budget as a single volume knob: `mult = clamp(0.6 + 0.8 * engagement, 0.6, 1.3)`
//!
//! All state persisted to SQLite. Lazy exponential decay on read (no background jobs).

use rusqlite::Connection;
use std::sync::Mutex;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Fast engagement decay half-life (seconds). τ = 2 days.
const FAST_TAU: f64 = 2.0 * 86400.0;
/// Slow engagement decay half-life (seconds). τ = 14 days.
const SLOW_TAU: f64 = 14.0 * 86400.0;

/// Graded reward thresholds (seconds).
const REWARD_FULL_THRESHOLD: f64 = 30.0 * 60.0;  // ≤30 min → 1.0
const REWARD_HALF_THRESHOLD: f64 = 4.0 * 3600.0;  // ≤4h → 0.5

/// Max consecutive ignores for backoff calculation.
const MAX_IGNORE_STREAK: u32 = 5;

/// Hierarchical shrinkage pseudo-observation count.
const SHRINKAGE_K: f64 = 3.0;

/// Number of time-of-day slots (4h each).
const TIME_SLOTS: usize = 6;

/// Budget bounds.
const MIN_BUDGET: u32 = 2;
const MAX_BUDGET: u32 = 20;

/// Activity frequency EMA smoothing factor.
const ACTIVITY_EMA_ALPHA: f64 = 0.1;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Per-category engagement stats (Beta distribution sufficient statistics).
#[derive(Debug, Clone)]
struct CategoryStats {
    alpha: f64,  // successes + prior
    beta: f64,   // failures + prior
    last_update_ts: f64,
}

impl Default for CategoryStats {
    fn default() -> Self {
        Self { alpha: 1.0, beta: 1.0, last_update_ts: 0.0 }
    }
}

impl CategoryStats {
    fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Apply exponential decay to move α,β toward prior (1,1).
    fn decay(&mut self, dt_secs: f64, tau: f64) {
        if dt_secs <= 0.0 || tau <= 0.0 {
            return;
        }
        let factor = (-dt_secs / tau).exp();
        // Decay excess above prior toward 0
        self.alpha = 1.0 + (self.alpha - 1.0) * factor;
        self.beta = 1.0 + (self.beta - 1.0) * factor;
    }
}

/// Time-of-day receptivity slot.
#[derive(Debug, Clone)]
struct TimeSlot {
    alpha: f64,
    beta: f64,
}

impl Default for TimeSlot {
    fn default() -> Self {
        Self { alpha: 1.0, beta: 1.0 }
    }
}

impl TimeSlot {
    fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }
}

// ─── UserModel ──────────────────────────────────────────────────────────────

pub struct UserModel {
    inner: Mutex<UserModelInner>,
}

struct UserModelInner {
    // ── Global Engagement (dual-timescale Beta) ──
    fast_alpha: f64,
    fast_beta: f64,
    slow_alpha: f64,
    slow_beta: f64,

    // ── Category Preferences ──
    /// Maps instinct category name → stats.
    categories: std::collections::HashMap<String, CategoryStats>,

    // ── Time-of-day Receptivity ──
    /// 6 slots: 0=00-04, 1=04-08, 2=08-12, 3=12-16, 4=16-20, 5=20-24
    time_slots: [TimeSlot; TIME_SLOTS],

    // ── Ignore Streak ──
    ignore_streak: u32,

    // ── Activity Frequency ──
    /// EMA of inter-message intervals (seconds).
    activity_interval_ema: f64,

    // ── Timestamps ──
    last_proactive_ts: f64,
    last_user_ts: f64,
    last_decay_ts: f64,

    // ── Pending proactive tracking ──
    /// (instinct_name, category, sent_ts) — awaiting user response.
    pending_proactive: Option<(String, String, f64)>,
}

impl UserModel {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(UserModelInner {
                // Warm prior: biased toward engagement (α=2, β=1)
                fast_alpha: 2.0,
                fast_beta: 1.0,
                slow_alpha: 2.0,
                slow_beta: 1.0,
                categories: std::collections::HashMap::new(),
                time_slots: Default::default(),
                ignore_streak: 0,
                activity_interval_ema: 3600.0, // 1h default
                last_proactive_ts: 0.0,
                last_user_ts: 0.0,
                last_decay_ts: 0.0,
                pending_proactive: None,
            }),
        }
    }

    // ─── Event Hooks ────────────────────────────────────────────────────

    /// Called when a proactive message is sent to the user.
    pub fn on_proactive_sent(&self, instinct_name: &str, category: &str, now_ts: f64) {
        let mut m = self.inner.lock().unwrap();
        m.pending_proactive = Some((
            instinct_name.to_string(),
            category.to_string(),
            now_ts,
        ));
        m.last_proactive_ts = now_ts;
    }

    /// Called when the user sends a message (any message, not just replies).
    pub fn on_user_message(&self, now_ts: f64) {
        let mut m = self.inner.lock().unwrap();

        // Apply lazy decay first
        apply_decay(&mut m, now_ts);

        // Update activity frequency EMA
        if m.last_user_ts > 0.0 {
            let interval = (now_ts - m.last_user_ts).max(1.0);
            m.activity_interval_ema = (1.0 - ACTIVITY_EMA_ALPHA) * m.activity_interval_ema
                + ACTIVITY_EMA_ALPHA * interval;
        }

        // Check if this is a response to a pending proactive message
        if let Some((instinct_name, category, sent_ts)) = m.pending_proactive.take() {
            let delay = now_ts - sent_ts;
            let reward = if delay <= REWARD_FULL_THRESHOLD {
                1.0
            } else if delay <= REWARD_HALF_THRESHOLD {
                0.5
            } else {
                0.0
            };

            // Update global engagement
            m.fast_alpha += reward;
            m.fast_beta += 1.0 - reward;
            m.slow_alpha += reward;
            m.slow_beta += 1.0 - reward;

            // Update category stats
            let cat = m.categories.entry(category.clone()).or_default();
            cat.alpha += reward;
            cat.beta += 1.0 - reward;
            cat.last_update_ts = now_ts;

            // Update time-of-day slot
            let hour = time_hour_from_ts(sent_ts);
            let slot = (hour / 4) as usize;
            if slot < TIME_SLOTS {
                m.time_slots[slot].alpha += reward;
                m.time_slots[slot].beta += 1.0 - reward;
            }

            // Reset ignore streak on any response
            if reward > 0.0 {
                m.ignore_streak = 0;
            }

            tracing::debug!(
                instinct = %instinct_name,
                category = %category,
                delay_min = delay / 60.0,
                reward = reward,
                engagement_fast = m.fast_alpha / (m.fast_alpha + m.fast_beta),
                engagement_slow = m.slow_alpha / (m.slow_alpha + m.slow_beta),
                "UserModel: recorded proactive response"
            );
        }

        m.last_user_ts = now_ts;
    }

    /// Called when a proactive message expires without response.
    /// Typically after ~2h of silence following a proactive send.
    pub fn on_proactive_ignored(&self, now_ts: f64) {
        let mut m = self.inner.lock().unwrap();
        apply_decay(&mut m, now_ts);

        if let Some((_instinct_name, category, sent_ts)) = m.pending_proactive.take() {
            // Record as ignored (reward = 0)
            m.fast_alpha += 0.0;
            m.fast_beta += 1.0;
            m.slow_alpha += 0.0;
            m.slow_beta += 1.0;

            let cat = m.categories.entry(category).or_default();
            cat.beta += 1.0;
            cat.last_update_ts = now_ts;

            let hour = time_hour_from_ts(sent_ts);
            let slot = (hour / 4) as usize;
            if slot < TIME_SLOTS {
                m.time_slots[slot].beta += 1.0;
            }

            m.ignore_streak += 1;
        }
    }

    // ─── Computed Multipliers ───────────────────────────────────────────

    /// Global engagement score (0..1). Blends fast (70%) and slow (30%).
    pub fn engagement(&self) -> f64 {
        let m = self.inner.lock().unwrap();
        let fast = m.fast_alpha / (m.fast_alpha + m.fast_beta);
        let slow = m.slow_alpha / (m.slow_alpha + m.slow_beta);
        0.7 * fast + 0.3 * slow
    }

    /// Category preference multiplier for a given category.
    /// Uses hierarchical shrinkage: blends per-category mean with global mean.
    /// Returns a value in [0.5, 1.5].
    pub fn category_preference(&self, category: &str) -> f64 {
        let m = self.inner.lock().unwrap();
        let global_mean = m.fast_alpha / (m.fast_alpha + m.fast_beta);

        let cat_mean = if let Some(cat) = m.categories.get(category) {
            let n = (cat.alpha - 1.0) + (cat.beta - 1.0); // observations (minus prior)
            if n < 0.5 {
                global_mean // No data, use global
            } else {
                // Hierarchical shrinkage: blend toward global with k pseudo-observations
                let shrunk = (cat.mean() * n + global_mean * SHRINKAGE_K) / (n + SHRINKAGE_K);
                shrunk
            }
        } else {
            global_mean
        };

        // Ratio relative to global, clamped
        let ratio = if global_mean > 0.01 {
            cat_mean / global_mean
        } else {
            1.0
        };
        ratio.clamp(0.5, 1.5)
    }

    /// Time-of-day receptivity multiplier for a given hour (0-23).
    /// Returns a value in [0.5, 1.5].
    pub fn time_receptivity(&self, hour: u32) -> f64 {
        let m = self.inner.lock().unwrap();
        let slot = (hour / 4) as usize;
        if slot >= TIME_SLOTS {
            return 1.0;
        }

        let slot_mean = m.time_slots[slot].mean();

        // Compute global time mean (average across all slots)
        let global_time_mean: f64 = m.time_slots.iter()
            .map(|s| s.mean())
            .sum::<f64>() / TIME_SLOTS as f64;

        let ratio = if global_time_mean > 0.01 {
            slot_mean / global_time_mean
        } else {
            1.0
        };
        ratio.clamp(0.5, 1.5)
    }

    /// Ignore streak backoff multiplier: 0.5^min(streak, 5).
    /// Returns 1.0 (no backoff) to 0.03125 (5+ ignores).
    pub fn backoff_multiplier(&self) -> f64 {
        let m = self.inner.lock().unwrap();
        let streak = m.ignore_streak.min(MAX_IGNORE_STREAK);
        0.5_f64.powi(streak as i32)
    }

    /// Effective daily budget multiplier based on engagement.
    /// Returns a multiplier in [0.6, 1.3] to apply to the bond-scaled base budget.
    pub fn budget_multiplier(&self) -> f64 {
        let eng = self.engagement();
        (0.6 + 0.8 * eng).clamp(0.6, 1.3)
    }

    /// Adaptive desire pressure lambda — users who interact more frequently
    /// should get faster desire buildup.
    pub fn adaptive_desire_lambda(&self) -> f64 {
        let m = self.inner.lock().unwrap();
        // Base lambda scaled by activity frequency
        // More active user (shorter intervals) → higher lambda → faster desire buildup
        let base_lambda = 0.0003; // Same as DESIRE_DECAY_RATE
        let freq_factor = (3600.0 / m.activity_interval_ema.max(60.0)).clamp(0.5, 3.0);
        base_lambda * freq_factor
    }

    /// Compute the overall urgency multiplier for a candidate proactive message.
    /// This combines all adaptive factors into a single scalar.
    pub fn urgency_multiplier(&self, category: &str, hour: u32) -> f64 {
        let cat = self.category_preference(category);
        let time = self.time_receptivity(hour);
        let backoff = self.backoff_multiplier();
        let budget = self.budget_multiplier();

        let mult = cat * time * backoff * budget;

        tracing::debug!(
            category = %category,
            hour = hour,
            cat_pref = %format!("{:.3}", cat),
            time_recept = %format!("{:.3}", time),
            backoff = %format!("{:.3}", backoff),
            budget_mult = %format!("{:.3}", budget),
            combined = %format!("{:.4}", mult),
            "UserModel urgency multiplier"
        );

        mult
    }

    /// Get the timestamp of the pending proactive message (if any).
    /// Used by bridge.rs to detect ignored messages after 2h timeout.
    pub fn inner_pending_ts(&self) -> Option<f64> {
        let m = self.inner.lock().unwrap();
        m.pending_proactive.as_ref().map(|(_, _, ts)| *ts)
    }

    // ─── SQLite Persistence ─────────────────────────────────────────────

    /// Initialize the user_model table.
    pub fn init_db(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_model (
                key TEXT PRIMARY KEY,
                value REAL NOT NULL,
                updated_at REAL NOT NULL DEFAULT 0
            );"
        ).ok();

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_model_categories (
                category TEXT PRIMARY KEY,
                alpha REAL NOT NULL DEFAULT 1.0,
                beta REAL NOT NULL DEFAULT 1.0,
                last_update_ts REAL NOT NULL DEFAULT 0
            );"
        ).ok();

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_model_time_slots (
                slot INTEGER PRIMARY KEY,
                alpha REAL NOT NULL DEFAULT 1.0,
                beta REAL NOT NULL DEFAULT 1.0
            );"
        ).ok();
    }

    /// Save current state to SQLite.
    pub fn save(&self, conn: &Connection) {
        let m = self.inner.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let upsert = |key: &str, val: f64| {
            conn.execute(
                "INSERT INTO user_model (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
                rusqlite::params![key, val, now],
            ).ok();
        };

        upsert("fast_alpha", m.fast_alpha);
        upsert("fast_beta", m.fast_beta);
        upsert("slow_alpha", m.slow_alpha);
        upsert("slow_beta", m.slow_beta);
        upsert("ignore_streak", m.ignore_streak as f64);
        upsert("activity_interval_ema", m.activity_interval_ema);
        upsert("last_proactive_ts", m.last_proactive_ts);
        upsert("last_user_ts", m.last_user_ts);
        upsert("last_decay_ts", m.last_decay_ts);

        // Categories
        for (cat, stats) in &m.categories {
            conn.execute(
                "INSERT INTO user_model_categories (category, alpha, beta, last_update_ts)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(category) DO UPDATE SET alpha = ?2, beta = ?3, last_update_ts = ?4",
                rusqlite::params![cat, stats.alpha, stats.beta, stats.last_update_ts],
            ).ok();
        }

        // Time slots
        for (i, slot) in m.time_slots.iter().enumerate() {
            conn.execute(
                "INSERT INTO user_model_time_slots (slot, alpha, beta)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(slot) DO UPDATE SET alpha = ?2, beta = ?3",
                rusqlite::params![i as i64, slot.alpha, slot.beta],
            ).ok();
        }
    }

    /// Load state from SQLite. Returns a new UserModel with loaded state,
    /// or default if no saved state exists.
    pub fn load(conn: &Connection) -> Self {
        let model = Self::new();
        {
            let mut m = model.inner.lock().unwrap();

            let get = |key: &str| -> Option<f64> {
                conn.query_row(
                    "SELECT value FROM user_model WHERE key = ?1",
                    rusqlite::params![key],
                    |row| row.get(0),
                ).ok()
            };

            if let Some(v) = get("fast_alpha") { m.fast_alpha = v; }
            if let Some(v) = get("fast_beta") { m.fast_beta = v; }
            if let Some(v) = get("slow_alpha") { m.slow_alpha = v; }
            if let Some(v) = get("slow_beta") { m.slow_beta = v; }
            if let Some(v) = get("ignore_streak") { m.ignore_streak = v as u32; }
            if let Some(v) = get("activity_interval_ema") { m.activity_interval_ema = v; }
            if let Some(v) = get("last_proactive_ts") { m.last_proactive_ts = v; }
            if let Some(v) = get("last_user_ts") { m.last_user_ts = v; }
            if let Some(v) = get("last_decay_ts") { m.last_decay_ts = v; }

            // Load categories
            if let Ok(mut stmt) = conn.prepare(
                "SELECT category, alpha, beta, last_update_ts FROM user_model_categories"
            ) {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, f64>(3)?,
                    ))
                }) {
                    for row in rows.flatten() {
                        m.categories.insert(row.0, CategoryStats {
                            alpha: row.1,
                            beta: row.2,
                            last_update_ts: row.3,
                        });
                    }
                }
            }

            // Load time slots
            if let Ok(mut stmt) = conn.prepare(
                "SELECT slot, alpha, beta FROM user_model_time_slots"
            ) {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                }) {
                    for row in rows.flatten() {
                        let idx = row.0 as usize;
                        if idx < TIME_SLOTS {
                            m.time_slots[idx] = TimeSlot { alpha: row.1, beta: row.2 };
                        }
                    }
                }
            }
        }
        model
    }

    /// Get a diagnostic summary of the current adaptive state.
    pub fn diagnostic_summary(&self) -> String {
        let m = self.inner.lock().unwrap();
        let fast_eng = m.fast_alpha / (m.fast_alpha + m.fast_beta);
        let slow_eng = m.slow_alpha / (m.slow_alpha + m.slow_beta);
        let blended = 0.7 * fast_eng + 0.3 * slow_eng;

        let mut cats: Vec<_> = m.categories.iter()
            .map(|(k, v)| format!("{}={:.2}", k, v.mean()))
            .collect();
        cats.sort();

        let time_str: Vec<_> = m.time_slots.iter().enumerate()
            .map(|(i, s)| format!("{}h-{}h={:.2}", i * 4, (i + 1) * 4, s.mean()))
            .collect();

        format!(
            "Engagement: fast={:.3} slow={:.3} blended={:.3} | \
             Ignore streak: {} | Activity EMA: {:.0}s | \
             Categories: [{}] | Time slots: [{}]",
            fast_eng, slow_eng, blended,
            m.ignore_streak,
            m.activity_interval_ema,
            cats.join(", "),
            time_str.join(", "),
        )
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Apply lazy exponential decay to all engagement stats.
fn apply_decay(m: &mut UserModelInner, now_ts: f64) {
    if m.last_decay_ts <= 0.0 {
        m.last_decay_ts = now_ts;
        return;
    }
    let dt = now_ts - m.last_decay_ts;
    if dt < 60.0 {
        return; // Don't decay more than once per minute
    }

    // Decay global engagement
    let fast_factor = (-dt / FAST_TAU).exp();
    m.fast_alpha = 1.0 + (m.fast_alpha - 1.0) * fast_factor;
    m.fast_beta = 1.0 + (m.fast_beta - 1.0) * fast_factor;

    let slow_factor = (-dt / SLOW_TAU).exp();
    m.slow_alpha = 1.0 + (m.slow_alpha - 1.0) * slow_factor;
    m.slow_beta = 1.0 + (m.slow_beta - 1.0) * slow_factor;

    // Decay category stats
    for cat in m.categories.values_mut() {
        cat.decay(dt, FAST_TAU);
    }

    m.last_decay_ts = now_ts;
}

/// Extract hour (0-23) from a Unix timestamp, using local time.
fn time_hour_from_ts(ts: f64) -> u32 {
    // Use system local time offset. On the VM this is UTC or configured TZ.
    let secs = ts as i64;
    // Simple UTC-based hour extraction (companion config can set timezone later)
    ((secs % 86400) / 3600) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cold_start_warm_prior() {
        let model = UserModel::new();
        // Warm prior: 2/3 ≈ 0.667 (biased toward engagement)
        let eng = model.engagement();
        assert!(eng > 0.6 && eng < 0.7, "Cold start engagement should be ~0.667, got {}", eng);
    }

    #[test]
    fn test_response_increases_engagement() {
        let model = UserModel::new();
        let now = 1700000000.0;

        model.on_proactive_sent("Humor", "Social", now);
        // User responds within 10 minutes
        model.on_user_message(now + 600.0);

        let eng = model.engagement();
        // Should be higher than cold start (0.667)
        assert!(eng > 0.67, "Engagement should increase after positive response, got {}", eng);
    }

    #[test]
    fn test_ignore_decreases_engagement() {
        let model = UserModel::new();
        let now = 1700000000.0;

        model.on_proactive_sent("Humor", "Social", now);
        model.on_proactive_ignored(now + 7200.0);

        let eng = model.engagement();
        // Should be lower than cold start
        assert!(eng < 0.67, "Engagement should decrease after ignore, got {}", eng);
    }

    #[test]
    fn test_ignore_streak_backoff() {
        let model = UserModel::new();
        let now = 1700000000.0;

        // 3 consecutive ignores
        for i in 0..3 {
            let t = now + (i as f64) * 3600.0;
            model.on_proactive_sent("Humor", "Social", t);
            model.on_proactive_ignored(t + 7200.0);
        }

        let backoff = model.backoff_multiplier();
        // 0.5^3 = 0.125
        assert!((backoff - 0.125).abs() < 0.01, "Backoff after 3 ignores should be 0.125, got {}", backoff);
    }

    #[test]
    fn test_category_preference_shrinkage() {
        let model = UserModel::new();
        let now = 1700000000.0;

        // Send 5 Social messages, all responded to quickly
        for i in 0..5 {
            let t = now + (i as f64) * 3600.0;
            model.on_proactive_sent("Humor", "Social", t);
            model.on_user_message(t + 120.0); // 2 min response
        }

        let social_pref = model.category_preference("Social");
        let unknown_pref = model.category_preference("Research");

        // Social should be boosted, Research should be near 1.0
        assert!(social_pref > 1.0, "Social preference should be boosted, got {}", social_pref);
        assert!((unknown_pref - 1.0).abs() < 0.2, "Unknown category should be near 1.0, got {}", unknown_pref);
    }

    #[test]
    fn test_budget_multiplier_bounds() {
        let model = UserModel::new();
        let mult = model.budget_multiplier();
        assert!(mult >= 0.6 && mult <= 1.3, "Budget multiplier out of bounds: {}", mult);
    }
}
