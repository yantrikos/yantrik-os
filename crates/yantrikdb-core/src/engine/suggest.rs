//! Engine-level suggest_next_step() API.
//!
//! Wires the full cognitive pipeline into `YantrikDB` methods that
//! load nodes and edges from the database and run the pipeline.

use crate::error::Result;
use crate::state::{CognitiveEdgeKind, CognitiveNode, NodeKind};
use crate::suggest::{self, ExecutionMode, NextStepRequest, NextStepResponse};

use super::{now, YantrikDB};

impl YantrikDB {
    /// Run the full suggest_next_step() pipeline on the cognitive graph.
    ///
    /// This is the primary public API for the cognitive kernel.
    /// Loads all relevant nodes and edges, then runs the 4-stage pipeline.
    pub fn suggest_next_step(&self, request: &NextStepRequest) -> Result<NextStepResponse> {
        // Load all cognitive nodes
        let mut all_nodes: Vec<CognitiveNode> = Vec::new();
        for kind in NodeKind::ALL.iter() {
            all_nodes.extend(self.load_cognitive_nodes_by_kind(*kind)?);
        }

        // Load relevant edges
        let mut all_edges = Vec::new();
        let relevant_edge_kinds = [
            CognitiveEdgeKind::AdvancesGoal,
            CognitiveEdgeKind::BlocksGoal,
            CognitiveEdgeKind::Triggers,
            CognitiveEdgeKind::Supports,
            CognitiveEdgeKind::Contradicts,
            CognitiveEdgeKind::AssociatedWith,
            CognitiveEdgeKind::Prefers,
            CognitiveEdgeKind::Avoids,
            CognitiveEdgeKind::Constrains,
        ];
        for kind in &relevant_edge_kinds {
            all_edges.extend(self.load_cognitive_edges_by_kind(*kind)?);
        }

        let node_refs: Vec<&CognitiveNode> = all_nodes.iter().collect();
        let response = suggest::suggest_next_step(request, &node_refs, &all_edges);

        Ok(response)
    }

    /// Quick suggest with default settings and current timestamp.
    pub fn suggest_next_step_auto(&self) -> Result<NextStepResponse> {
        let request = NextStepRequest {
            now: now(),
            ..NextStepRequest::default()
        };
        self.suggest_next_step(&request)
    }

    /// Suggest with a specific execution mode.
    pub fn suggest_next_step_mode(&self, mode: ExecutionMode) -> Result<NextStepResponse> {
        let request = NextStepRequest {
            now: now(),
            mode,
            ..NextStepRequest::default()
        };
        self.suggest_next_step(&request)
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use crate::engine::YantrikDB;
    use crate::policy::PolicyContext;
    use crate::state::*;
    use crate::suggest::{ExecutionMode, NextStepRequest};

    fn test_db() -> YantrikDB {
        YantrikDB::new(":memory:", 8).unwrap()
    }

    #[test]
    fn test_suggest_empty_graph() {
        let db = test_db();
        let resp = db.suggest_next_step_auto().unwrap();

        assert!(resp.chosen.is_none());
        assert_eq!(resp.metrics.intents_count, 0);
    }

    #[test]
    fn test_suggest_with_goal() {
        let db = test_db();
        let mut alloc = NodeIdAllocator::new();

        let goal_id = alloc.alloc(NodeKind::Goal);
        let mut goal = CognitiveNode::new(
            goal_id,
            "Ship feature".to_string(),
            NodePayload::Goal(GoalPayload {
                description: "Ship feature".to_string(),
                status: GoalStatus::Active,
                progress: 0.5,
                deadline: None,
                priority: Priority::Critical,
                parent_goal: None,
                completion_criteria: "Deployed".to_string(),
            }),
        );
        goal.attrs.urgency = 0.85;
        db.persist_cognitive_node(&goal).unwrap();
        db.persist_node_id_allocator(&alloc).unwrap();

        let resp = db.suggest_next_step_auto().unwrap();

        assert!(resp.metrics.intents_count > 0);
        assert!(resp.metrics.total_us > 0);
    }

    #[test]
    fn test_suggest_fast_mode() {
        let db = test_db();
        let mut alloc = NodeIdAllocator::new();

        let goal_id = alloc.alloc(NodeKind::Goal);
        let mut goal = CognitiveNode::new(
            goal_id,
            "Quick task".to_string(),
            NodePayload::Goal(GoalPayload {
                description: "Quick task".to_string(),
                status: GoalStatus::Active,
                progress: 0.3,
                deadline: None,
                priority: Priority::High,
                parent_goal: None,
                completion_criteria: "Done".to_string(),
            }),
        );
        goal.attrs.urgency = 0.7;
        db.persist_cognitive_node(&goal).unwrap();
        db.persist_node_id_allocator(&alloc).unwrap();

        let resp = db.suggest_next_step_mode(ExecutionMode::Fast).unwrap();

        assert_eq!(resp.metrics.mode, ExecutionMode::Fast);
    }

    #[test]
    fn test_suggest_with_context() {
        let db = test_db();
        let mut alloc = NodeIdAllocator::new();

        let goal_id = alloc.alloc(NodeKind::Goal);
        let mut goal = CognitiveNode::new(
            goal_id,
            "Context test".to_string(),
            NodePayload::Goal(GoalPayload {
                description: "Context test".to_string(),
                status: GoalStatus::Active,
                progress: 0.2,
                deadline: None,
                priority: Priority::High,
                parent_goal: None,
                completion_criteria: "Done".to_string(),
            }),
        );
        goal.attrs.urgency = 0.7;
        db.persist_cognitive_node(&goal).unwrap();
        db.persist_node_id_allocator(&alloc).unwrap();

        let mut ctx = PolicyContext::default();
        ctx.current_hour = 14;
        ctx.cognitive_load = 0.5;

        let request = NextStepRequest {
            now: 1710000000.0,
            turn_text: Some("What should I work on?".to_string()),
            context: ctx,
            mode: ExecutionMode::Balanced,
            overrides: None,
        };

        let resp = db.suggest_next_step(&request).unwrap();
        assert!(resp.metrics.total_us > 0);
    }

    #[test]
    fn test_suggest_with_constraint() {
        let db = test_db();
        let mut alloc = NodeIdAllocator::new();

        let goal_id = alloc.alloc(NodeKind::Goal);
        let mut goal = CognitiveNode::new(
            goal_id,
            "Test goal".to_string(),
            NodePayload::Goal(GoalPayload {
                description: "Test goal".to_string(),
                status: GoalStatus::Active,
                progress: 0.3,
                deadline: None,
                priority: Priority::High,
                parent_goal: None,
                completion_criteria: "Done".to_string(),
            }),
        );
        goal.attrs.urgency = 0.7;
        db.persist_cognitive_node(&goal).unwrap();

        let constraint_id = alloc.alloc(NodeKind::Constraint);
        let constraint = CognitiveNode::new(
            constraint_id,
            "No execute".to_string(),
            NodePayload::Constraint(ConstraintPayload {
                description: "No execute during work".to_string(),
                constraint_type: ConstraintType::Hard,
                condition: "no execute actions".to_string(),
                imposed_by: "user".to_string(),
            }),
        );
        db.persist_cognitive_node(&constraint).unwrap();
        db.persist_node_id_allocator(&alloc).unwrap();

        let resp = db.suggest_next_step_auto().unwrap();
        assert!(resp.metrics.total_us > 0);
    }
}
