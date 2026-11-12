//! The person's say over `commitment_prep` (#1184): answering its question, switching it on or
//! off, and forgetting a promise, which takes what was prepared from it along.

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
                "description": "Record the user's answer about getting promised things ready (opening the files they named in Mind View and drafting the email, never sending). 'yes' answers 'Shall I get these ready for promises like this?'; 'no' or 'off' stops it; 'on' turns it back on.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "choice": { "type": "string", "description": "yes, no, on or off" }
                    },
                    "required": ["choice"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let conn = &ctx.db.conn();
        match args.get("choice").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase().as_str() {
            "yes" => {
                store::set_consent(conn, true);
                store::set_enabled(conn, true);
                "Got it. I'll get promised things ready, and never send anything.".to_string()
            }
            "no" | "off" => {
                store::set_consent(conn, false);
                store::set_enabled(conn, false);
                "Okay. I won't get promised things ready.".to_string()
            }
            "on" => {
                store::set_enabled(conn, true);
                "Getting promised things ready is on.".to_string()
            }
            _ => "Error: choice must be yes, no, on or off".to_string(),
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
