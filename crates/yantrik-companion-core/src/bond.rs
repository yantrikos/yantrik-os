//! Bond level types — shared across all companion modules.

use serde::{Deserialize, Serialize};

/// Bond level enum — unlocks progressively more personality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum BondLevel {
    Stranger = 1,
    Acquaintance = 2,
    Friend = 3,
    Confidant = 4,
    PartnerInCrime = 5,
}

impl BondLevel {
    pub fn from_score(score: f64) -> Self {
        match score {
            s if s < 0.5 => BondLevel::Stranger,
            s if s < 1.5 => BondLevel::Acquaintance,
            s if s < 2.5 => BondLevel::Friend,
            s if s < 3.5 => BondLevel::Confidant,
            _ => BondLevel::PartnerInCrime,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            BondLevel::Stranger => "Stranger",
            BondLevel::Acquaintance => "Acquaintance",
            BondLevel::Friend => "Friend",
            BondLevel::Confidant => "Confidant",
            BondLevel::PartnerInCrime => "Partner-in-Crime",
        }
    }

    pub fn as_u8(&self) -> u8 {
        *self as u8
    }
}

/// Snapshot of bond state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BondState {
    pub bond_score: f64,
    pub bond_level: BondLevel,
    pub total_interactions: i64,
    pub vulnerability_events: i64,
    pub humor_successes: i64,
    pub humor_attempts: i64,
    pub deep_conversations: i64,
    pub shared_references: i64,
    pub current_streak_days: i64,
    pub longest_streak_days: i64,
    pub first_interaction_at: Option<f64>,
    pub days_together: f64,
}
