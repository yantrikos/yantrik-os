//! Bond level types — shared across all companion modules.

use serde::{Deserialize, Serialize};

use crate::text::capitalize;

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

/// The most one (app, action) pair can add to the score, however many cards it raises: four
/// allows' worth, which stays below the first level (0.5), so no one pair buys a level. A Mind can
/// raise a trivial card as often as it likes, and a person who has stopped reading it allows each
/// one. A pair's falls are not capped. A For-you item counts under its sender harness, so a Mind
/// posting many notices is held to the same cap.
pub const PAIR_RISE_CAP: f64 = 0.4;

/// The most all cards together can add to the score, across every (app, action) pair: the same
/// four allows' worth, below the first level. Two trivial pairs, or a Mind raising cards under
/// many names, cannot reach a level by cards alone; what lifts the bond past it is a person's
/// feedback on For-you items and prepared work, which stays capped per sender.
pub const CARD_RISE_CAP: f64 = 0.4;

/// How long a receipt is kept. Receipts older than that, by the clock, are dropped when the app starts and when one is written, and the
/// person can drop them all at once (`BondTracker::forget_receipts`).
pub const RECEIPT_RETENTION_DAYS: i64 = 180;

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
    /// Something the machine did on its own (a Mind's archive or trash, a wired model) was undone
    /// by the person. Undoing what the person did themselves is not a receipt, and no path from
    /// the audit of unasked actions undoes anything yet.
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

/// What the app says about one action, as the card carried it. Stored with each receipt so the
/// table can say, from the same facts the gate decides on, whether the action still asks and
/// whether it may be offered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionFacts {
    /// The action's grade (`safe`, `sensitive`, `dangerous`); empty when unknown.
    pub grade: String,
    /// The app says it cannot be taken back.
    pub cannot_be_undone: bool,
    /// The app says it runs whatever it is given.
    pub open_ended: bool,
    /// The app says it fetches an address it is handed.
    pub fetches_address: bool,
    /// The app says it sends something off this machine.
    pub leaves_machine: bool,
    /// The card drew no "Allow for this session" button (`Card::can_session` was false): it
    /// carried a per-call sentence or was a one-time question. The inverse of `can_session`, so
    /// that an unknown answer reads as "may offer" only through the other guards, and one card
    /// without the button closes the offer for good.
    pub no_session_rule: bool,
}

impl ActionFacts {
    /// Fold a newer answer's facts in, keeping the cautious side of each: what any answer said
    /// about leaving the machine, fetching an address or being irreversible stays said, and a
    /// `dangerous` grade stays `dangerous`. An app that drops a sentence for a moment does not
    /// make its action offerable.
    pub fn absorb(&mut self, newer: &ActionFacts) {
        if self.grade != "dangerous" && !newer.grade.is_empty() {
            self.grade = newer.grade.clone();
        }
        self.cannot_be_undone |= newer.cannot_be_undone;
        self.open_ended |= newer.open_ended;
        self.fetches_address |= newer.fetches_address;
        self.leaves_machine |= newer.leaves_machine;
        self.no_session_rule |= newer.no_session_rule;
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
    /// What the app said about the action when it was asked.
    pub facts: ActionFacts,
    /// The id of the approval card, notice or prepared item. Recording the same id twice is a
    /// no-op. Empty means the receipt has no record of its own (an undo), and the store gives
    /// it an id: every one of those is its own receipt.
    pub source_id: String,
}

impl Receipt {
    /// A receipt about no particular action: a For-you item, a prepared item.
    pub fn bare(kind: ReceiptKind, source_id: impl Into<String>) -> Self {
        Receipt {
            kind,
            app: String::new(),
            action: String::new(),
            facts: ActionFacts::default(),
            source_id: source_id.into(),
        }
    }

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

/// The score as receipts add up: each in the order it happened, the running total held between
/// zero and [`SCORE_CEILING`], and no (app, action) pair adding more than [`PAIR_RISE_CAP`].
/// Time never enters.
#[derive(Debug, Default)]
pub struct ScoreFold {
    score: f64,
    receipts: i64,
    risen: std::collections::HashMap<(String, String), f64>,
    cards_risen: f64,
}

impl ScoreFold {
    /// Add one receipt's `delta`, made by `app` and `action` (empty for neither).
    pub fn add(&mut self, delta: f64, app: &str, action: &str) {
        self.fold(delta, app, action, false);
    }

    /// Add the receipt of a card's answer: capped per pair, and with every other card's rises
    /// under [`CARD_RISE_CAP`] in all.
    pub fn add_card(&mut self, delta: f64, app: &str, action: &str) {
        self.fold(delta, app, action, true);
    }

    fn fold(&mut self, delta: f64, app: &str, action: &str, card: bool) {
        self.receipts += 1;
        let delta = if delta > 0.0 && !app.is_empty() {
            let risen = self.risen.entry((app.to_string(), action.to_string())).or_insert(0.0);
            let mut room = (PAIR_RISE_CAP - *risen).max(0.0);
            if card {
                room = room.min((CARD_RISE_CAP - self.cards_risen).max(0.0));
            }
            let taken = delta.min(room);
            *risen += taken;
            if card {
                self.cards_risen += taken;
            }
            taken
        } else {
            delta
        };
        self.score = (self.score + delta).clamp(0.0, SCORE_CEILING);
    }

    /// The score and how many receipts it is made of.
    pub fn finish(self) -> (f64, i64) {
        (self.score, self.receipts)
    }
}

/// One line of the table: what was asked of one app, for one family of action, and what the
/// person answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BondRow {
    pub app: String,
    pub action: String,
    /// Cards the person answered. A card that expired or was withdrawn is not here.
    pub answered: u32,
    pub allowed: u32,
    pub refused: u32,
    /// Allowed answers since the last refusal.
    pub clean_streak: u32,
    /// What the newest answer's card said, which is what the gate would read for this action now.
    pub facts: ActionFacts,
    /// What every answer's card said, the cautious side of each ([`ActionFacts::absorb`]). The
    /// offer is read from this.
    pub ever: ActionFacts,
    /// The receipts behind the counts, newest first, as `source_id`s.
    pub receipts: Vec<String>,
}

impl BondRow {
    /// Whether the companion may offer to stop asking: a clean streak of
    /// [`OFFER_CLEAN_STREAK`], and an action the session rule could actually cover.
    ///
    /// `may_offer_session_rule` is the shell's own test for whether the card draws "Allow for
    /// this session", given a grade and whether the app says it cannot be undone
    /// (`approvals_bond::session_rule_allowed`), so the offer is never wider than the button that
    /// would apply it, and no card for the action ever lacked that button (`ever.no_session_rule`). On top of that, nothing that ever left the machine or fetched an address
    /// is offered, and nothing whose grade was never recorded.
    pub fn offerable(&self, may_offer_session_rule: impl Fn(&str, bool) -> bool) -> bool {
        self.clean_streak >= OFFER_CLEAN_STREAK
            && !self.ever.grade.is_empty()
            && !self.ever.leaves_machine
            && !self.ever.fetches_address
            && !self.ever.no_session_rule
            && may_offer_session_rule(&self.ever.grade, self.ever.cannot_be_undone)
    }

    /// The offer in words, or `None` when there is none.
    pub fn offer(&self, may_offer_session_rule: impl Fn(&str, bool) -> bool) -> Option<String> {
        self.offerable(may_offer_session_rule).then(|| {
            format!(
                "{} {} has been allowed the last {} times. Stop asking for this session?",
                capitalize(&self.app),
                self.action,
                self.clean_streak,
            )
        })
    }

    /// The row as the table reads it: "Notes · create: answered 4, allowed 4".
    pub fn line(&self) -> String {
        let mut line = format!(
            "{} · {}: answered {}, allowed {}",
            capitalize(&self.app),
            self.action,
            self.answered,
            self.allowed
        );
        if self.refused > 0 {
            line.push_str(&format!(", refused {}", self.refused));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(grade: &str) -> ActionFacts {
        ActionFacts { grade: grade.into(), ..Default::default() }
    }

    fn row(streak: u32, ever: ActionFacts) -> BondRow {
        BondRow {
            app: "notes".into(),
            action: "create".into(),
            answered: streak,
            allowed: streak,
            refused: 0,
            clean_streak: streak,
            facts: ever.clone(),
            ever,
            receipts: vec![],
        }
    }

    /// The shell's rule, as `approvals_bond::session_rule_allowed` has it.
    fn shell_allows(grade: &str, cannot_be_undone: bool) -> bool {
        grade != "dangerous" && !cannot_be_undone
    }

    #[test]
    fn the_offer_needs_the_whole_streak_and_a_stay_at_home_action() {
        assert!(row(3, facts("sensitive")).offer(shell_allows).is_none());
        assert_eq!(
            row(4, facts("sensitive")).offer(shell_allows).as_deref(),
            Some("Notes create has been allowed the last 4 times. Stop asking for this session?")
        );
        let sends = ActionFacts { leaves_machine: true, ..facts("sensitive") };
        assert!(row(9, sends).offer(shell_allows).is_none(), "what leaves the machine is never offered");
        let fetches = ActionFacts { fetches_address: true, ..facts("sensitive") };
        assert!(row(9, fetches).offer(shell_allows).is_none(), "nor what fetches an address");
    }

    #[test]
    fn the_offer_is_no_wider_than_the_session_button() {
        let gone = ActionFacts { cannot_be_undone: true, ..facts("sensitive") };
        assert!(row(8, gone).offer(shell_allows).is_none(), "what cannot be undone has no session button");
        assert!(row(8, facts("dangerous")).offer(shell_allows).is_none(), "nor what is dangerous");
        assert!(row(8, ActionFacts::default()).offer(shell_allows).is_none(), "an unknown grade proves nothing");
    }

    #[test]
    fn the_cap_is_below_the_first_level() {
        assert!(PAIR_RISE_CAP < 0.5);
        let mut fold = ScoreFold::default();
        for _ in 0..1000 {
            fold.add(0.10, "notes", "create");
        }
        assert_eq!(BondLevel::from_score(fold.finish().0), BondLevel::Stranger);
    }

    #[test]
    fn a_card_without_the_session_button_closes_the_offer() {
        let mut ever = facts("sensitive");
        ever.absorb(&ActionFacts { no_session_rule: true, ..facts("sensitive") });
        ever.absorb(&facts("sensitive"));
        assert!(ever.no_session_rule, "sticky: a later card with the button does not reopen it");
        assert!(row(9, ever).offer(shell_allows).is_none());
    }

    #[test]
    fn absorbing_keeps_the_cautious_side() {
        let mut ever = ActionFacts { leaves_machine: true, ..facts("dangerous") };
        ever.absorb(&facts("sensitive"));
        assert!(ever.leaves_machine && ever.grade == "dangerous");
    }

    #[test]
    fn a_pair_adds_no_more_than_its_cap_and_its_falls_are_not_capped() {
        let mut fold = ScoreFold::default();
        for _ in 0..100 {
            fold.add(0.10, "notes", "create");
        }
        assert_eq!(fold.score, PAIR_RISE_CAP);
        fold.add(-0.20, "notes", "create");
        assert!((fold.score - 0.2).abs() < 1e-9);
        fold.add(0.10, "", "");
        assert!((fold.score - 0.3).abs() < 1e-9, "a receipt with no pair is not capped");
    }

    #[test]
    fn the_row_reads_like_a_statement() {
        assert_eq!(row(4, facts("sensitive")).line(), "Notes · create: answered 4, allowed 4");
    }

    #[test]
    fn only_falls_carry_a_reason() {
        let mut r = Receipt::bare(ReceiptKind::CardRefused, "c1");
        r.app = "email".into();
        r.action = "send".into();
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
