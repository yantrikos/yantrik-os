//! Proactive Feature System — modular, pure, testable.
//!
//! Each feature implements `ProactiveFeature`. The `FeatureRegistry` holds
//! all features and routes events to them. The `UrgencyScorer` filters
//! urges before they reach the UI.
//!
//! Adding a feature = one file + implement trait + one register call.

pub mod resource_guardian;
pub mod process_sentinel;
pub mod error_companion;
pub mod focus_flow;
pub mod notification_relay;
pub mod tool_suggester;
pub mod network_watcher;
pub mod clipboard_intelligence;

use std::collections::HashMap;
use yantrik_os::{SystemEvent, SystemSnapshot};

// ── Urge types ──

/// An urge produced by a feature. The UI decides how to render it.
#[derive(Debug, Clone)]
pub struct Urge {
    /// Unique ID for deduplication and feedback tracking.
    pub id: String,
    /// Which feature produced this urge.
    pub source: String,
    /// Human-readable title (short, for Whisper Card heading).
    pub title: String,
    /// Human-readable body (1-2 sentences).
    pub body: String,
    /// Raw urgency score (0.0 - 1.0). Higher = more urgent.
    pub urgency: f32,
    /// Confidence in the assessment (0.0 - 1.0).
    pub confidence: f32,
    /// Category for icon/color selection.
    pub category: UrgeCategory,
}

/// Categories for urge rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrgeCategory {
    /// System resource warning (battery, CPU, disk, RAM).
    Resource,
    /// Security / process anomaly.
    Security,
    /// File lifecycle (stale downloads, large files).
    FileManagement,
    /// Focus / productivity.
    Focus,
    /// Positive feedback / celebration.
    Celebration,
    /// Shell command error.
    Shell,
    /// App notification (from D-Bus notification daemon).
    Notification,
}

/// What the user did with an urge.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// User acted on it (clicked, followed advice).
    Acted,
    /// User explicitly dismissed it.
    Dismissed,
    /// Urge expired without interaction.
    Expired,
}

/// A scored urge ready for UI display.
#[derive(Debug, Clone)]
pub struct ScoredUrge {
    pub urge: Urge,
    /// Final pressure score: urgency × confidence × interruptibility.
    pub pressure: f32,
    /// Display tier based on pressure.
    pub tier: UrgeTier,
}

/// How aggressively to display the urge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrgeTier {
    /// Floating card + optional sound (pressure >= 0.85).
    Interrupt,
    /// Subtle card, no sound (pressure >= 0.50).
    Whisper,
    /// Only visible in Quiet Queue (pressure >= 0.25).
    Queue,
    /// Drop entirely (pressure < 0.25).
    Drop,
}

// ── Feature context ──

/// Read-only context passed to features during evaluation.
/// Features can read system state but cannot mutate anything.
pub struct FeatureContext<'a> {
    pub system: &'a SystemSnapshot,
    pub clock: std::time::SystemTime,
    /// Current bond level (V15: for bond-aware message personality).
    pub bond_level: u8,
}

// ── ProactiveFeature trait ──

/// A proactive feature that observes system events and produces urges.
///
/// Invariants:
/// - `on_event` and `on_tick` are pure: (events, state) → urges.
/// - Features cannot mutate UI, system, or memory directly.
/// - Features can maintain internal state (cooldowns, counters).
pub trait ProactiveFeature: Send {
    /// Feature name (for logging and config).
    fn name(&self) -> &str;

    /// React to a specific system event. Return urges if warranted.
    fn on_event(&mut self, event: &SystemEvent, ctx: &FeatureContext) -> Vec<Urge>;

    /// Periodic tick (called every poll cycle). For time-based logic.
    fn on_tick(&mut self, ctx: &FeatureContext) -> Vec<Urge>;

    /// Feedback: the user interacted with an urge this feature produced.
    fn on_feedback(&mut self, _urge_id: &str, _outcome: Outcome) {
        // Default: ignore feedback. Features can override to adapt.
    }
}

// ── Feature Registry ──

/// Holds all registered features and routes events to them.
pub struct FeatureRegistry {
    features: Vec<Box<dyn ProactiveFeature>>,
}

impl FeatureRegistry {
    pub fn new() -> Self {
        Self {
            features: Vec::new(),
        }
    }

    /// Register a new feature.
    pub fn register(&mut self, feature: Box<dyn ProactiveFeature>) {
        tracing::info!(feature = feature.name(), "Registered proactive feature");
        self.features.push(feature);
    }

    /// Process a system event through all features. Returns all urges.
    pub fn process_event(&mut self, event: &SystemEvent, ctx: &FeatureContext) -> Vec<Urge> {
        let mut urges = Vec::new();
        for feature in &mut self.features {
            urges.extend(feature.on_event(event, ctx));
        }
        urges
    }

    /// Tick all features. Returns all urges.
    pub fn tick(&mut self, ctx: &FeatureContext) -> Vec<Urge> {
        let mut urges = Vec::new();
        for feature in &mut self.features {
            urges.extend(feature.on_tick(ctx));
        }
        urges
    }

    /// Route feedback to the feature that produced the urge.
    pub fn feedback(&mut self, urge_id: &str, source: &str, outcome: Outcome) {
        for feature in &mut self.features {
            if feature.name() == source {
                feature.on_feedback(urge_id, outcome);
                return;
            }
        }
    }
}

// ── Urgency Scorer ──

/// Filters and scores urges before they reach the UI.
pub struct UrgencyScorer {
    /// User's current interruptibility (0.0 = do not disturb, 1.0 = fully available).
    interruptibility: f32,
    /// Recent urge IDs to prevent duplicates within a cooldown window.
    recent_urges: HashMap<String, std::time::Instant>,
    /// Cooldown duration for deduplication.
    cooldown: std::time::Duration,
}

impl UrgencyScorer {
    pub fn new() -> Self {
        Self {
            interruptibility: 1.0,
            recent_urges: HashMap::new(),
            cooldown: std::time::Duration::from_secs(300), // 5 min dedup window
        }
    }

    /// Current interruptibility level.
    pub fn interruptibility(&self) -> f32 {
        self.interruptibility
    }

    /// Set the user's interruptibility level (from FocusFlow or manual).
    pub fn set_interruptibility(&mut self, level: f32) {
        self.interruptibility = level.clamp(0.0, 1.0);
    }

    /// Score and filter a batch of urges. Returns only those above the Drop threshold.
    pub fn score(&mut self, urges: Vec<Urge>) -> Vec<ScoredUrge> {
        let now = std::time::Instant::now();

        // Clean expired entries
        self.recent_urges.retain(|_, t| now.duration_since(*t) < self.cooldown);

        let mut scored = Vec::new();
        for urge in urges {
            // Dedup check
            if self.recent_urges.contains_key(&urge.id) {
                continue;
            }

            let pressure = urge.urgency * urge.confidence * self.interruptibility;
            let tier = match pressure {
                p if p >= 0.85 => UrgeTier::Interrupt,
                p if p >= 0.50 => UrgeTier::Whisper,
                p if p >= 0.25 => UrgeTier::Queue,
                _ => UrgeTier::Drop,
            };

            if tier != UrgeTier::Drop {
                self.recent_urges.insert(urge.id.clone(), now);
                scored.push(ScoredUrge {
                    urge,
                    pressure,
                    tier,
                });
            }
        }

        // Sort by pressure descending
        scored.sort_by(|a, b| b.pressure.partial_cmp(&a.pressure).unwrap_or(std::cmp::Ordering::Equal));
        scored
    }
}
