//! spawn_agents — parallel sub-agent execution tool.
//!
//! The LLM calls this when it identifies multiple independent tasks
//! that can be executed in parallel for faster results.

use serde_json::Value;

use super::{Tool, ToolContext, PermissionLevel};

struct SpawnAgentsTool;

impl Tool for SpawnAgentsTool {
    fn name(&self) -> &'static str {
        "spawn_agents"
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Standard
    }

    fn category(&self) -> &'static str {
        "delegation"
    }

    fn definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "spawn_agents",
                "description": "Run 2-5 independent tasks in parallel using sub-agents",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "tasks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "minItems": 2,
                            "maxItems": 5,
                            "description": "List of independent task descriptions (2-5 items). Each task will be executed by a separate sub-agent with its own tools."
                        }
                    },
                    "required": ["tasks"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &Value) -> String {
        let tasks: Vec<String> = match args.get("tasks").and_then(|v| v.as_array()) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            None => return "Error: 'tasks' must be an array of strings".to_string(),
        };

        if tasks.len() < 2 {
            return "Error: need at least 2 tasks for parallel execution. For a single task, just do it directly.".to_string();
        }
        if tasks.len() > 5 {
            return "Error: maximum 5 parallel tasks allowed to avoid resource exhaustion.".to_string();
        }

        let spawner = match &ctx.agent_spawner {
            Some(any) => match any.downcast_ref::<super::AgentSpawnerContext>() {
                Some(s) => s,
                None => return "Error: agent spawning not available in this context.".to_string(),
            },
            None => return "Error: agent spawning not available in this context.".to_string(),
        };

        let config = crate::sub_agent::SubAgentConfig {
            max_steps: spawner.max_steps,
            max_tokens: spawner.max_tokens,
            temperature: spawner.temperature,
            user_name: spawner.user_name.clone(),
            db_path: spawner.db_path.clone(),
            embedding_dim: spawner.embedding_dim,
            companion_config: spawner.config.clone(),
        };

        tracing::info!(
            count = tasks.len(),
            tasks = ?tasks,
            "Spawning parallel sub-agents"
        );

        let results = crate::sub_agent::spawn_parallel_agents(
            tasks,
            spawner.llm.clone(),
            config,
        );

        // Format results
        let mut output = String::from("## Parallel Agent Results\n\n");
        for result in &results {
            let status = if result.success { "SUCCESS" } else { "FAILED" };
            output.push_str(&format!(
                "### Agent {} — {} ({}ms)\n**Task:** {}\n{}\n",
                result.agent_id,
                status,
                result.elapsed_ms,
                result.task,
                result.response,
            ));
            if !result.tool_calls_made.is_empty() {
                output.push_str(&format!(
                    "**Tools used:** {}\n",
                    result.tool_calls_made.join(", ")
                ));
            }
            output.push('\n');
        }

        let success_count = results.iter().filter(|r| r.success).count();
        output.push_str(&format!(
            "---\n{}/{} tasks completed successfully.",
            success_count,
            results.len()
        ));

        output
    }
}

pub fn register(registry: &mut super::ToolRegistry) {
    registry.register(Box::new(SpawnAgentsTool));
}
