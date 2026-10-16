//! Router report tool — reads the cognitive router's own routing log.

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(RouterReportTool));
}

pub struct RouterReportTool;

impl Tool for RouterReportTool {
    fn name(&self) -> &'static str { "router_report" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "system" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "router_report",
                "description": "Report how the cognitive router handled recent queries: offline vs LLM share, per-intent counts, average duration, failures",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "hours": {
                            "type": "number",
                            "description": "How far back to look, in hours (default 24)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let hours = args
            .get("hours")
            .and_then(|v| v.as_f64())
            .filter(|h| h.is_finite() && *h > 0.0)
            .unwrap_or(24.0);

        let conn = &ctx.db.conn();
        crate::cognitive_router::routing_report_text(conn, hours)
    }
}
