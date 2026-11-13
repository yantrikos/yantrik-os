//! The person's say over `commitment_prep` (#1184): refusing it, and forgetting a promise, which
//! takes what was prepared from it along. These tools can only say no: the yes, and turning it back
//! on, are person-only (`commitment_consent`, answered by a click the shell wires), because a tool
//! can be called by a model that has just read an email, or by a recipe step.

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::commitment_store as store;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(CommitmentPrepSettingTool));
    reg.register(Box::new(ForgetCommitmentTool));
}

pub struct CommitmentPrepSettingTool;

impl Tool for CommitmentPrepSettingTool {
    fn name(&self) -> &'static str { "commitment_prep_setting" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "life_management" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "commitment_prep_setting",
                "description": "Stop getting promised things ready (opening the files the user named in Mind View and drafting the email), when the user says no or asks for it off. It cannot turn it on: only the user can, with the switch in Settings \u{2192} Companion.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "choice": { "type": "string", "description": "no or off" }
                    },
                    "required": ["choice"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let conn = &ctx.db.conn();
        match args.get("choice").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase().as_str() {
            "no" | "off" => {
                store::refuse(conn);
                "Okay. I won't get promised things ready. Only you can turn it back on, in Settings \u{2192} Companion.".to_string()
            }
            "yes" | "on" => "Error: only the user can turn this on, in Settings \u{2192} Companion. I can only stop it.".to_string(),
            _ => "Error: choice must be no or off".to_string(),
        }
    }
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
