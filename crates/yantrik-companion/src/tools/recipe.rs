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
                                WaitFor steps need: condition ({\"type\":\"Duration\",\"seconds\":N} or {\"type\":\"Time\",\"hour\":H,\"minute\":M} on this machine's own clock), timeout_secs (optional). \
                                Notify steps need: message (use {{var}} for variables). \
                                AskUser steps need: question, store_as, choices (optional list). \
                                Branch steps need: condition (a variable name: set and not empty, false or 0 takes then_steps), then_steps, else_steps (lists of steps). \
                                Agent steps hand a turn to a role from the agent catalog and keep its answer: role (researcher, planner, coder, reviewer, red-team, writer, chair, scribe), prompt, store_as, context (optional). \
                                Agent steps that do not read each other's answers work at the same time, and a step that reads one waits for it. \
                                An Agent step goes at the top of a recipe, not inside a Branch's arm. \
                                A recipe with Agent steps runs only when the person starts it: the Recipes screen, or the shell's run_recipe, which asks them. \
                                A JumpIf back to an earlier step is a loop; one that never waits is stopped after 100 steps.",
                            "items": { "type": "object" }
                        },
                        "trigger": {
                            "type": "object",
                            "description": "Optional trigger: what starts it on its own, with nobody at the desk — \
                                so an Agent step in it asks the person on a card before any role above safe. \
                                Types: 'Manual' (default), \
                                'Cron' (needs 'expression' like '0 9 * * *', read on this machine's own clock), \
                                'Event' (needs 'event_type', a type the desktop records like 'system/network' or its last part 'network'; optional 'filter' object whose keys must match), \
                                'RecipeComplete' (needs 'recipe_id': another recipe's id or name; the run gets that recipe's variables as {{after_<name>}}, and {{after_result}})."
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
        if crate::recipe::agent_in_arm(&steps) {
            return format!("Recipe not created: {}.", crate::recipe::IN_ARM_REFUSED);
        }
        if let Some((i, hour, minute)) = steps.iter().enumerate().find_map(|(i, s)| match s {
            RecipeStep::WaitFor { condition: WaitCondition::Time { hour, minute }, .. } if *hour > 23 || *minute > 59 => {
                Some((i, *hour, *minute))
            }
            _ => None,
        }) {
            return format!("Recipe not created: step {i} waits for {hour:02}:{minute:02}, which is no time of day.");
        }

        // Parse trigger
        let trigger = args.get("trigger").and_then(|v| {
            serde_json::from_value::<TriggerType>(v.clone()).ok()
        });

        let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");

        // Inject past failure learnings as warnings
        let learnings = RecipeStore::get_failure_learnings(&ctx.db.conn(), 5);
        if !learnings.is_empty() {
            tracing::debug!(
                count = learnings.len(),
                "Past recipe failure learnings available for context"
            );
        }

        let recipe_id = RecipeStore::create(
            &ctx.db.conn(),
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
        let recipes = RecipeStore::list(&ctx.db.conn(), status, 20);

        if recipes.is_empty() {
            return match status {
                Some(s) => format!("No {} recipes. Tip: use find_recipe to search built-in templates by intent.", s),
                None => "No recipes found. Tip: use find_recipe to search 50 built-in templates by intent, or create_recipe to build a custom one.".to_string(),
            };
        }

        let mut result = format!("Recipes ({}):\n\n", recipes.len());
        for r in &recipes {
            let icon = match r.status {
                crate::recipe::RecipeStatus::Running => "▶",
                crate::recipe::RecipeStatus::Waiting => "⏸",
                crate::recipe::RecipeStatus::Paused => "‖",
                crate::recipe::RecipeStatus::Done => "✓",
                crate::recipe::RecipeStatus::Failed => "✗",
                crate::recipe::RecipeStatus::Pending => "○",
            };
            let steps = RecipeStore::get_steps(&ctx.db.conn(), &r.id);
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
                            "description": "The recipe ID to run (e.g., 'rcp_7f3a9c01b2d4')"
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

        // By id, then by name; a template, or a recipe that has already run, starts as a new
        // recipe with its own record (`RecipeStore::start_run`). This reset the recipe in place —
        // steps to pending, pointer to 0 — so every run of a built-in wrote over the last one.
        // Marked running: the worker starts every running recipe after the turn that ran this
        // tool (`RecipeStore::get_resumable`).
        //
        // A formation — a recipe with Agent steps — is refused here: starting agents needs the
        // person's leave, which this tool, graded standard, does not ask for. The shell's own
        // `run_recipe` (sensitive) and the Recipes screen do (`recipe_templates::start`).
        let vars = args.get("variables").and_then(|v| v.as_object());
        let conn = ctx.db.conn();
        match crate::recipe_templates::start(&conn, id_or_name, vars, None) {
            Ok((recipe, run)) if run == recipe.id => format!(
                "Recipe '{}' [{}] queued for execution. It will start processing immediately.",
                recipe.name, run
            ),
            Ok((recipe, run)) => format!(
                "Recipe '{}' started as a new run [{}] (from [{}]). It will start processing immediately.",
                recipe.name, run, recipe.id
            ),
            Err(why) => why,
        }
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::RecipeStatus;
    use serde_json::json;
    use yantrikdb_core::YantrikDB;

    fn ctx(db: &YantrikDB) -> ToolContext<'_> {
        ToolContext {
            db,
            max_permission: PermissionLevel::Standard,
            registry_metadata: None,
            task_manager: None,
            incognito: false,
            agent_spawner: None,
        }
    }

    /// A run of a template is a new recipe, and the template stays one (#176). `run_recipe` reset
    /// the built-in in place — its steps to pending, its pointer to 0 — so each run wrote over the
    /// last one's record, and a built-in that had run once was no longer a template.
    #[test]
    fn running_a_template_starts_a_new_recipe() {
        let db = YantrikDB::new(":memory:", 384).expect("in-memory database");
        RecipeStore::ensure_tables(&db.conn());
        let steps = [
            RecipeStep::Tool { tool_name: "get_weather".into(), args: json!({"city": "{{city}}"}), store_as: "weather".into(), on_error: ErrorAction::Fail },
            RecipeStep::Notify { message: "{{weather}}".into() },
        ];
        RecipeStore::ensure_builtin(&db.conn(), "builtin_weather", "Weather", "", &steps);

        let said = RunRecipeTool.execute(&ctx(&db), &json!({"recipe_id": "builtin_weather", "variables": {"city": "Pune"}}));
        let running = RecipeStore::list(&db.conn(), Some("running"), 10);
        assert_eq!(running.len(), 1, "{said}");
        let run = running[0].clone();
        assert_ne!(run.id, "builtin_weather", "a run of a template is a recipe of its own: {said}");
        assert!(said.contains(&run.id), "the tool names the run it started: {said}");
        assert_eq!(run.name, "Weather");
        assert_eq!(RecipeStore::get_steps(&db.conn(), &run.id).len(), 2);
        assert_eq!(RecipeStore::get_vars(&db.conn(), &run.id).get("city"), Some(&json!("Pune")));
        assert_eq!(RecipeStore::get(&db.conn(), "builtin_weather").map(|r| r.status), Some(RecipeStatus::Pending), "the template is untouched");
        assert!(RecipeStore::get_vars(&db.conn(), "builtin_weather").is_empty(), "and holds no run's variables");

        // In flight, it is not started again over itself.
        let again = RunRecipeTool.execute(&ctx(&db), &json!({"recipe_id": run.id}));
        assert!(again.contains("already"), "{again}");
        assert_eq!(RecipeStore::list(&db.conn(), Some("running"), 10).len(), 1);

        // Finished, running it again is another new recipe, and the first keeps its record.
        RecipeStore::complete_step(&db.conn(), &run.id, 0, "Sunny");
        RecipeStore::complete_step(&db.conn(), &run.id, 1, "Sunny");
        RecipeStore::update_status(&db.conn(), &run.id, &RecipeStatus::Done, 2);
        let rerun = RunRecipeTool.execute(&ctx(&db), &json!({"recipe_id": run.id}));
        let second = RecipeStore::list(&db.conn(), Some("running"), 10);
        assert_eq!(second.len(), 1, "{rerun}");
        assert_ne!(second[0].id, run.id, "{rerun}");
        assert_eq!(RecipeStore::get(&db.conn(), &run.id).map(|r| r.status), Some(RecipeStatus::Done));
        assert_eq!(RecipeStore::get_steps(&db.conn(), &run.id)[0].result.as_deref(), Some("Sunny"), "the first run's record is kept");
        assert!(RecipeStore::get_steps(&db.conn(), &second[0].id).iter().all(|s| s.status == "pending"));
    }
}
