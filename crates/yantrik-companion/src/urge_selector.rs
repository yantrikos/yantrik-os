//! Urge Selection Engine — tier-based log-linear softmax with hazard fairness.
//!
//! Replaces the old resonance-based max-urgency selection for EXECUTE urges.
//!
//! # Algorithm
//!
//! 1. Filter urges: remove resonance-suppressed and budget-exhausted.
//! 2. 10% serendipity override: uniform random from Tier 2-3 candidates.
//! 3. Process tiers 0..3 in order. First non-empty tier wins.
//!    - Tier 0: deterministic argmax on urgency (scheduled tasks, reminders).
//!    - Tiers 1-3: Gumbel-max softmax sampling.
//! 4. Log-linear score:
//!    s(u) = 1.2·ln(U) + 1.5·ln(T) + ln(F_cat) + 0.5·ln(F_inst) + ln(R) + ln(B)
//! 5. Sample via Gumbel-max: argmax_i [s(u_i)/τ + Gumbel_i] with τ=0.7.

use std::collections::HashMap;

use crate::types::{InstinctCategory, TimeSensitivity, UrgeSpec};

// ─── Constants ──────────────────────────────────────────────────────────────

/// Gumbel-max temperature. Lower = more deterministic. 0.7 gives moderate exploration.
const TEMPERATURE: f64 = 0.7;

/// Serendipity override probability.
const SERENDIPITY_PROB: f64 = 0.10;

/// Hazard fairness constants for category-level boost.
const CAT_FAIRNESS_K: f64 = 1.5;
const CAT_FAIRNESS_TAU: f64 = 7200.0; // 2 hours

/// Hazard fairness constants for instinct-level boost.
const INST_FAIRNESS_K: f64 = 0.5;
const INST_FAIRNESS_TAU: f64 = 14400.0; // 4 hours

/// Log-linear score weights.
const W_URGENCY: f64 = 1.2;
const W_TIME_RECEPT: f64 = 1.5;
const W_CAT_FAIRNESS: f64 = 1.0;
const W_INST_FAIRNESS: f64 = 0.5;
const W_NOVELTY: f64 = 1.0;
const W_DIM_RETURNS: f64 = 1.0;

// ─── Default Mappings ───────────────────────────────────────────────────────

/// Centralized default category for an instinct name.
pub fn default_category(instinct_name: &str) -> InstinctCategory {
    match instinct_name {
        "scheduler" | "Reminder" | "Automation" | "morning_brief" | "GoalKeeper"
        | "PredictiveWorkflow" | "SmartUpdates" | "EmailWatch" | "OpportunityScout" => {
            InstinctCategory::Anticipatory
        }
        "check_in" | "CheckIn" | "HealthPulse" | "EnergyMap" | "NightOwl" | "CognitiveLoad" => {
            InstinctCategory::Wellbeing
        }
        "Humor" | "ConnectionWeaver" | "ContextBridge" | "RelationshipRadar"
        | "ConversationalCallback" | "BondMilestone" | "QuestionAsking" => {
            InstinctCategory::Social
        }
        "SkillForge" | "MentorMatch" | "FutureSelf" | "GrowthMirror"
        | "DecisionLab" | "MoneyMind" | "DealWatch" => InstinctCategory::Growth,
        "WorldSense" | "TrendWatch" | "LocalPulse" | "GoldenFind" | "Curiosity"
        | "CulturalRadar" | "weather_watch" | "WeatherWatch" | "ActivityRecommender"
        | "InterestIntelligence" => InstinctCategory::Awareness,
        "EmotionalAwareness" | "Aftermath" | "EveningReflection" | "SilenceReveal"
        | "DreamKeeper" | "FollowUp" => InstinctCategory::Emotional,
        _ => InstinctCategory::Meta,
    }
}

/// Centralized default time sensitivity for an instinct name.
pub fn default_time_sensitivity(instinct_name: &str) -> TimeSensitivity {
    match instinct_name {
        // Tier 0: Immediate
        "scheduler" | "Reminder" | "Automation" => TimeSensitivity::Immediate,
        // Tier 1: Today
        "morning_brief" | "GoalKeeper" | "NightOwl" | "EmailWatch"
        | "EmotionalAwareness" | "Aftermath" | "EveningReflection"
        | "BondMilestone" | "CognitiveLoad" | "PredictiveWorkflow"
        | "weather_watch" | "WeatherWatch" | "ConflictAlerting" | "Cortex" => TimeSensitivity::Today,
        // Tier 2: Soon
        "HealthPulse" | "check_in" | "CheckIn" | "GrowthMirror" | "DecisionLab"
        | "MoneyMind" | "DealWatch" | "WorldSense" | "RelationshipRadar"
        | "SilenceReveal" | "FollowUp" | "SmartUpdates" | "OpportunityScout"
        | "InterestIntelligence" | "Routine" => TimeSensitivity::Soon,
        // Tier 3: Ambient
        _ => TimeSensitivity::Ambient,
    }
}

// ─── Fairness Tracker ───────────────────────────────────────────────────────

/// Tracks when each category and instinct last fired, for hazard-based fairness.
pub struct FairnessTracker {
    category_last_fire: HashMap<InstinctCategory, f64>,
    instinct_last_fire: HashMap<String, f64>,
    daily_exec_counts: HashMap<String, u32>,
    last_daily_reset: f64,
}

impl FairnessTracker {
    pub fn new() -> Self {
        Self {
            category_last_fire: HashMap::new(),
            instinct_last_fire: HashMap::new(),
            daily_exec_counts: HashMap::new(),
            last_daily_reset: 0.0,
        }
    }

    /// Hazard-based category fairness: F = 1 + k·(1 - e^{-Δ/τ})
    pub fn category_factor(&self, cat: InstinctCategory, now_ts: f64) -> f64 {
        let delta = match self.category_last_fire.get(&cat) {
            Some(&ts) if ts > 0.0 => (now_ts - ts).max(0.0),
            _ => CAT_FAIRNESS_TAU * 2.0,
        };
        1.0 + CAT_FAIRNESS_K * (1.0 - (-delta / CAT_FAIRNESS_TAU).exp())
    }

    /// Hazard-based instinct fairness: F = 1 + k·(1 - e^{-Δ/τ})
    pub fn instinct_factor(&self, name: &str, now_ts: f64) -> f64 {
        let delta = match self.instinct_last_fire.get(name) {
            Some(&ts) if ts > 0.0 => (now_ts - ts).max(0.0),
            _ => INST_FAIRNESS_TAU * 2.0,
        };
        1.0 + INST_FAIRNESS_K * (1.0 - (-delta / INST_FAIRNESS_TAU).exp())
    }

    /// Daily execution count for diminishing returns.
    pub fn exec_count(&self, name: &str) -> u32 {
        self.daily_exec_counts.get(name).copied().unwrap_or(0)
    }

    /// Record that an urge fired.
    pub fn record_fire(&mut self, urge: &UrgeSpec, now_ts: f64) {
        self.category_last_fire.insert(urge.category, now_ts);
        self.instinct_last_fire.insert(urge.instinct_name.clone(), now_ts);
        *self.daily_exec_counts.entry(urge.instinct_name.clone()).or_insert(0) += 1;
    }

    /// Reset daily counters if needed.
    pub fn maybe_reset_daily(&mut self, now_ts: f64) {
        if now_ts - self.last_daily_reset > 72000.0 {
            self.daily_exec_counts.clear();
            self.last_daily_reset = now_ts;
        }
    }
}

// ─── Category Budget ────────────────────────────────────────────────────────

/// Per-category daily token budget.
pub struct CategoryBudget {
    tokens: HashMap<InstinctCategory, u32>,
    last_reset_ts: f64,
}

impl CategoryBudget {
    pub fn new() -> Self {
        let mut tokens = HashMap::new();
        tokens.insert(InstinctCategory::Anticipatory, 6);
        tokens.insert(InstinctCategory::Wellbeing, 4);
        tokens.insert(InstinctCategory::Social, 5);
        tokens.insert(InstinctCategory::Growth, 4);
        tokens.insert(InstinctCategory::Awareness, 5);
        tokens.insert(InstinctCategory::Emotional, 4);
        tokens.insert(InstinctCategory::Meta, 3);
        Self {
            tokens,
            last_reset_ts: 0.0,
        }
    }

    pub fn has_tokens(&self, cat: InstinctCategory) -> bool {
        self.tokens.get(&cat).copied().unwrap_or(0) > 0
    }

    pub fn consume(&mut self, cat: InstinctCategory) -> bool {
        if let Some(t) = self.tokens.get_mut(&cat) {
            if *t > 0 {
                *t -= 1;
                return true;
            }
        }
        false
    }

    pub fn maybe_reset(&mut self, now_ts: f64) {
        if now_ts - self.last_reset_ts > 72000.0 {
            *self = Self::new();
            self.last_reset_ts = now_ts;
        }
    }

    pub fn remaining(&self, cat: InstinctCategory) -> u32 {
        self.tokens.get(&cat).copied().unwrap_or(0)
    }
}

// ─── Selection ──────────────────────────────────────────────────────────────

pub struct SelectionInput<'a> {
    pub urges: &'a [UrgeSpec],
    pub novelty: &'a HashMap<String, f64>,
    pub suppressed: &'a HashMap<String, bool>,
    pub now_ts: f64,
    pub time_receptivity: &'a HashMap<String, f64>,
}

pub struct SelectionResult {
    pub index: usize,
    pub serendipity: bool,
    pub score: f64,
}

/// Select the best EXECUTE urge using tier-based log-linear softmax.
pub fn select(
    input: &SelectionInput,
    fairness: &FairnessTracker,
    budgets: &CategoryBudget,
) -> Option<SelectionResult> {
    let candidates: Vec<(usize, &UrgeSpec)> = input.urges.iter().enumerate()
        .filter(|(_, u)| !input.suppressed.get(&u.instinct_name).copied().unwrap_or(false))
        .filter(|(_, u)| budgets.has_tokens(u.category))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // ── Serendipity Override (10%) ──────────────────────────────────
    let entropy = pseudo_random_f64(input.now_ts);
    if entropy < SERENDIPITY_PROB {
        let ambient: Vec<&(usize, &UrgeSpec)> = candidates.iter()
            .filter(|(_, u)| u.time_sensitivity.tier() >= 2)
            .collect();
        if !ambient.is_empty() {
            let pick_idx = pseudo_random_usize(input.now_ts, ambient.len());
            let (orig_idx, _) = ambient[pick_idx];
            tracing::info!(
                instinct = %input.urges[*orig_idx].instinct_name,
                "Serendipity override: random ambient pick"
            );
            return Some(SelectionResult {
                index: *orig_idx,
                serendipity: true,
                score: 0.0,
            });
        }
    }

    // ── Tier-by-tier Processing ─────────────────────────────────────
    for tier in 0..=3u8 {
        let tier_candidates: Vec<&(usize, &UrgeSpec)> = candidates.iter()
            .filter(|(_, u)| u.time_sensitivity.tier() == tier)
            .collect();

        if tier_candidates.is_empty() {
            continue;
        }

        // Tier 0: deterministic argmax
        if tier == 0 {
            let best = tier_candidates.iter()
                .max_by(|a, b| a.1.urgency.partial_cmp(&b.1.urgency).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap();
            return Some(SelectionResult {
                index: best.0,
                serendipity: false,
                score: best.1.urgency,
            });
        }

        // Tiers 1-3: Gumbel-max softmax sampling
        if let Some(result) = gumbel_max_select(&tier_candidates, input, fairness, tier) {
            return Some(result);
        }
    }

    None
}

/// Log-linear score for a single urge.
fn log_linear_score(
    urge: &UrgeSpec,
    input: &SelectionInput,
    fairness: &FairnessTracker,
    tier: u8,
) -> f64 {
    let u = urge.urgency.max(0.01);
    let t = input.time_receptivity
        .get(&format!("{:?}", urge.category))
        .copied()
        .unwrap_or(1.0)
        .max(0.01);
    let f_cat = fairness.category_factor(urge.category, input.now_ts);
    let f_inst = fairness.instinct_factor(&urge.instinct_name, input.now_ts);
    let r = input.novelty
        .get(&urge.instinct_name)
        .copied()
        .unwrap_or(0.5)
        .max(0.01);
    let n = fairness.exec_count(&urge.instinct_name);
    let b = diminishing_returns(tier, n).max(0.01);

    W_URGENCY * u.ln()
        + W_TIME_RECEPT * t.ln()
        + W_CAT_FAIRNESS * f_cat.ln()
        + W_INST_FAIRNESS * f_inst.ln()
        + W_NOVELTY * r.ln()
        + W_DIM_RETURNS * b.ln()
}

/// Gumbel-max trick: argmax_i [s(u_i)/τ + G_i] where G_i ~ Gumbel(0,1).
fn gumbel_max_select(
    candidates: &[&(usize, &UrgeSpec)],
    input: &SelectionInput,
    fairness: &FairnessTracker,
    tier: u8,
) -> Option<SelectionResult> {
    if candidates.is_empty() {
        return None;
    }

    let mut best_idx = candidates[0].0;
    let mut best_score_raw = f64::NEG_INFINITY;
    let mut best_perturbed = f64::NEG_INFINITY;

    for (i, &&(orig_idx, ref urge)) in candidates.iter().enumerate() {
        let s = log_linear_score(urge, input, fairness, tier);
        let u_rand = pseudo_random_f64_seeded(input.now_ts, &urge.instinct_name, i);
        let gumbel = -(-(u_rand.max(1e-10)).ln()).ln();
        let perturbed = s / TEMPERATURE + gumbel;

        tracing::debug!(
            instinct = %urge.instinct_name,
            score = %format!("{:.3}", s),
            perturbed = %format!("{:.3}", perturbed),
            tier = tier,
            "Log-linear score"
        );

        if perturbed > best_perturbed {
            best_perturbed = perturbed;
            best_score_raw = s;
            best_idx = orig_idx;
        }
    }

    Some(SelectionResult {
        index: best_idx,
        serendipity: false,
        score: best_score_raw,
    })
}

/// Tier-dependent diminishing returns.
fn diminishing_returns(tier: u8, exec_count: u32) -> f64 {
    let base: f64 = if tier <= 1 { 0.95 } else { 0.70 };
    base.powi(exec_count as i32)
}

// ─── Pseudo-random Helpers ──────────────────────────────────────────────────

fn pseudo_random_f64(ts: f64) -> f64 {
    let bits = ts.to_bits();
    let mixed = splitmix64(bits);
    (mixed as f64 / u64::MAX as f64).clamp(1e-10, 1.0 - 1e-10)
}

fn pseudo_random_usize(ts: f64, max: usize) -> usize {
    let bits = ts.to_bits();
    let mixed = splitmix64(bits);
    (mixed as usize) % max
}

fn pseudo_random_f64_seeded(ts: f64, name: &str, idx: usize) -> f64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ts.to_bits().hash(&mut hasher);
    name.hash(&mut hasher);
    idx.hash(&mut hasher);
    let h = hasher.finish();
    let mixed = splitmix64(h);
    (mixed as f64 / u64::MAX as f64).clamp(1e-10, 1.0 - 1e-10)
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

// ─── High-Level Pipeline ────────────────────────────────────────────────────

/// High-level function that runs the full selection pipeline.
/// Called from both main.rs and bridge.rs.
pub fn process_execute_urges(
    urges: &[UrgeSpec],
    fairness: &mut FairnessTracker,
    budgets: &mut CategoryBudget,
    resonance: &crate::resonance::ResonanceEngine,
    user_model: &crate::user_model::UserModel,
    recent_messages: &[String],
    bond_level: crate::bond::BondLevel,
    now_ts: f64,
) -> Option<(UrgeSpec, bool)> {
    if urges.is_empty() {
        return None;
    }

    budgets.maybe_reset(now_ts);
    fairness.maybe_reset_daily(now_ts);

    // 1. Compute novelty + suppression from resonance gates
    let mut novelty_map: HashMap<String, f64> = HashMap::new();
    let mut suppress_map: HashMap<String, bool> = HashMap::new();

    for urge in urges {
        let novelty = resonance.information_novelty_pub(&urge.reason, recent_messages);
        let depth_ok = resonance.depth_check(&urge.instinct_name, bond_level);

        novelty_map.insert(urge.instinct_name.clone(), novelty);

        if !depth_ok {
            suppress_map.insert(urge.instinct_name.clone(), true);
            tracing::info!(
                instinct = %urge.instinct_name,
                "Depth gate suppressed: too deep for bond level"
            );
        }
    }

    // 2. Compute time receptivity from user model
    let mut time_receptivity: HashMap<String, f64> = HashMap::new();
    let hour = (now_ts as i64 % 86400 / 3600) as u32;
    for cat in &[
        InstinctCategory::Anticipatory, InstinctCategory::Wellbeing,
        InstinctCategory::Social, InstinctCategory::Growth,
        InstinctCategory::Awareness, InstinctCategory::Emotional,
        InstinctCategory::Meta,
    ] {
        let cat_str = format!("{:?}", cat);
        let recept = user_model.urgency_multiplier(&cat_str, hour);
        time_receptivity.insert(cat_str, recept);
    }

    // 3. Run selection
    let input = SelectionInput {
        urges,
        novelty: &novelty_map,
        suppressed: &suppress_map,
        now_ts,
        time_receptivity: &time_receptivity,
    };

    let result = select(&input, fairness, budgets)?;

    // 4. Record fire
    let selected = urges[result.index].clone();
    fairness.record_fire(&selected, now_ts);
    budgets.consume(selected.category);

    tracing::info!(
        instinct = %selected.instinct_name,
        category = ?selected.category,
        tier = selected.time_sensitivity.tier(),
        score = %format!("{:.3}", result.score),
        serendipity = result.serendipity,
        budget_remaining = budgets.remaining(selected.category),
        "Urge selected by tier-softmax"
    );

    Some((selected, result.serendipity))
}
