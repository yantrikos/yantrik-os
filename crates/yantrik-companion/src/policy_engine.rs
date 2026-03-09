//! Contextual Policy Engine — predicate-based permission decisions.
//!
//! Replaces the flat 4-level permission gate (Safe/Standard/Sensitive/Dangerous)
//! with a rich predicate evaluation system. Each action is evaluated against
//! multiple context predicates to yield a nuanced decision.
//!
//! **Predicates** (evaluated per action):
//! - Is it irreversible?
//! - Is it external-facing (sends email, posts message)?
//! - Is it a bulk operation?
//! - Does it touch financial/legal data?
//! - Is the recipient new (never contacted before)?
//! - Does it access secrets/credentials?
//! - Does it touch private memories?
//! - Does it run in background (no user watching)?
//! - Is confidence below threshold?
//! - Is the user focused/away?
//!
//! **Decisions**:
//! - AllowSilently: proceed with no notification
//! - AllowWithAudit: proceed but log for review
//! - AskInline: quick inline confirmation
//! - AskHighFriction: explicit confirmation dialog
//! - DeferUntilAvailable: queue until user is present
//! - Deny: block the action

use std::collections::HashSet;

use crate::trust_model::TrustState;

// ── Predicates ──────────────────────────────────────────────────────────────

/// Context predicates about an action.
#[derive(Debug, Clone, Default)]
pub struct ActionContext {
    /// The action/tool being evaluated.
    pub action_name: String,
    /// Is this action irreversible (delete, send, post)?
    pub irreversible: bool,
    /// Does this action affect external parties (email, message, post)?
    pub external_facing: bool,
    /// Is this a bulk operation (affects many items)?
    pub bulk_operation: bool,
    /// Does this touch financial or legal data?
    pub financial_legal: bool,
    /// Is the recipient new (never contacted before)?
    pub new_recipient: bool,
    /// Does this access secrets, credentials, or keys?
    pub touches_secrets: bool,
    /// Does this access private-flagged memories?
    pub touches_private_memories: bool,
    /// Is this running in the background (proactive, no user watching)?
    pub background_action: bool,
    /// Is the agent's confidence in this action low?
    pub low_confidence: bool,
    /// Is the user currently focused/deep-working?
    pub user_focused: bool,
    /// Is the user idle/away?
    pub user_away: bool,
    /// Custom risk flags (tool-specific).
    pub custom_flags: HashSet<String>,
}

impl ActionContext {
    pub fn new(action_name: &str) -> Self {
        Self {
            action_name: action_name.to_string(),
            ..Default::default()
        }
    }

    /// Auto-populate predicates based on known tool categories.
    pub fn from_tool(tool_name: &str) -> Self {
        let mut ctx = Self::new(tool_name);

        match tool_name {
            // Irreversible + external
            "send_email" | "email_send" | "send_message" | "whatsapp_send" => {
                ctx.irreversible = true;
                ctx.external_facing = true;
            }
            // File modification
            "write_file" | "manage_files" | "edit_file" => {
                ctx.irreversible = true;
            }
            // Command execution
            "run_command" => {
                ctx.irreversible = true;
            }
            // Memory/vault
            "remember" | "save_user_fact" => {}
            "vault_store" | "vault_read" => {
                ctx.touches_secrets = true;
            }
            // Read-only tools
            "recall" | "read_file" | "list_files" | "search_files"
            | "email_list" | "email_read" | "calendar_list"
            | "web_search" | "browse" | "get_weather"
            | "system_info" | "introspect" => {}
            // Browser actions
            "browser_click" | "browser_type" | "browser_click_element"
            | "browser_type_element" => {
                ctx.external_facing = true; // Could submit forms
            }
            _ => {}
        }

        ctx
    }

    /// Calculate a numeric risk score (0.0 = safe, 1.0 = very risky).
    pub fn risk_score(&self) -> f64 {
        let mut score: f64 = 0.0;

        if self.irreversible { score += 0.25; }
        if self.external_facing { score += 0.20; }
        if self.bulk_operation { score += 0.15; }
        if self.financial_legal { score += 0.25; }
        if self.new_recipient { score += 0.10; }
        if self.touches_secrets { score += 0.20; }
        if self.touches_private_memories { score += 0.10; }
        if self.background_action { score += 0.10; }
        if self.low_confidence { score += 0.15; }

        score.min(1.0)
    }
}

// ── Policy Decision ─────────────────────────────────────────────────────────

/// The policy engine's decision for an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Proceed silently — no notification needed.
    AllowSilently,
    /// Proceed but create an audit log entry.
    AllowWithAudit,
    /// Ask for quick inline confirmation (low friction).
    AskInline { reason: String },
    /// Ask for explicit confirmation (high friction dialog).
    AskHighFriction { reason: String },
    /// Defer until user is available (queue for later).
    DeferUntilAvailable { reason: String },
    /// Block the action entirely.
    Deny { reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::AllowSilently | Self::AllowWithAudit)
    }

    pub fn needs_confirmation(&self) -> bool {
        matches!(self, Self::AskInline { .. } | Self::AskHighFriction { .. })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AllowSilently => "allow_silent",
            Self::AllowWithAudit => "allow_audit",
            Self::AskInline { .. } => "ask_inline",
            Self::AskHighFriction { .. } => "ask_high_friction",
            Self::DeferUntilAvailable { .. } => "defer",
            Self::Deny { .. } => "deny",
        }
    }
}

// ── Policy Engine ───────────────────────────────────────────────────────────

/// The contextual policy engine.
pub struct PolicyEngine {
    /// Hard-deny list: these actions are never allowed autonomously.
    deny_list: HashSet<String>,
    /// Always-allow list: these actions skip evaluation.
    allow_list: HashSet<String>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        let mut deny_list = HashSet::new();
        // Actions that should never be autonomous
        deny_list.insert("vault_delete".into());
        deny_list.insert("delete_all_memories".into());

        let mut allow_list = HashSet::new();
        // Actions that are always safe
        allow_list.insert("recall".into());
        allow_list.insert("read_file".into());
        allow_list.insert("list_files".into());
        allow_list.insert("search_files".into());
        allow_list.insert("introspect".into());
        allow_list.insert("system_info".into());
        allow_list.insert("get_weather".into());

        Self { deny_list, allow_list }
    }

    /// Evaluate an action against policy predicates.
    pub fn evaluate(
        &self,
        action: &ActionContext,
        trust: &TrustState,
    ) -> PolicyDecision {
        // Hard deny list
        if self.deny_list.contains(&action.action_name) {
            return PolicyDecision::Deny {
                reason: format!("'{}' is on the deny list", action.action_name),
            };
        }

        // Always-allow list (for user-initiated actions with safe tools)
        if self.allow_list.contains(&action.action_name) && !action.background_action {
            return PolicyDecision::AllowSilently;
        }

        let risk = action.risk_score();

        // Secrets always require high-friction confirmation
        if action.touches_secrets {
            return PolicyDecision::AskHighFriction {
                reason: "This action accesses secrets or credentials".into(),
            };
        }

        // Financial/legal always requires high-friction confirmation
        if action.financial_legal {
            return PolicyDecision::AskHighFriction {
                reason: "This action involves financial or legal data".into(),
            };
        }

        // Background actions have a higher bar
        if action.background_action {
            if !trust.can_act_autonomously() {
                return PolicyDecision::DeferUntilAvailable {
                    reason: "Action trust too low for background execution".into(),
                };
            }
            if risk > 0.3 {
                return PolicyDecision::DeferUntilAvailable {
                    reason: format!("Background action risk ({:.0}%) exceeds threshold", risk * 100.0),
                };
            }
            return PolicyDecision::AllowWithAudit;
        }

        // User is away → defer risky actions
        if action.user_away && risk > 0.2 {
            return PolicyDecision::DeferUntilAvailable {
                reason: "User is away — deferring risky action".into(),
            };
        }

        // External-facing + new recipient → high friction
        if action.external_facing && action.new_recipient {
            return PolicyDecision::AskHighFriction {
                reason: "Sending to a new recipient — please confirm".into(),
            };
        }

        // External-facing + irreversible → inline confirmation unless high trust
        if action.external_facing && action.irreversible {
            if trust.action >= 0.8 {
                return PolicyDecision::AllowWithAudit;
            }
            return PolicyDecision::AskInline {
                reason: format!("Confirm: {} (external, irreversible)", action.action_name),
            };
        }

        // Irreversible with low confidence → inline confirmation
        if action.irreversible && action.low_confidence {
            return PolicyDecision::AskInline {
                reason: "Low confidence on irreversible action — please confirm".into(),
            };
        }

        // Bulk operations → inline confirmation
        if action.bulk_operation {
            return PolicyDecision::AskInline {
                reason: "Bulk operation — please confirm".into(),
            };
        }

        // Low risk + reasonable trust → allow
        if risk <= 0.2 {
            return PolicyDecision::AllowSilently;
        }

        // Medium risk → allow with audit if trust is decent
        if risk <= 0.4 && trust.action >= 0.5 {
            return PolicyDecision::AllowWithAudit;
        }

        // Default: inline confirmation for anything else
        PolicyDecision::AskInline {
            reason: format!("Action '{}' requires confirmation (risk: {:.0}%)", action.action_name, risk * 100.0),
        }
    }

    /// Add an action to the deny list.
    pub fn add_deny(&mut self, action: &str) {
        self.deny_list.insert(action.to_string());
    }

    /// Add an action to the allow list.
    pub fn add_allow(&mut self, action: &str) {
        self.allow_list.insert(action.to_string());
    }

    /// Remove from deny list.
    pub fn remove_deny(&mut self, action: &str) {
        self.deny_list.remove(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust_model::TrustState;

    fn default_trust() -> TrustState {
        TrustState::default()
    }

    fn high_trust() -> TrustState {
        TrustState {
            action: 0.9,
            personal: 0.8,
            taste: 0.7,
            updated_at: 0.0,
        }
    }

    #[test]
    fn safe_tools_always_allowed() {
        let engine = PolicyEngine::new();
        let ctx = ActionContext::from_tool("recall");
        let decision = engine.evaluate(&ctx, &default_trust());
        assert_eq!(decision, PolicyDecision::AllowSilently);
    }

    #[test]
    fn deny_list_blocks() {
        let engine = PolicyEngine::new();
        let ctx = ActionContext::from_tool("vault_delete");
        let decision = engine.evaluate(&ctx, &high_trust());
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn external_irreversible_needs_confirmation() {
        let engine = PolicyEngine::new();
        let ctx = ActionContext::from_tool("send_email");
        let decision = engine.evaluate(&ctx, &default_trust());
        assert!(decision.needs_confirmation(),
            "send_email should need confirmation, got {:?}", decision);
    }

    #[test]
    fn external_irreversible_allowed_with_high_trust() {
        let engine = PolicyEngine::new();
        let ctx = ActionContext::from_tool("send_email");
        let decision = engine.evaluate(&ctx, &high_trust());
        assert_eq!(decision, PolicyDecision::AllowWithAudit);
    }

    #[test]
    fn background_deferred_without_action_trust() {
        let engine = PolicyEngine::new();
        let mut ctx = ActionContext::from_tool("write_file");
        ctx.background_action = true;
        let decision = engine.evaluate(&ctx, &default_trust());
        assert!(matches!(decision, PolicyDecision::DeferUntilAvailable { .. }));
    }

    #[test]
    fn background_allowed_with_high_trust_low_risk() {
        let engine = PolicyEngine::new();
        let mut ctx = ActionContext::new("remember");
        ctx.background_action = true;
        let decision = engine.evaluate(&ctx, &high_trust());
        assert_eq!(decision, PolicyDecision::AllowWithAudit);
    }

    #[test]
    fn secrets_always_high_friction() {
        let engine = PolicyEngine::new();
        let ctx = ActionContext::from_tool("vault_read");
        let decision = engine.evaluate(&ctx, &high_trust());
        assert!(matches!(decision, PolicyDecision::AskHighFriction { .. }));
    }

    #[test]
    fn new_recipient_high_friction() {
        let engine = PolicyEngine::new();
        let mut ctx = ActionContext::from_tool("send_email");
        ctx.new_recipient = true;
        let decision = engine.evaluate(&ctx, &high_trust());
        assert!(matches!(decision, PolicyDecision::AskHighFriction { .. }));
    }

    #[test]
    fn user_away_defers_risky() {
        let engine = PolicyEngine::new();
        let mut ctx = ActionContext::from_tool("write_file");
        ctx.user_away = true;
        let decision = engine.evaluate(&ctx, &default_trust());
        assert!(matches!(decision, PolicyDecision::DeferUntilAvailable { .. }));
    }

    #[test]
    fn risk_score_calculation() {
        let low = ActionContext::from_tool("recall");
        assert!(low.risk_score() < 0.1);

        let high = ActionContext {
            action_name: "send_money".into(),
            irreversible: true,
            external_facing: true,
            financial_legal: true,
            new_recipient: true,
            ..Default::default()
        };
        assert!(high.risk_score() > 0.5);
    }

    #[test]
    fn bulk_operations_need_confirmation() {
        let engine = PolicyEngine::new();
        let mut ctx = ActionContext::new("batch_delete");
        ctx.bulk_operation = true;
        ctx.irreversible = true;
        let decision = engine.evaluate(&ctx, &high_trust());
        assert!(decision.needs_confirmation());
    }

    #[test]
    fn low_confidence_irreversible_needs_confirmation() {
        let engine = PolicyEngine::new();
        let mut ctx = ActionContext::from_tool("write_file");
        ctx.low_confidence = true;
        let decision = engine.evaluate(&ctx, &default_trust());
        assert!(decision.needs_confirmation());
    }

    #[test]
    fn custom_deny_allow_lists() {
        let mut engine = PolicyEngine::new();

        engine.add_deny("dangerous_tool");
        let ctx = ActionContext::new("dangerous_tool");
        assert!(matches!(engine.evaluate(&ctx, &high_trust()), PolicyDecision::Deny { .. }));

        engine.add_allow("my_safe_tool");
        let ctx = ActionContext::new("my_safe_tool");
        assert_eq!(engine.evaluate(&ctx, &default_trust()), PolicyDecision::AllowSilently);
    }
}
