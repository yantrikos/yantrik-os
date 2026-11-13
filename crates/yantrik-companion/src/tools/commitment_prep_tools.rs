//! Forgetting a promise, which takes what was prepared from it along (#1184).
//!
//! There is deliberately no tool for the `commitment_prep` setting, not even one that can only
//! say no: a tool can be called by a model that has just read an email, or by a recipe step, and
//! the setting is the person's. Both its answers are person-only (`commitment_consent`, answered by
//! a click the shell wires). A test (`commitment_prep_consent.rs`) holds the registry to that.

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::commitment_store as store;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ForgetCommitmentTool));
}

pub struct ForgetCommitmentTool;

impl Tool for ForgetCommitmentTool {
    fn name(&self) -> &'static str { "forget_commitment" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "life_management" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "forget_commitment",
                "description": "Forget a promise the user made, and anything prepared for it",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "commitment_id": { "type": "integer", "description": "The id of the commitment" }
                    },
                    "required": ["commitment_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let Some(id) = args.get("commitment_id").and_then(|v| v.as_i64()) else {
            return "Error: commitment_id is required".to_string();
        };
        if store::forget(&ctx.db.conn(), id) {
            "Forgotten, with anything prepared for it.".to_string()
        } else {
            "I don't have that promise.".to_string()
        }
    }
}
