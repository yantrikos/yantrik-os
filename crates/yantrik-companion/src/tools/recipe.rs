//! Recipe tools — LLM can create, list, and run structured automation recipes.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::recipe::{RecipeStep, RecipeStore, TriggerType, ErrorAction, Condition, WaitCondition};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(CreateRecipeTool));
    reg.register(Box::new(ListRecipesTool));
    reg.register(Box::new(RunRecipeTool));
    reg.register(Box::new(FindRecipeTool));
}

// ── create_recipe ──

struct CreateRecipeTool;

impl Tool for CreateRecipeTool {
    fn name(&self) -> &'static str { "create_recipe" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_recipe",
                "description": "Create a structured automation recipe. Recipes are ordered lists of steps \
                    that execute automatically. Tool steps run directly without LLM. Think steps call the LLM. \
                    Use for repeatable multi-step automations like 'check email and summarize' or 'backup files daily'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Short recipe name (e.g., 'Morning email digest')"
                        },
                        "description": {
                            "type": "string",
                            "description": "What this recipe does"
                        },
                        "steps": {
                            "type": "array",
                            "description": "Ordered list of steps. Each step is an object with 'type' field. \
                                Types: 'Tool' (direct tool call, no LLM), 'Think' (LLM reasoning), \
                                'JumpIf' (conditional jump), 'WaitFor' (pause until condition), 'Notify' (send message to user). \
                                Tool steps need: tool_name, args (object), store_as (variable name), \
                                on_error (optional: {\"action\":\"Fail\"}, {\"action\":\"Skip\"}, {\"action\":\"Retry\",\"max\":3}, \
                                {\"action\":\"JumpTo\",\"step\":N}, or {\"action\":\"Replan\"} for auto-healing). \
                                PREFER on_error={\"action\":\"Replan\"} for critical steps — it auto-diagnoses failures and generates new steps. \
                                Think steps need: prompt (use {{var}} for variable references), store_as. \
                                JumpIf steps need: condition (object with 'op' field), target_step (index). \
                                WaitFor steps need: condition ({\"type\":\"Duration\",\"seconds\":N} or {\"type\":\"Time\",\"hour\":H,\"minute\":M}), timeout_secs (optional). \
                                Notify steps need: message (use {{var}} for variables).",
                            "items": { "type": "object" }
                        },
                        "trigger": {
                            "type": "object",
                            "description": "Optional trigger. Types: 'Manual' (default), \
                                'Cron' (needs 'expression' like '0 9 * * *'), \
                                'Event' (needs 'event_type' like 'email:new')."
                        }
                    },
                    "required": ["name", "steps"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: name".to_string(),
        };

        let steps_val = match args.get("steps").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return "Missing required parameter: steps (must be an array)".to_string(),
        };

        // Parse steps
        let mut steps = Vec::new();
        for (i, step_val) in steps_val.iter().enumerate() {
            match serde_json::from_value::<RecipeStep>(step_val.clone()) {
                Ok(step) => steps.push(step),
                Err(e) => return format!("Failed to parse step {}: {}. Step JSON: {}", i, e, step_val),
            }
        }

        if steps.is_empty() {
            return "Recipe must have at least one step".to_string();
        }

        // Parse trigger
        let trigger = args.get("trigger").and_then(|v| {
            serde_json::from_value::<TriggerType>(v.clone()).ok()
        });

        let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");

        // Inject past failure learnings as warnings
        let learnings = RecipeStore::get_failure_learnings(ctx.db.conn(), 5);
        if !learnings.is_empty() {
            tracing::debug!(
                count = learnings.len(),
                "Past recipe failure learnings available for context"
            );
        }

        let recipe_id = RecipeStore::create(
            ctx.db.conn(),
            name,
            description,
            &steps,
            trigger.as_ref(),
        );

        let trigger_desc = match &trigger {
            Some(TriggerType::Cron { expression }) => format!(" Trigger: cron '{}'.", expression),
            Some(TriggerType::Event { event_type, .. }) => format!(" Trigger: on '{}'.", event_type),
            Some(TriggerType::RecipeComplete { recipe_id }) => format!(" Trigger: after recipe {}.", recipe_id),
            _ => String::new(),
        };

        format!(
            "Recipe created: [{}] '{}' with {} steps.{} Use run_recipe to execute it manually.",
            recipe_id, name, steps.len(), trigger_desc
        )
    }
}

// ── list_recipes ──

struct ListRecipesTool;

impl Tool for ListRecipesTool {
    fn name(&self) -> &'static str { "list_recipes" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_recipes",
                "description": "List automation recipes",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "description": "Filter by status: 'pending', 'running', 'waiting', 'done', 'failed'. Omit for all."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let status = args.get("status").and_then(|v| v.as_str());
        let recipes = RecipeStore::list(ctx.db.conn(), status, 20);

        if recipes.is_empty() {
            return match status {
                Some(s) => format!("No {} recipes.", s),
                None => "No recipes found.".to_string(),
            };
        }

        let mut result = format!("Recipes ({}):\n\n", recipes.len());
        for r in &recipes {
            let icon = match r.status {
                crate::recipe::RecipeStatus::Running => "▶",
                crate::recipe::RecipeStatus::Waiting => "⏸",
                crate::recipe::RecipeStatus::Done => "✓",
                crate::recipe::RecipeStatus::Failed => "✗",
                crate::recipe::RecipeStatus::Pending => "○",
            };
            let steps = RecipeStore::get_steps(ctx.db.conn(), &r.id);
            let step_info = format!("{}/{} steps", r.current_step, steps.len());
            result.push_str(&format!("{} [{}] {} — {} ({})\n", icon, r.id, r.name, r.status.as_str(), step_info));
            if let Some(err) = &r.error_message {
                result.push_str(&format!("  Error: {}\n", err));
            }
        }
        result
    }
}

// ── run_recipe ──

struct RunRecipeTool;

impl Tool for RunRecipeTool {
    fn name(&self) -> &'static str { "run_recipe" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_recipe",
                "description": "Manually start or restart a recipe. The recipe will execute in the background, \
                    step by step. Tool steps run instantly; Think steps use the LLM.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "recipe_id": {
                            "type": "string",
                            "description": "The recipe ID to run (e.g., 'rcp_abc12345')"
                        },
                        "variables": {
                            "type": "object",
                            "description": "Optional initial variables for the recipe (key-value pairs)"
                        }
                    },
                    "required": ["recipe_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let id_or_name = match args.get("recipe_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: recipe_id".to_string(),
        };

        // Try by ID first, then by name (case-insensitive)
        let recipe = RecipeStore::get(ctx.db.conn(), id_or_name)
            .or_else(|| RecipeStore::find_by_name(ctx.db.conn(), id_or_name));

        let recipe = match recipe {
            Some(r) => r,
            None => return format!("Recipe not found: {}", id_or_name),
        };

        let recipe_id = &recipe.id;

        // Set initial variables if provided
        if let Some(vars) = args.get("variables").and_then(|v| v.as_object()) {
            for (key, value) in vars {
                RecipeStore::set_var(ctx.db.conn(), recipe_id, key, value);
            }
        }

        // Reset to step 0 and mark as pending (bridge will pick it up via ProcessRecipeStep)
        RecipeStore::update_status(ctx.db.conn(), recipe_id, &crate::recipe::RecipeStatus::Pending, 0);

        // Reset all steps to pending
        ctx.db.conn().execute(
            "UPDATE recipe_steps SET status = 'pending', result = NULL WHERE recipe_id = ?1",
            rusqlite::params![recipe_id],
        )
        .ok();

        format!(
            "Recipe '{}' [{}] queued for execution. It will start processing immediately.",
            recipe.name, recipe_id
        )
    }
}

// ── find_recipe ──

struct FindRecipeTool;

impl Tool for FindRecipeTool {
    fn name(&self) -> &'static str { "find_recipe" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "find_recipe",
                "description": "Find a recipe template matching a user's intent. \
                    Use this when the user asks for a multi-step task that might match \
                    a built-in recipe (e.g., 'check my emails', 'morning briefing', \
                    'research topic X', 'system health check'). Returns matching recipes \
                    with their required variables.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The user's request to match against recipe templates"
                        }
                    },
                    "required": ["query"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: query".to_string(),
        };

        let matches = crate::recipe_templates::match_intent(query, 5);

        if matches.is_empty() {
            return format!(
                "No recipe templates match '{}'. You can create a custom recipe with create_recipe.",
                query
            );
        }

        let mut result = format!("Matching recipes for '{}':\n\n", query);
        for (id, name, score) in &matches {
            // Get template details for required vars
            if let Some(template) = crate::recipe_templates::get_template(id) {
                result.push_str(&format!(
                    "- {} [{}] (score: {:.1})\n  {}\n",
                    name, id, score, template.description
                ));
                if !template.required_vars.is_empty() {
                    result.push_str("  Required variables:\n");
                    for (var_name, var_desc) in template.required_vars {
                        result.push_str(&format!("    - {}: {}\n", var_name, var_desc));
                    }
                }
                result.push('\n');
            }
        }
        result.push_str("Use run_recipe with the recipe ID and any required variables to execute.");
        result
    }
}