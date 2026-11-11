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

/// Snapshot of the bond.
///
/// The score is a sum of receipts and nothing else (see [`ReceiptKind`]). `total_interactions`,
/// `first_interaction_at` and `days_together` are presence facts: they say the person was here,
/// and never feed the score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BondState {
    pub bond_score: f64,
    pub bond_level: BondLevel,
    /// How many receipts the score is made of.
    pub receipts: i64,
    pub total_interactions: i64,
    pub first_interaction_at: Option<f64>,
    pub days_together: f64,
}

/// Highest the score goes, and the point of [`BondLevel::PartnerInCrime`].
pub const SCORE_CEILING: f64 = 5.0;

/// How many allowed cards in a row, for one app and action, before the companion may *offer*
/// to stop asking. A refusal puts the count back to zero.
pub const OFFER_CLEAN_STREAK: u32 = 4;

/// The things a person did that the bond is made of. Nothing they said is on this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptKind {
    /// A card was Allowed.
    CardAllowed,
    /// A card was refused.
    CardRefused,
    /// A For-you item was marked Useful.
    ForYouUseful,
    /// A For-you item was marked Not useful.
    ForYouNotUseful,
    /// A prepared item was opened or used.
    PreparedUsed,
    /// Something the machine did was undone.
    Undone,
}

impl ReceiptKind {
    /// How far the score moves. Falls are larger than rises: trust comes slowly and goes fast.
    pub fn delta(self) -> f64 {
        match self {
            Self::CardAllowed => 0.10,
            Self::ForYouUseful | Self::PreparedUsed => 0.05,
            Self::CardRefused => -0.20,
            Self::ForYouNotUseful => -0.10,
            Self::Undone => -0.15,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CardAllowed => "card_allowed",
            Self::CardRefused => "card_refused",
            Self::ForYouUseful => "for_you_useful",
            Self::ForYouNotUseful => "for_you_not_useful",
            Self::PreparedUsed => "prepared_used",
            Self::Undone => "undone",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        [
            Self::CardAllowed,
            Self::CardRefused,
            Self::ForYouUseful,
            Self::ForYouNotUseful,
            Self::PreparedUsed,
            Self::Undone,
        ]
        .into_iter()
        .find(|k| k.as_str() == word)
    }

    /// Whether this receipt lowers the score.
    pub fn is_fall(self) -> bool {
        self.delta() < 0.0
    }
}

/// One receipt: what the person did, to which app and action, and the id of the record it came
/// from so the same press is never counted twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub kind: ReceiptKind,
    /// The app the card or item belonged to ("notes"), empty for a receipt with no app.
    pub app: String,
    /// The action, ("create"), empty for a receipt with no action.
    pub action: String,
    /// Whether the action sends something off this machine. Such rows are never offered.
    pub leaves_machine: bool,
    /// The id of the approval card, notice or prepared item. Recording the same id twice is a
    /// no-op.
    pub source_id: String,
}

impl Receipt {
    /// The sentence stored beside a receipt that lowered the score: why it fell.
    pub fn why_it_fell(&self) -> Option<String> {
        if !self.kind.is_fall() {
            return None;
        }
        let target = if self.app.is_empty() {
            String::new()
        } else {
            format!(" ({} · {})", self.app, self.action)
        };
        Some(match self.kind {
            ReceiptKind::CardRefused => format!("You refused a card{target}."),
            ReceiptKind::ForYouNotUseful => "You marked an item Not useful.".to_string(),
            _ => format!("You undid something the machine did{target}."),
        })
    }
}

/// One line of the table: what was asked of one app, for one family of action, and what the
/// person answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BondRow {
    pub app: String,
    pub action: String,
    pub asked: u32,
    pub allowed: u32,
    pub refused: u32,
    /// Allowed answers since the last refusal.
    pub clean_streak: u32,
    pub leaves_machine: bool,
    /// The receipts behind the counts, newest first, as `source_id`s.
    pub receipts: Vec<String>,
}

impl BondRow {
    /// Whether the companion may offer to stop asking: a clean streak of
    /// [`OFFER_CLEAN_STREAK`], and the action stays on this machine.
    pub fn offerable(&self) -> bool {
        !self.leaves_machine && self.clean_streak >= OFFER_CLEAN_STREAK
    }

    /// The offer in words, or `None` when there is none.
    pub fn offer(&self) -> Option<String> {
        self.offerable().then(|| {
            format!(
                "{} {} has been allowed {n} of {n} times. Stop asking for this session?",
                title_case(&self.app),
                self.action,
                n = self.clean_streak,
            )
        })
    }

    /// The row as the table reads it: "Notes · create: asked 4, allowed 4".
    pub fn line(&self) -> String {
        let mut line = format!(
            "{} · {}: asked {}, allowed {}",
            title_case(&self.app),
            self.action,
            self.asked,
            self.allowed
        );
        if self.refused > 0 {
            line.push_str(&format!(", refused {}", self.refused));
        }
        line
    }
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(streak: u32, leaves: bool) -> BondRow {
        BondRow {
            app: "notes".into(),
            action: "create".into(),
            asked: streak,
            allowed: streak,
            refused: 0,
            clean_streak: streak,
            leaves_machine: leaves,
            receipts: vec![],
        }
    }

    #[test]
    fn the_offer_needs_the_whole_streak_and_a_stay_at_home_action() {
        assert!(row(3, false).offer().is_none());
        assert_eq!(
            row(4, false).offer().as_deref(),
            Some("Notes create has been allowed 4 of 4 times. Stop asking for this session?")
        );
        assert!(row(9, true).offer().is_none(), "what leaves the machine is never offered");
    }

    #[test]
    fn the_row_reads_like_a_statement() {
        assert_eq!(row(4, false).line(), "Notes · create: asked 4, allowed 4");
    }

    #[test]
    fn only_falls_carry_a_reason() {
        let mut r = Receipt {
            kind: ReceiptKind::CardRefused,
            app: "email".into(),
            action: "send".into(),
            leaves_machine: true,
            source_id: "c1".into(),
        };
        assert!(r.why_it_fell().unwrap().contains("email · send"));
        r.kind = ReceiptKind::CardAllowed;
        assert!(r.why_it_fell().is_none());
    }

    #[test]
    fn every_kind_round_trips_its_word() {
        for kind in [ReceiptKind::CardAllowed, ReceiptKind::Undone, ReceiptKind::PreparedUsed] {
            assert_eq!(ReceiptKind::parse(kind.as_str()), Some(kind));
        }
    }
}
