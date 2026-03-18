//! Query Planner — asks the LLM whether to answer directly or create a research recipe.
//!
//! For complex queries (trip planning, comparisons, multi-source research), the model
//! generates a recipe JSON with Tool steps + a final Think synthesis step. For simple
//! queries, it returns "direct" and the normal chat pipeline handles it.
//!
//! This prevents the 4B model from fabricating data by:
//! 1. Forcing multi-step research through deterministic Tool execution
//! 2. Isolating synthesis into a final Think step with only tool outputs as context
//! 3. The model never gets a chance to "freestyle" an answer for complex queries

use serde::{Deserialize, Serialize};

use crate::recipe::{RecipeStep, ErrorAction};

/// Result of the planner classification call.
#[derive(Debug, Clone)]
pub enum PlanDecision {
    /// Simple query — let the normal chat pipeline handle it.
    Direct,
    /// Complex query — execute this recipe, then synthesize.
    Recipe {
        goal: String,
        steps: Vec<RecipeStep>,
    },
}

/// Raw JSON output from the planner LLM call.
#[derive(Debug, Deserialize, Serialize)]
struct PlannerOutput {
    mode: String,
    #[serde(default)]
    steps: Vec<PlannerStep>,
    #[serde(default)]
    synthesize: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct PlannerStep {
    tool: String,
    #[serde(default)]
    args: serde_json::Value,
    #[serde(default)]
    save_as: String,
}

/// Build the planner system prompt with few-shot examples.
fn build_planner_prompt(available_tools: &[&str]) -> String {
    let tool_list = available_tools.join(", ");
    format!(
        r#"You are a task planner. Given a user request, decide:
1. If answerable with 0-2 tool calls → reply: {{"mode": "direct"}}
2. If it needs research across multiple sources → reply with a step plan

Available tools: {tool_list}

If plan needed, reply:
{{"mode": "plan", "steps": [{{"tool": "tool_name", "args": {{}}, "save_as": "var_name"}}, ...], "synthesize": "prompt using {{{{variables}}}}"}}

Rules:
- Use Tool steps for ALL data gathering (weather, searches, lookups)
- The synthesize prompt runs AFTER all tools complete — it gets {{{{var_name}}}} for each step result
- synthesize must say "Use ONLY data from" the variables
- Maximum 8 tool steps
- For comparisons, search EACH option separately

Examples:

User: "What's the weather?"
{{"mode": "direct"}}

User: "Tell me a joke"
{{"mode": "direct"}}

User: "Check my email"
{{"mode": "direct"}}

User: "Compare restaurants near me for date night"
{{"mode": "plan", "steps": [{{"tool": "web_search", "args": {{"query": "best date night restaurants near me reviews prices"}}, "save_as": "restaurants"}}, {{"tool": "web_search", "args": {{"query": "romantic restaurant atmosphere ratings near me"}}, "save_as": "reviews"}}], "synthesize": "Compare top 3 restaurants from {{{{restaurants}}}} and {{{{reviews}}}}. Include prices, ratings, and atmosphere. Use ONLY data from search results."}}

User: "Plan a day trip with hiking and food"
{{"mode": "plan", "steps": [{{"tool": "get_weather", "args": {{}}, "save_as": "weather"}}, {{"tool": "web_search", "args": {{"query": "best hiking trails near me"}}, "save_as": "trails"}}, {{"tool": "web_search", "args": {{"query": "restaurants near hiking trails food"}}, "save_as": "food"}}], "synthesize": "Plan a day trip using {{{{weather}}}}, {{{{trails}}}}, and {{{{food}}}}. Include trail details, restaurant suggestions, and weather considerations. Use ONLY data from search results."}}"#
    )
}

/// Ask the LLM whether this query should be handled directly or via a recipe plan.
///
/// Returns `PlanDecision::Direct` for simple queries, or `PlanDecision::Recipe` for
/// complex ones that need multi-step research.
pub fn plan_or_direct(
    llm: &dyn yantrik_ml::LLMBackend,
    user_text: &str,
    available_tools: &[&str],
) -> PlanDecision {
    use yantrik_ml::{ChatMessage, GenerationConfig};

    // Quick bypass for very short queries (greetings, single words)
    let word_count = user_text.split_whitespace().count();
    if word_count <= 3 {
        tracing::info!(words = word_count, "QueryPlanner: short query → direct");
        return PlanDecision::Direct;
    }

    let system_prompt = build_planner_prompt(available_tools);
    let messages = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(user_text),
    ];

    let gen_config = GenerationConfig {
        max_tokens: 512,
        temperature: 0.3, // Low temperature for structured output
        ..Default::default()
    };

    let response = match llm.chat(&messages, &gen_config, None) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("QueryPlanner LLM call failed: {e} → falling back to direct");
            return PlanDecision::Direct;
        }
    };

    // Parse the JSON response
    let text = response.text.trim();
    let parsed: PlannerOutput = match serde_json::from_str(text) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                raw = text,
                err = %e,
                "QueryPlanner: invalid JSON → falling back to direct"
            );
            return PlanDecision::Direct;
        }
    };

    if parsed.mode == "direct" || parsed.steps.is_empty() {
        tracing::info!("QueryPlanner: model chose direct mode");
        return PlanDecision::Direct;
    }

    // Convert planner steps to RecipeSteps
    let mut recipe_steps: Vec<RecipeStep> = Vec::new();

    // Add a Notify step at the beginning for UX
    let step_count = parsed.steps.len();
    let tool_names: Vec<&str> = parsed.steps.iter().map(|s| s.tool.as_str()).collect();
    let progress_msg = format!(
        "I'll research this thoroughly — running {} searches to gather real data.",
        step_count
    );
    recipe_steps.push(RecipeStep::Notify {
        message: progress_msg,
    });

    // Convert each planner step to a RecipeStep::Tool
    for step in &parsed.steps {
        let save_as = if step.save_as.is_empty() {
            step.tool.clone()
        } else {
            step.save_as.clone()
        };

        recipe_steps.push(RecipeStep::Tool {
            tool_name: step.tool.clone(),
            args: step.args.clone(),
            store_as: save_as,
            on_error: ErrorAction::Skip, // Skip failed steps, note in synthesis
        });
    }

    // Add the synthesis Think step at the end
    let synthesize_prompt = if parsed.synthesize.is_empty() {
        let var_refs: Vec<String> = parsed.steps.iter().map(|s| {
            let var = if s.save_as.is_empty() { &s.tool } else { &s.save_as };
            format!("{{{{{}}}}}", var)
        }).collect();
        format!(
            "Synthesize a comprehensive answer using the collected data: {}. \
             Use ONLY data from the search results. If any data is missing, say so.",
            var_refs.join(", ")
        )
    } else {
        parsed.synthesize.clone()
    };

    recipe_steps.push(RecipeStep::Think {
        prompt: synthesize_prompt,
        store_as: "final_answer".to_string(),
    });

    let goal = format!("Research plan for: {}", &user_text[..user_text.len().min(100)]);

    tracing::info!(
        steps = recipe_steps.len(),
        tools = ?tool_names,
        "QueryPlanner: recipe mode — {} tool steps + synthesis",
        step_count
    );

    PlanDecision::Recipe {
        goal,
        steps: recipe_steps,
    }
}

/// Check if a query planner result has a recipe that's worth executing.
/// Returns the recipe ID if created, or None if direct mode.
pub fn maybe_create_recipe(
    conn: &rusqlite::Connection,
    decision: &PlanDecision,
) -> Option<String> {
    match decision {
        PlanDecision::Direct => None,
        PlanDecision::Recipe { goal, steps } => {
            let recipe_id = crate::recipe::RecipeStore::create(
                conn, goal, goal, steps, None,
            );
            tracing::info!(recipe_id = %recipe_id, "QueryPlanner: recipe created for execution");
            Some(recipe_id)
        }
    }
}
