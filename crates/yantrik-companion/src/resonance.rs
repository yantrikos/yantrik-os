
//! Resonance Model — A novel mathematical framework for communication priority
//! and connection building in AI companion systems.
//!
//! # Theory
//!
//! Human relationships follow **coupled oscillator dynamics**: two entities
//! synchronize through well-timed, novel, appropriately-deep interactions.
//! This model combines three mathematical frameworks:
//!
//! 1. **Kuramoto Synchronization** — Phase alignment between companion and
//!    user communication rhythms. When phases align (φ → 0), messages feel
//!    naturally timed. When misaligned (φ → π), they feel intrusive.
//!
//! 2. **Information Theory** — Novelty scoring via semantic distance from
//!    recent messages. High-entropy (surprising) messages carry more value.
//!    Repeated themes suffer exponential decay in perceived value.
//!
//! 3. **Social Penetration Theory** (Altman & Taylor, 1973) — Relationships
//!    deepen through progressive self-disclosure. The companion must match
//!    the user's disclosure depth, not exceed it. Depth is modeled as layers
//!    of an onion, with bond level gating access to deeper layers.
//!
//! # The Resonance Score (v3)
//!
//! ```text
//! log(quality) = Σ(w_i · ln(x_i)) / Σw_i - γ · Var(ln(x_i))
//! gate = (1-ε) · D·B·(1-F) + ε · g₀
//! R = quality · gate · sigmoid_gates
//! final = 0.4 · urgency + 0.6 · R
//! ```
//!
//! Where:
//! - `I(m)` = Information novelty of message m (0.0–1.0)
//! - `P(t)` = Phase alignment score (0.0–1.0, from Kuramoto dynamics)
//! - `D(t)` = Desire pressure (grows with silence, decays after messaging)
//! - `B(t)` = Bond resonance factor (deeper bonds amplify communication)
//! - `F(t)` = Fatigue factor (recent message density dampens urgency)
//! - `V(m)` = Variety bonus (rewards switching between instinct categories)
//!
//! # Phase Dynamics (Kuramoto-inspired)
//!
//! The phase difference evolves according to:
//! ```text
//! dφ/dt = Δω - K · sin(φ)
//! ```
//!
//! Where Δω is the frequency mismatch between companion and user rhythms,
//! and K is the coupling strength (proportional to bond level). Stronger
//! bonds → stronger pull toward synchronization → more forgiving of timing.
//!
//! # Bond Growth Model
//!
//! Bond amplitude evolves as:
//! ```text
//! dA/dt = α · Q(t) · reciprocity(t) - β · A(t) + γ · depth_match(t)
//! ```
//!
//! - Quality interactions (user responds positively) increase bond
//! - Natural decay ensures relationships need maintenance
//! - Depth matching (reciprocal disclosure) amplifies growth
//! - Mismatched depth (companion too deep, user shallow) penalizes
//!
//! # Connection Strategy Axioms
//!
//! 1. **Optimal timing beats optimal content** — A mediocre message at the
//!    right moment outperforms a brilliant message at the wrong time.
//! 2. **Scarcity creates value** — Fewer, higher-quality messages build
//!    stronger connections than frequent low-value ones.
//! 3. **Strategic silence is communication** — Not speaking can strengthen
//!    the relationship more than speaking, especially early on.
//! 4. **Reciprocity drives deepening** — Match the user's energy, don't
//!    exceed it. A companion that's "too eager" erodes trust.
//! 5. **Variety prevents habituation** — Rotating between instinct categories
//!    prevents the user from mentally filtering out messages.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::bond::BondLevel;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Kuramoto coupling strength per bond level.
/// Higher coupling → stronger pull toward synchronization.
/// Based on empirical relationship psychology: early relationships
/// are fragile (low K), deep relationships are resilient (high K).
const COUPLING_STRANGER: f64 = 0.1;
const COUPLING_ACQUAINTANCE: f64 = 0.25;
const COUPLING_FRIEND: f64 = 0.5;
const COUPLING_CONFIDANT: f64 = 0.75;
const COUPLING_PARTNER: f64 = 0.95;

/// Natural decay rate for desire pressure (λ in D(t) = 1 - e^(-λt)).
/// Controls how fast the "urge to communicate" builds during silence.
/// At λ = 0.0003: 50% pressure after ~38 minutes, 90% after ~2.1 hours.
const DESIRE_DECAY_RATE: f64 = 0.0003;

/// Fatigue half-life in seconds.
/// Each sent message contributes fatigue that decays with this half-life.
/// At 1800s (30 min): a message sent 30 min ago has half its fatigue weight.
const FATIGUE_HALF_LIFE: f64 = 1800.0;

/// Maximum number of category history entries to track for variety scoring.
const VARIETY_HISTORY_SIZE: usize = 20;

/// Phase correction applied per positive interaction (radians).
/// Pulls phase toward alignment. Larger = faster sync.
const PHASE_CORRECTION_POSITIVE: f64 = 0.3;

/// Phase drift applied per negative signal (ignored message, etc.)
const PHASE_CORRECTION_NEGATIVE: f64 = -0.15;

/// Bond growth per quality interaction (α coefficient).
const BOND_GROWTH_ALPHA: f64 = 0.008;

/// Natural bond decay per hour (β coefficient).
/// At β=0.001: bond loses ~0.024 per day without interaction.
/// Prevents stale relationships from claiming deep bond.
const BOND_DECAY_BETA: f64 = 0.001;

/// Depth matching bonus (γ coefficient).
/// Rewards reciprocal disclosure depth matching.
const DEPTH_MATCH_GAMMA: f64 = 0.005;

/// Depth penalty for exceeding user's disclosure level.
/// Companion going too deep too fast erodes trust.
const DEPTH_OVERSHOOT_PENALTY: f64 = 0.01;

/// Balance penalty coefficient (γ) for log-space variance.
/// Penalizes imbalanced quality components (one great, one terrible).
/// Higher γ → stronger preference for balanced scores.
const BALANCE_GAMMA: f64 = 0.3;

/// Exploration prior ε — fraction of gate that's always "on".
/// Ensures the companion is never 100% silent, even in poor conditions.
const GATE_EPSILON: f64 = 0.1;
const GATE_BASELINE: f64 = 0.15;

/// Adaptive frequency learning rate (μ) — how fast Δω adjusts.
/// Hebbian: consistent lead/lag slowly corrects the mismatch.
const ADAPTIVE_FREQ_MU: f64 = 0.0005;

/// Adaptive frequency forgetting rate (λ) — natural decay of learned Δω.
const ADAPTIVE_FREQ_LAMBDA: f64 = 0.001;

/// Second-harmonic coupling ratio (K₂ = ratio * K₁).
/// Destabilizes anti-phase equilibrium (φ=π) so perturbations push away.
/// From generalized Kuramoto models (Acebrón et al., 2005).
const SECOND_HARMONIC_RATIO: f64 = 0.4;

/// Anti-phase escape learning rate (μ₂) for sin(φ/2) term.
/// Only active near φ≈π via sigmoid gate. Small to avoid bias when healthy.
const ANTIPHASE_ESCAPE_MU: f64 = 0.001;

/// Phase evidence decay time constant (seconds).
/// Controls how fast phase certainty decays without user interaction.
/// At τ=1800 (30 min): evidence halves every ~21 minutes.
const PHASE_EVIDENCE_TAU: f64 = 1800.0;

/// Post-stuck Δω cooldown time constant (seconds).
/// After a stuck reset, base drift is reduced for this duration.
const STUCK_COOLDOWN_TAU: f64 = 1800.0;

// ─── Instinct Categories for Variety Scoring ────────────────────────────────

/// Categories for variety tracking — instincts grouped by function.
/// Variety bonus rewards switching between categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstinctCategory {
    /// Research & knowledge: DeepDive, MythBuster, WonderSense, CookingCompanion
    Research,
    /// Self-reflection: GrowthMirror, IdentityThread, SecondBrain, LegacyBuilder
    Reflection,
    /// Social intelligence: ContextBridge, ConnectionWeaver, RelationshipRadar
    Social,
    /// World awareness: WorldSense, TrendWatch, LocalPulse, GoldenFind
    WorldAwareness,
    /// Wellness: HealthPulse, EnergyMap, NightOwl
    Wellness,
    /// Growth: SkillForge, GoalKeeper, FutureSelf, MentorMatch
    Growth,
    /// Creative: Humor, CulturalRadar, PhilosophyCompanion, CreativeSpark
    Creative,
    /// Practical: MoneyMind, DecisionLab, OpportunityScout
    Practical,
    /// Temporal: EveningReflection, MemoryWeaver, TimeCapture, TraditionKeeper
    Temporal,
    /// Meta: PatternBreaker, DevilsAdvocate, DebriefPartner, SocraticSpark
    MetaCognitive,
    /// System: scheduler, check_in, automation
    System,
}

impl InstinctCategory {
    /// Map an instinct name to its category.
    pub fn from_instinct(name: &str) -> Self {
        match name {
            // Research
            "DeepDive" | "MythBuster" | "WonderSense" | "CookingCompanion" => Self::Research,
            // Reflection
            "GrowthMirror" | "IdentityThread" | "SecondBrain" | "LegacyBuilder"
            | "SelfAwareness" => Self::Reflection,
            // Social
            "ContextBridge" | "ConnectionWeaver" | "RelationshipRadar" | "Humor" => Self::Social,
            // World awareness
            "WorldSense" | "TrendWatch" | "LocalPulse" | "GoldenFind" | "Curiosity" => {
                Self::WorldAwareness
            }
            // Wellness
            "HealthPulse" | "EnergyMap" | "NightOwl" => Self::Wellness,
            // Growth
            "SkillForge" | "GoalKeeper" | "FutureSelf" | "MentorMatch" => Self::Growth,
            // Creative
            "CulturalRadar" | "PhilosophyCompanion" | "TimeCapture" => Self::Creative,
            // Practical
            "MoneyMind" | "DecisionLab" | "OpportunityScout" => Self::Practical,
            // Temporal
            "EveningReflection" | "MemoryWeaver" | "TraditionKeeper"
            | "morning_brief" | "ConversationalCallback" => Self::Temporal,
            // Meta-cognitive
            "PatternBreaker" | "DevilsAdvocate" | "DebriefPartner" | "SocraticSpark"
            | "Aftermath" => Self::MetaCognitive,
            // System
            _ => Self::System,
        }
    }
}

// ─── Disclosure Depth Model ─────────────────────────────────────────────────

/// Disclosure depth layers (Social Penetration Theory).
/// Each layer represents increasingly personal territory.
/// The companion should MATCH the user's depth, never exceed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DisclosureDepth {
    /// Surface: weather, news, general facts. Safe for Stranger/Acquaintance.
    Surface = 0,
    /// Preferences: interests, opinions, likes/dislikes. Friend territory.
    Preferences = 1,
    /// Personal: goals, fears, struggles, relationships. Confidant territory.
    Personal = 2,
    /// Core: identity, values, existential questions, vulnerability. Partner only.
    Core = 3,
}

impl DisclosureDepth {
    /// Maximum disclosure depth appropriate for a bond level.
    pub fn max_for_bond(bond: BondLevel) -> Self {
        match bond {
            BondLevel::Stranger => Self::Surface,
            BondLevel::Acquaintance => Self::Preferences,
            BondLevel::Friend => Self::Personal,
            BondLevel::Confidant => Self::Core,
            BondLevel::PartnerInCrime => Self::Core,
        }
    }

    /// Map an instinct to its typical disclosure depth.
    pub fn of_instinct(name: &str) -> Self {
        match name {
            // Surface: factual, impersonal
            "WorldSense" | "TrendWatch" | "LocalPulse" | "CookingCompanion"
            | "WonderSense" | "MythBuster" | "CulturalRadar" | "TraditionKeeper"
            | "OpportunityScout" | "scheduler" => Self::Surface,
            // Preferences: interest-adjacent, personalized but not intimate
            "GoldenFind" | "DeepDive" | "ConnectionWeaver" | "Curiosity"
            | "MoneyMind" | "SkillForge" | "DecisionLab" | "MentorMatch"
            | "Humor" | "EnergyMap" => Self::Preferences,
            // Personal: touches goals, struggles, growth
            "GrowthMirror" | "GoalKeeper" | "ContextBridge" | "RelationshipRadar"
            | "FutureSelf" | "PatternBreaker" | "DebriefPartner" | "HealthPulse"
            | "NightOwl" | "TimeCapture" | "DreamKeeper" | "EveningReflection"
            | "Aftermath" | "SocraticSpark" => Self::Personal,
            // Core: identity, existential, deeply personal
            "IdentityThread" | "SecondBrain" | "LegacyBuilder"
            | "DevilsAdvocate" | "PhilosophyCompanion" => Self::Core,
            _ => Self::Surface,
        }
    }
}

// ─── Resonance Engine ───────────────────────────────────────────────────────

/// The Resonance Engine computes communication priority scores and tracks
/// the dynamic state of the companion-user relationship.
pub struct ResonanceEngine {
    // ── Kuramoto Phase State ────────────────────────────────────────────
    /// Current phase difference (radians, 0 to 2π).
    /// 0 = perfectly aligned, π = maximally misaligned.
    phase: Mutex<f64>,

    /// Estimated user communication frequency (messages per hour).
    /// Learned from interaction patterns. Higher = user expects more contact.
    user_frequency: Mutex<f64>,

    /// Companion's recent communication frequency (messages per hour).
    companion_frequency: Mutex<f64>,

    // ── Fatigue Tracking ────────────────────────────────────────────────
    /// Timestamps of recently sent messages (for fatigue calculation).
    sent_timestamps: Mutex<Vec<f64>>,

    // ── Variety Tracking ────────────────────────────────────────────────
    /// Recent instinct categories that fired (for variety bonus).
    category_history: Mutex<Vec<InstinctCategory>>,

    // ── User Disclosure Depth ───────────────────────────────────────────
    /// Estimated user disclosure depth (learned from message content).
    user_depth: Mutex<DisclosureDepth>,

    // ── Interaction Quality History ─────────────────────────────────────
    /// Rolling window of interaction quality scores (0.0–1.0).
    /// Positive response = 1.0, ignored = 0.0, negative = -0.5.
    quality_history: Mutex<Vec<f64>>,

    // ── Timing State ────────────────────────────────────────────────────
    /// Timestamp of last message sent by companion.
    last_sent_ts: Mutex<f64>,

    /// Timestamp of last user interaction.
    last_user_ts: Mutex<f64>,

    // ── Adaptive Frequency Mismatch ──────────────────────────────────────
    /// Learned frequency offset (Hebbian). Evolves via:
    /// dΔω_adapt/dt = -λ·Δω_adapt + μ·sin(φ)
    /// Positive = companion tends to lead; negative = companion tends to lag.
    adaptive_delta_omega: Mutex<f64>,

    // ── Stuck Detection ────────────────────────────────────────────────
    /// Consecutive ticks where |φ| > 2.5 rad (near anti-phase).
    stuck_counter: Mutex<u32>,
    /// Timestamp of last stuck reset (for post-reset Δω cooldown).
    stuck_reset_ts: Mutex<f64>,

    // ── Connection Building Strategy ────────────────────────────────────
    /// Bond growth recommendation: positive = invest more, negative = pull back.
    growth_signal: Mutex<f64>,
}

/// Result of resonance scoring for a single urge.
#[derive(Debug, Clone)]
pub struct ResonanceScore {
    /// The final composite score (0.0–1.0). Replaces raw urgency.
    pub score: f64,
    /// Information novelty component.
    pub novelty: f64,
    /// Phase alignment component.
    pub phase_alignment: f64,
    /// Desire pressure component.
    pub desire: f64,
    /// Bond resonance component.
    pub bond_factor: f64,
    /// Fatigue dampening (1.0 = no fatigue, 0.0 = fully fatigued).
    pub fatigue_inv: f64,
    /// Variety bonus.
    pub variety: f64,
    /// Depth appropriateness (1.0 = perfect match, 0.0 = way too deep).
    pub depth_fit: f64,
    /// Whether this message should be suppressed by the resonance model.
    pub suppress: bool,
    /// Reason for suppression (if any).
    pub suppress_reason: String,
}

impl ResonanceEngine {
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(0.0),
            user_frequency: Mutex::new(1.0),    // Default: ~1 msg/hour
            companion_frequency: Mutex::new(0.5), // Conservative start
            sent_timestamps: Mutex::new(Vec::new()),
            category_history: Mutex::new(Vec::new()),
            user_depth: Mutex::new(DisclosureDepth::Surface),
            quality_history: Mutex::new(Vec::new()),
            last_sent_ts: Mutex::new(0.0),
            last_user_ts: Mutex::new(0.0),
            adaptive_delta_omega: Mutex::new(0.0),
            stuck_counter: Mutex::new(0),
            stuck_reset_ts: Mutex::new(0.0),
            growth_signal: Mutex::new(0.0),
        }
    }

    // ── Core Scoring ────────────────────────────────────────────────────

    /// Compute the Resonance Score for a candidate message.
    ///
    /// This is the central formula:
    /// ```text
    /// R(m, t) = I(m)^0.6 · P(t) · D(t) · B(bond) · (1 - F(t)) · V(cat) · Depth(m, bond)
    /// ```
    ///
    /// The 0.6 exponent on novelty prevents it from dominating — a slightly
    /// repetitive but perfectly timed message still has value.
    pub fn score(
        &self,
        instinct_name: &str,
        raw_urgency: f64,
        message_text: &str,
        recent_messages: &[String],
        bond_level: BondLevel,
        bond_score: f64,
        now_ts: f64,
    ) -> ResonanceScore {
        let novelty = self.information_novelty(message_text, recent_messages);
        let phase_alignment_raw = self.phase_alignment_score();
        let desire = self.desire_pressure(now_ts);
        let bond_factor = self.bond_resonance(bond_level, bond_score);
        let fatigue_inv = 1.0 - self.fatigue_factor(now_ts);
        let variety = self.variety_bonus(instinct_name);
        let depth_fit = self.depth_appropriateness(instinct_name, bond_level);

        // ── Phase Uncertainty Blending (v4) ──────────────────────────────
        // When user interactions are sparse, phase is poorly identified.
        // Blend toward neutral (0.5) proportional to evidence decay.
        let last_user = *self.last_user_ts.lock().unwrap();
        let silence_secs = if last_user > 0.0 { (now_ts - last_user).max(0.0) } else { 0.0 };
        let evidence = (-silence_secs / PHASE_EVIDENCE_TAU).exp().clamp(0.0, 1.0);
        let phase_alignment = evidence * phase_alignment_raw + (1.0 - evidence) * 0.5;

        // Interaction sparsity scalar for downstream adjustments
        let sparsity = 1.0 - (-silence_secs / 1200.0).exp(); // τ_S = 20 min

        // ── The Resonance Formula (v4) ──────────────────────────────────
        //
        // Product-of-experts interpretation: each quality factor is a
        // calibrated "compatibility probability." We aggregate in log-space
        // with a variance penalty for balance (inequality aversion).
        //
        //   log(quality) = Σ(w_i · ln(x_i)) / Σw_i - γ_eff · Var(ln(x_i))
        //   gate = (1-ε) · D·B·(1-F) + ε · g₀    (exploration prior)
        //   R = quality · gate · sigmoid_gates
        //   final = 0.4 · raw_urgency + 0.6 · R
        //
        // v4: γ softened under sparsity (noisier component estimates).

        let w_novelty = 2.0_f64;
        let w_phase = 1.5_f64;
        let w_variety = 1.0_f64;
        let w_depth = 2.0_f64;
        let w_sum = w_novelty + w_phase + w_variety + w_depth;

        // Log-space quality with variance penalty
        let ln_i = novelty.max(0.01).ln();
        let ln_p = phase_alignment.max(0.01).ln();
        let ln_v = variety.max(0.5).ln(); // variety min is 0.5
        let ln_d = depth_fit.max(0.01).ln();

        let weighted_logs = [w_novelty * ln_i, w_phase * ln_p, w_variety * ln_v, w_depth * ln_d];
        let mean_log = weighted_logs.iter().sum::<f64>() / w_sum;

        // Variance of the individual (unweighted) log values
        let logs = [ln_i, ln_p, ln_v, ln_d];
        let log_mean = logs.iter().sum::<f64>() / 4.0;
        let var_log = logs.iter().map(|l| (l - log_mean).powi(2)).sum::<f64>() / 4.0;

        // v4: Soften variance penalty under sparsity (noisier estimates)
        let gamma_eff = BALANCE_GAMMA * (1.0 - 0.4 * sparsity);
        let quality = (mean_log - gamma_eff * var_log).exp();

        // Gate with ε-mixture exploration prior (smooth, interpretable)
        let raw_gate = desire * bond_factor * fatigue_inv;
        let gate = (1.0 - GATE_EPSILON) * raw_gate + GATE_EPSILON * GATE_BASELINE;

        // Smooth suppression via sigmoid gates (no hard thresholds)
        fn sigmoid(x: f64) -> f64 {
            1.0 / (1.0 + (-x).exp())
        }
        let phase_gate = sigmoid(15.0 * (phase_alignment - 0.1));
        let depth_gate = sigmoid(12.0 * (depth_fit - 0.25));
        let fatigue_gate = sigmoid(20.0 * (fatigue_inv - 0.08));
        let suppress_mult = phase_gate * depth_gate * fatigue_gate;

        let resonance = quality * gate * suppress_mult;

        // Blend: instinct urgency (40%) + resonance dynamics (60%)
        let blended = 0.4 * raw_urgency + 0.6 * resonance;
        let score = blended.clamp(0.0, 1.0);

        // ── Suppression Decision ─────────────────────────────────────────
        // Smooth gates handle most suppression via low scores.
        // Hard suppress only for extreme cases (belt-and-suspenders).
        let suppress = suppress_mult < 0.05 && raw_urgency < 0.8;
        let suppress_reason = if suppress {
            format!(
                "Phase misalignment ({:.2}) — companion and user out of sync",
                phase_alignment_raw,
            )
        } else {
            String::new()
        };

        // ── Component Telemetry (v4) ─────────────────────────────────────
        tracing::debug!(
            instinct = %instinct_name,
            novelty = %format!("{:.3}", novelty),
            phase_raw = %format!("{:.3}", phase_alignment_raw),
            phase_eff = %format!("{:.3}", phase_alignment),
            evidence = %format!("{:.3}", evidence),
            variety = %format!("{:.3}", variety),
            depth = %format!("{:.3}", depth_fit),
            desire = %format!("{:.3}", desire),
            bond = %format!("{:.3}", bond_factor),
            fatigue_inv = %format!("{:.3}", fatigue_inv),
            quality = %format!("{:.3}", quality),
            gate = %format!("{:.3}", gate),
            suppress_mult = %format!("{:.3}", suppress_mult),
            sparsity = %format!("{:.3}", sparsity),
            gamma_eff = %format!("{:.3}", gamma_eff),
            resonance = %format!("{:.4}", resonance),
            final_score = %format!("{:.4}", score),
            suppress = suppress,
            "Resonance component breakdown"
        );

        ResonanceScore {
            score,
            novelty,
            phase_alignment,
            desire,
            bond_factor,
            fatigue_inv,
            variety,
            depth_fit,
            suppress,
            suppress_reason,
        }
    }

    // ── Component Functions ──────────────────────────────────────────────

    /// Information Novelty I(m): semantic distance from recent messages.
    ///
    /// Uses word-level Jaccard distance (1 - similarity).
    /// Returns 1.0 for completely novel, 0.0 for identical to recent.
    ///
    /// We check against ALL recent messages and take the MINIMUM novelty
    /// (maximum similarity), because even one similar recent message means
    /// this content lacks surprise.
    fn information_novelty(&self, message: &str, recent: &[String]) -> f64 {
        if recent.is_empty() || message.is_empty() {
            return 1.0; // No history = maximum novelty
        }

        let msg_lower = message.to_lowercase();
        let msg_words: std::collections::HashSet<&str> =
            msg_lower.split_whitespace().collect();

        if msg_words.is_empty() {
            return 1.0;
        }

        let mut max_similarity = 0.0f64;
        for recent_msg in recent {
            let recent_lower = recent_msg.to_lowercase();
            let recent_words: std::collections::HashSet<&str> =
                recent_lower.split_whitespace().collect();

            let intersection = msg_words.intersection(&recent_words).count();
            let union = msg_words.union(&recent_words).count();
            if union > 0 {
                let jaccard = intersection as f64 / union as f64;
                max_similarity = max_similarity.max(jaccard);
            }
        }

        1.0 - max_similarity
    }

    /// Phase Alignment P(t): how "in sync" the companion is with the user.
    ///
    /// Returns 0.0–1.0 where 1.0 = perfectly aligned.
    /// Derived from the Kuramoto phase variable: P = (1 + cos(φ)) / 2.
    fn phase_alignment_score(&self) -> f64 {
        let phi = *self.phase.lock().unwrap();
        // Map cos(φ) from [-1, 1] to [0, 1]
        (1.0 + phi.cos()) / 2.0
    }

    /// Desire Pressure D(t): urge to communicate grows with silence.
    ///
    /// D(t) = 1 - e^(-λ · silence_duration)
    ///
    /// Approaches 1.0 asymptotically — there's always some pressure to speak
    /// after prolonged silence. λ controls the curve shape.
    fn desire_pressure(&self, now_ts: f64) -> f64 {
        let last_sent = *self.last_sent_ts.lock().unwrap();
        if last_sent == 0.0 {
            return 0.8; // First message — moderate desire
        }
        let silence_secs = (now_ts - last_sent).max(0.0);
        1.0 - (-DESIRE_DECAY_RATE * silence_secs).exp()
    }

    /// Bond Resonance B(bond): deeper bonds amplify communication value.
    ///
    /// Uses a sigmoid-like curve so early bond levels still allow communication
    /// but at reduced resonance. The formula:
    ///
    /// B = 0.3 + 0.7 · (bond_score^0.7)
    ///
    /// This ensures even Stranger (bond_score ~0.0) gets B = 0.3 (messages
    /// still possible) while Partner (bond_score ~1.0) gets B = 1.0.
    fn bond_resonance(&self, _bond_level: BondLevel, bond_score: f64) -> f64 {
        0.3 + 0.7 * bond_score.clamp(0.0, 1.0).powf(0.7)
    }

    /// Fatigue Factor F(t): how "tired" the user is of hearing from us.
    ///
    /// Each sent message contributes fatigue that decays exponentially:
    /// F(t) = Σ (e^(-ln(2) · age_i / half_life)) for each recent message
    ///
    /// Capped at 1.0. The half-life controls how fast fatigue dissipates.
    fn fatigue_factor(&self, now_ts: f64) -> f64 {
        let timestamps = self.sent_timestamps.lock().unwrap();
        let decay_constant = (2.0_f64).ln() / FATIGUE_HALF_LIFE;

        let mut fatigue = 0.0;
        for &ts in timestamps.iter() {
            let age = (now_ts - ts).max(0.0);
            fatigue += (-decay_constant * age).exp();
        }

        // Normalize: each message at age=0 contributes 1.0 fatigue.
        // Divisor of 5.0 means it takes ~5 rapid-fire messages to saturate.
        // This prevents the companion from going silent after normal activity.
        (fatigue / 5.0).min(1.0)
    }

    /// Variety Bonus V(cat): rewards switching between instinct categories.
    ///
    /// If the last N messages were all from the same category, variety = 0.5.
    /// If switching to a different category, variety = 1.0 + 0.1 bonus.
    /// Intermediate values based on how recently this category fired.
    fn variety_bonus(&self, instinct_name: &str) -> f64 {
        let cat = InstinctCategory::from_instinct(instinct_name);
        let history = self.category_history.lock().unwrap();

        if history.is_empty() {
            return 1.0;
        }

        // Count how many of the last 5 entries match this category
        let recent_same = history.iter().rev().take(5)
            .filter(|&&c| c == cat)
            .count();

        match recent_same {
            0 => 1.1, // Fresh category — bonus!
            1 => 1.0, // Normal
            2 => 0.85,
            3 => 0.7,
            _ => 0.5,  // Saturated category
        }
    }

    /// Depth Appropriateness: does this instinct's depth match the bond level?
    ///
    /// Returns 1.0 if the instinct's typical depth ≤ bond's max depth.
    /// Returns 0.0–0.5 if the instinct is too deep for the current bond.
    /// Uses a graduated penalty rather than hard cutoff.
    fn depth_appropriateness(&self, instinct_name: &str, bond_level: BondLevel) -> f64 {
        let instinct_depth = DisclosureDepth::of_instinct(instinct_name) as i32;
        let max_depth = DisclosureDepth::max_for_bond(bond_level) as i32;

        let overshoot = instinct_depth - max_depth;
        if overshoot <= 0 {
            1.0 // Within bounds
        } else {
            // Graduated penalty: 1 layer too deep = 0.4, 2 = 0.15, 3 = 0.05
            match overshoot {
                1 => 0.4,
                2 => 0.15,
                _ => 0.05,
            }
        }
    }

    // ── Public Accessors for Urge Selector ────────────────────────────

    /// Public wrapper for information novelty scoring.
    /// Returns 0.0 (duplicate) to 1.0 (fully novel).
    pub fn information_novelty_pub(&self, message: &str, recent: &[String]) -> f64 {
        self.information_novelty(message, recent)
    }

    /// Returns true if the instinct's depth is appropriate for the bond level.
    /// Returns false if the instinct is too deep (depth_appropriateness < 0.3).
    pub fn depth_check(&self, instinct_name: &str, bond_level: BondLevel) -> bool {
        self.depth_appropriateness(instinct_name, bond_level) >= 0.3
    }

    // ── State Updates ───────────────────────────────────────────────────

    /// Record that a message was sent. Updates fatigue, phase, variety, frequency.
    pub fn record_sent(&self, instinct_name: &str, now_ts: f64) {
        // Update fatigue timestamps
        {
            let mut timestamps = self.sent_timestamps.lock().unwrap();
            timestamps.push(now_ts);
            // Keep only messages from the last 2 hours
            let cutoff = now_ts - 7200.0;
            timestamps.retain(|&ts| ts > cutoff);
        }

        // Update variety history
        {
            let cat = InstinctCategory::from_instinct(instinct_name);
            let mut history = self.category_history.lock().unwrap();
            history.push(cat);
            if history.len() > VARIETY_HISTORY_SIZE {
                history.remove(0);
            }
        }

        // Update last sent timestamp
        *self.last_sent_ts.lock().unwrap() = now_ts;

        // Update companion frequency (exponential moving average)
        {
            let mut freq = self.companion_frequency.lock().unwrap();
            let last_sent = *self.last_sent_ts.lock().unwrap();
            if last_sent > 0.0 {
                let gap_hours = (now_ts - last_sent).max(1.0) / 3600.0;
                let instant_freq = 1.0 / gap_hours;
                *freq = *freq * 0.8 + instant_freq * 0.2; // EMA smoothing
            }
        }
    }

    /// Record a user interaction. Updates phase, user frequency, quality.
    ///
    /// `quality` should be:
    /// - 1.0 for positive response (user engaged, replied substantively)
    /// - 0.5 for neutral (user acknowledged)
    /// - 0.0 for ignored (companion message got no response)
    /// - -0.5 for negative signal (user said "stop", "not now", etc.)
    pub fn record_user_interaction(&self, now_ts: f64, quality: f64) {
        // Update phase — positive interactions pull phase toward alignment
        {
            let mut phi = self.phase.lock().unwrap();
            if quality > 0.5 {
                // Positive: apply coupling correction toward 0
                *phi += PHASE_CORRECTION_POSITIVE * (-(*phi).sin());
            } else if quality < 0.0 {
                // Negative: push phase away (desynchronize)
                *phi += PHASE_CORRECTION_NEGATIVE;
            }
            // Normalize to [0, 2π]
            *phi = phi.rem_euclid(std::f64::consts::TAU);
        }

        // Update user frequency
        {
            let mut user_freq = self.user_frequency.lock().unwrap();
            let last_user = *self.last_user_ts.lock().unwrap();
            if last_user > 0.0 {
                let gap_hours = (now_ts - last_user).max(1.0) / 3600.0;
                let instant_freq = 1.0 / gap_hours;
                *user_freq = *user_freq * 0.7 + instant_freq * 0.3; // EMA
            }
        }

        *self.last_user_ts.lock().unwrap() = now_ts;

        // Record quality
        {
            let mut qh = self.quality_history.lock().unwrap();
            qh.push(quality);
            if qh.len() > 50 {
                qh.remove(0);
            }
        }
    }

    /// Advance the phase dynamics by one tick (called every think cycle).
    ///
    /// Implements generalized Kuramoto with second-harmonic coupling (v4):
    /// ```text
    /// dφ/dt = Δω_eff - K₁·sin(φ) - K₂·sin(2φ)
    /// dΔω_adapt/dt = -λ·Δω_adapt + μ₁·sin(φ) + μ₂·g(φ)·sin(φ/2)
    /// ```
    ///
    /// Key properties (v4):
    /// - Fatigue REMOVED from oscillator (applied downstream in gate only)
    /// - Second-harmonic K₂·sin(2φ) destabilizes anti-phase (φ=π)
    /// - Gated sin(φ/2) learning: maximal at π, enables escape
    /// - Stuck detector: deterministic reset after 10 minutes at anti-phase
    /// - Post-stuck Δω cooldown: reduces drift after reset
    /// - Subdivided timestep for numerical stability
    pub fn tick_phase(&self, bond_level: BondLevel, dt_secs: f64) {
        let base_coupling = match bond_level {
            BondLevel::Stranger => COUPLING_STRANGER,
            BondLevel::Acquaintance => COUPLING_ACQUAINTANCE,
            BondLevel::Friend => COUPLING_FRIEND,
            BondLevel::Confidant => COUPLING_CONFIDANT,
            BondLevel::PartnerInCrime => COUPLING_PARTNER,
        };

        // v4: Fatigue removed from oscillator — coupling is pure bond-based.
        // Fatigue only affects downstream gate, preventing the vicious cycle
        // where high fatigue → low coupling → phase drifts → can't recover.
        let coupling = base_coupling;
        let k2 = SECOND_HARMONIC_RATIO * coupling;

        let user_freq = *self.user_frequency.lock().unwrap();
        let comp_freq = *self.companion_frequency.lock().unwrap();

        let base_delta_omega = (comp_freq - user_freq) * std::f64::consts::TAU / 3600.0;
        let mut adapt_dw = self.adaptive_delta_omega.lock().unwrap();

        // v4: Post-stuck cooldown — reduce base drift after a reset
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let stuck_reset = *self.stuck_reset_ts.lock().unwrap();
        let cooldown = if stuck_reset > 0.0 {
            (-(now - stuck_reset) / STUCK_COOLDOWN_TAU).exp()
        } else {
            0.0
        };
        let effective_base_dw = base_delta_omega * (1.0 - 0.7 * cooldown);
        let delta_omega = effective_base_dw + *adapt_dw;

        let mut phi = self.phase.lock().unwrap();
        let total_dt = dt_secs.min(120.0);

        // Subdivide for stability: dt_step * K < 1.0
        let n_steps = (total_dt * base_coupling.max(0.01)).ceil().max(1.0) as usize;
        let dt_step = total_dt / n_steps as f64;

        // Sigmoid helper for anti-phase gate
        fn sigmoid(x: f64) -> f64 {
            1.0 / (1.0 + (-x).exp())
        }

        for _ in 0..n_steps {
            let sin_phi = phi.sin();

            // v4: Generalized Kuramoto with second-harmonic coupling
            // dφ/dt = Δω - K₁·sin(φ) - K₂·sin(2φ)
            // Second harmonic makes φ=π locally unstable (slope 2K₂ > 0)
            *phi += dt_step * (delta_omega - coupling * sin_phi - k2 * (2.0 * *phi).sin());

            // v4: Adaptive Δω with gated sin(φ/2) for anti-phase escape
            // sin(φ/2) is maximal at φ=π, enabling learning when sin(φ)≈0
            // Gated so it only activates near anti-phase (|φ| > 2.3 rad)
            let near_antiphase = sigmoid(8.0 * (phi.abs() - 2.3));
            *adapt_dw += dt_step * (
                -ADAPTIVE_FREQ_LAMBDA * *adapt_dw
                + ADAPTIVE_FREQ_MU * sin_phi
                + ANTIPHASE_ESCAPE_MU * near_antiphase * (*phi / 2.0).sin()
            );
        }
        *phi = phi.rem_euclid(std::f64::consts::TAU);
        // Clamp adaptive Δω to prevent runaway
        *adapt_dw = adapt_dw.clamp(-0.1, 0.1);

        // v4: Stuck detector — deterministic reset after 10 consecutive
        // ticks (10 minutes) with |φ| near π. More reliable than random nudges.
        {
            let mut stuck = self.stuck_counter.lock().unwrap();
            // Check if phase is near anti-phase (φ in [π-0.64, π+0.64])
            let phi_centered = (*phi - std::f64::consts::PI).abs();
            if phi_centered < 0.64 {
                *stuck += 1;
                if *stuck >= 10 {
                    // Reset: pull 70% toward neutral, clear learned drift
                    *phi = *phi * 0.3;
                    *adapt_dw = 0.0;
                    *self.stuck_reset_ts.lock().unwrap() = now;
                    *stuck = 0;
                    tracing::info!("Kuramoto stuck detector: reset phase from anti-phase");
                }
            } else {
                *stuck = 0;
            }
        }
    }

    /// Compute the bond growth recommendation.
    ///
    /// Returns a signal in [-1.0, 1.0]:
    /// - Positive: relationship is healthy, invest more (deeper content, more frequent)
    /// - Zero: maintain current level
    /// - Negative: pull back (user seems disengaged, reduce depth/frequency)
    pub fn connection_growth_signal(&self) -> f64 {
        let qh = self.quality_history.lock().unwrap();
        if qh.len() < 3 {
            return 0.0; // Not enough data
        }

        // Compute trend: are recent interactions getting better or worse?
        let recent: Vec<f64> = qh.iter().rev().take(5).copied().collect();
        let older: Vec<f64> = qh.iter().rev().skip(5).take(10).copied().collect();

        let recent_avg = if recent.is_empty() {
            0.5
        } else {
            recent.iter().sum::<f64>() / recent.len() as f64
        };

        let older_avg = if older.is_empty() {
            0.5
        } else {
            older.iter().sum::<f64>() / older.len() as f64
        };

        // Trend: positive if improving, negative if declining
        let trend = recent_avg - older_avg;

        // Absolute level: are interactions generally positive?
        let level_signal = recent_avg - 0.5; // Centered at 0

        // Combined: 60% level + 40% trend
        let signal = 0.6 * level_signal + 0.4 * trend;
        signal.clamp(-1.0, 1.0)
    }

    /// Get the recommended communication cadence (messages per hour).
    ///
    /// Based on user frequency matching with bond-level dampening:
    /// - Stranger: 50% of user's rate (conservative)
    /// - Partner: 90% of user's rate (near-matching)
    pub fn recommended_cadence(&self, bond_level: BondLevel) -> f64 {
        let user_freq = *self.user_frequency.lock().unwrap();
        let ratio = match bond_level {
            BondLevel::Stranger => 0.5,
            BondLevel::Acquaintance => 0.6,
            BondLevel::Friend => 0.7,
            BondLevel::Confidant => 0.8,
            BondLevel::PartnerInCrime => 0.9,
        };
        (user_freq * ratio).max(0.1) // At least 1 message per 10 hours
    }

    // ── Diagnostics ─────────────────────────────────────────────────────

    /// Get current phase alignment as a human-readable description.
    pub fn phase_description(&self) -> &'static str {
        let phi = *self.phase.lock().unwrap();
        let alignment = (1.0 + phi.cos()) / 2.0;
        if alignment > 0.8 {
            "strongly synchronized"
        } else if alignment > 0.6 {
            "well aligned"
        } else if alignment > 0.4 {
            "moderately aligned"
        } else if alignment > 0.2 {
            "drifting apart"
        } else {
            "desynchronized"
        }
    }

    /// Get the current growth signal as a strategy recommendation.
    pub fn strategy_recommendation(&self) -> &'static str {
        let signal = self.connection_growth_signal();
        if signal > 0.3 {
            "invest: deepen content, increase frequency slightly"
        } else if signal > 0.0 {
            "maintain: current approach is working"
        } else if signal > -0.3 {
            "caution: reduce frequency, keep content lighter"
        } else {
            "pull back: minimal contact, surface-level only"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_novelty_empty_history() {
        let engine = ResonanceEngine::new();
        assert_eq!(engine.information_novelty("hello world", &[]), 1.0);
    }

    #[test]
    fn test_novelty_identical_message() {
        let engine = ResonanceEngine::new();
        let recent = vec!["hello world".to_string()];
        let novelty = engine.information_novelty("hello world", &recent);
        assert!(novelty < 0.1, "Identical message should have near-zero novelty");
    }

    #[test]
    fn test_novelty_different_message() {
        let engine = ResonanceEngine::new();
        let recent = vec!["the weather is sunny today".to_string()];
        let novelty = engine.information_novelty(
            "debugging rust code with lifetimes",
            &recent,
        );
        assert!(novelty > 0.8, "Unrelated message should have high novelty");
    }

    #[test]
    fn test_phase_alignment_initial() {
        let engine = ResonanceEngine::new();
        // Initial phase = 0 → cos(0) = 1 → alignment = 1.0
        assert!((engine.phase_alignment_score() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_desire_pressure_grows_with_silence() {
        let engine = ResonanceEngine::new();
        *engine.last_sent_ts.lock().unwrap() = 1000.0;

        let d1 = engine.desire_pressure(1060.0);   // 1 min silence
        let d2 = engine.desire_pressure(4600.0);   // 1 hour silence
        let d3 = engine.desire_pressure(11800.0);  // 3 hour silence

        assert!(d1 < d2, "Desire should grow with silence");
        assert!(d2 < d3, "Desire should continue growing");
        assert!(d3 < 1.0, "Desire should never reach 1.0");
    }

    #[test]
    fn test_bond_resonance_scales_with_score() {
        let engine = ResonanceEngine::new();
        let b_low = engine.bond_resonance(BondLevel::Stranger, 0.0);
        let b_high = engine.bond_resonance(BondLevel::PartnerInCrime, 1.0);

        assert!(b_low >= 0.3, "Even zero bond should allow some resonance");
        assert!((b_high - 1.0).abs() < 0.01, "Max bond should give full resonance");
        assert!(b_low < b_high, "Higher bond should give higher resonance");
    }

    #[test]
    fn test_fatigue_decays_over_time() {
        let engine = ResonanceEngine::new();
        let now = 10000.0;

        // Send 3 messages rapidly
        engine.sent_timestamps.lock().unwrap().extend_from_slice(
            &[now - 10.0, now - 5.0, now]
        );

        let f1 = engine.fatigue_factor(now);            // Just sent
        let f2 = engine.fatigue_factor(now + 3600.0);   // 1 hour later

        assert!(f1 > f2, "Fatigue should decay over time");
    }

    #[test]
    fn test_variety_bonus_rewards_switching() {
        let engine = ResonanceEngine::new();
        {
            let mut h = engine.category_history.lock().unwrap();
            h.push(InstinctCategory::Research);
            h.push(InstinctCategory::Research);
            h.push(InstinctCategory::Research);
        }

        let same = engine.variety_bonus("DeepDive"); // Research again
        let diff = engine.variety_bonus("GoalKeeper"); // Growth — different!

        assert!(diff > same, "Switching categories should give higher variety");
    }

    #[test]
    fn test_depth_appropriateness() {
        let engine = ResonanceEngine::new();

        // IdentityThread (Core depth) with Stranger bond → should be very low
        let d1 = engine.depth_appropriateness("IdentityThread", BondLevel::Stranger);
        assert!(d1 < 0.2, "Core instinct with Stranger should be suppressed");

        // WorldSense (Surface depth) with any bond → should be 1.0
        let d2 = engine.depth_appropriateness("WorldSense", BondLevel::Stranger);
        assert!((d2 - 1.0).abs() < 0.01, "Surface instinct always fits");

        // IdentityThread with Partner → should be 1.0
        let d3 = engine.depth_appropriateness("IdentityThread", BondLevel::PartnerInCrime);
        assert!((d3 - 1.0).abs() < 0.01, "Core instinct with Partner fits");
    }

    #[test]
    fn test_full_resonance_score() {
        let engine = ResonanceEngine::new();
        *engine.last_sent_ts.lock().unwrap() = 5000.0;

        let score = engine.score(
            "WorldSense",
            0.5,
            "Interesting development in AI regulation today",
            &["The weather looks nice this morning".to_string()],
            BondLevel::Friend,
            0.5,
            10000.0,
        );

        assert!(score.score > 0.0, "Score should be positive");
        assert!(score.score <= 1.0, "Score should not exceed 1.0");
        assert!(!score.suppress, "Should not be suppressed");
        assert!(score.novelty > 0.5, "Should have decent novelty");
    }

    #[test]
    fn test_kuramoto_phase_evolution() {
        let engine = ResonanceEngine::new();

        // Set a non-zero phase
        *engine.phase.lock().unwrap() = 1.0; // Out of sync

        // Tick with strong coupling (Partner level)
        for _ in 0..100 {
            engine.tick_phase(BondLevel::PartnerInCrime, 60.0);
        }

        // Phase should have moved toward 0 (synchronized)
        let phi = *engine.phase.lock().unwrap();
        let alignment = (1.0 + phi.cos()) / 2.0;
        assert!(
            alignment > 0.5,
            "Strong coupling should pull phase toward alignment (got {})",
            alignment
        );
    }

    #[test]
    fn test_connection_growth_signal_positive() {
        let engine = ResonanceEngine::new();
        {
            let mut qh = engine.quality_history.lock().unwrap();
            // Old interactions: mediocre
            for _ in 0..5 {
                qh.push(0.4);
            }
            // Recent interactions: great
            for _ in 0..5 {
                qh.push(0.9);
            }
        }

        let signal = engine.connection_growth_signal();
        assert!(signal > 0.0, "Improving quality should give positive signal");
    }
}
