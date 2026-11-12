//! What the shell's worker calls for commitments (#1184): note what the person promised in a
//! message, and carry out an urge that asks for a promise to be prepared.
//!
//! They are here, as functions of what they read, so they are tested without the shell; the worker
//! loop only calls them. Neither runs in incognito: nothing the person says is turned into a
//! commitment, and nothing is prepared, while the companion is not to remember.

use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

use crate::anticipation::{now_ts, AnticipationEngine};
use crate::commitment_extractor::{extract_commitments_at, to_world_model_commitments};
use crate::commitment_prep::{commitment_to_prepare, prepare_with, Outcome, Settings};
use crate::companion::CompanionService;
use crate::types::UrgeSpec;
use crate::world_model::{CommitmentSource, WorldModel};

/// Store the commitments a message holds, as of `now`. How many were stored.
pub fn note_commitments_in(
    conn: &rusqlite::Connection,
    incognito: bool,
    user_name: &str,
    text: &str,
    turn_id: u64,
    now: f64,
) -> usize {
    if incognito {
        return 0;
    }
    let source = CommitmentSource::Conversation { turn_id: Some(turn_id.to_string()) };
    let extracted = extract_commitments_at(text, user_name, &source, now);
    let commitments = to_world_model_commitments(&extracted, user_name, source);
    for c in &commitments {
        WorldModel::insert_commitment(conn, c);
    }
    if !commitments.is_empty() {
        tracing::info!(count = commitments.len(), "Extracted commitments from user message");
    }
    commitments.len()
}

/// Carry out `urge` if it asks for a promise to be prepared. `true` when it did, so the caller
/// does not hand it on to the model: preparing opens files and drafts, says nothing to the person,
/// and does not spend the daily message budget.
pub fn run_urge(registry: &ToolRegistry, ctx: &ToolContext<'_>, urge: &UrgeSpec, now: f64, settings: &Settings) -> bool {
    let Some(commitment_id) = commitment_to_prepare(urge) else { return false };
    match prepare_with(registry, ctx, commitment_id, now, settings) {
        Outcome::Prepared(item) => {
            tracing::info!(commitment_id, state = %item.data["state"], summary = %item.description, "Commitment prepared");
        }
        Outcome::AlreadyPrepared => {}
        Outcome::Nothing(why) => tracing::debug!(commitment_id, why, "Commitment not prepared"),
    }
    true
}

impl CompanionService {
    /// Note the promises in a message the person sent.
    pub fn note_commitments(&self, text: &str, turn_id: u64) -> usize {
        note_commitments_in(&self.db.conn(), self.incognito, &self.config.user_name, text, turn_id, now_ts())
    }

    /// Carry out `urge` if it is a promise to prepare; see [`run_urge`]. The ceiling is Standard,
    /// which is all `glob` and `app_action` need; nothing here is above it. One run is held to
    /// [`crate::commitment_prep::BUDGET`], so it cannot keep the think loop for long.
    pub fn run_commitment_urge(&self, urge: &UrgeSpec) -> bool {
        let ctx = ToolContext {
            db: &self.db,
            max_permission: self.max_permission_now().min(PermissionLevel::Standard),
            registry_metadata: None,
            task_manager: Some(&self.task_manager),
            incognito: self.incognito,
            agent_spawner: None,
        };
        run_urge(&self.registry, &ctx, urge, now_ts(), &Settings::for_person())
    }

    /// The brief card's rows about preparing: the question, shown (which is what marks it asked), or
    /// the way back after the person turned it off. Their buttons are person-only
    /// (`commitment_consent`).
    pub fn commitment_brief_actions(&self) -> Vec<crate::commitment_consent::BriefAction> {
        if self.incognito {
            return Vec::new();
        }
        crate::commitment_consent::brief_actions(&self.db.conn(), now_ts())
    }

    /// The person pressed a brief-card button about preparing. Called only from the shell's click
    /// handler; what was said back, if the click answered anything.
    pub fn person_answers_commitment_prep(&self, action_id: &str) -> Option<String> {
        crate::commitment_consent::person_answer(&self.db.conn(), action_id)
    }

    /// Housekeeping for what was prepared: expired items are closed, and old ones deleted.
    pub fn commitment_housekeeping(&self) -> u64 {
        // A draft whose item has expired, or whose promise was forgotten, is cleared from Email.
        crate::commitment_slot::owe_for_expired(&self.db.conn(), now_ts());
        let ctx = ToolContext {
            db: &self.db,
            max_permission: self.max_permission_now().min(PermissionLevel::Standard),
            registry_metadata: None,
            task_manager: Some(&self.task_manager),
            incognito: self.incognito,
            agent_spawner: None,
        };
        if !crate::commitment_slot::owed(&self.db.conn()).is_empty() && !self.incognito {
            crate::commitment_slot::pay(&self.registry, &ctx, crate::commitment_prep::BUDGET);
        }
        AnticipationEngine::cleanup_expired(&self.db.conn())
    }
}
