//! Recipe Executor — processes pending/running recipes step by step.
//!
//! Called from the background cognition loop. Each tick processes one step
//! of each active recipe. Tool steps bypass the LLM; Think steps use it.
//! Recipes persist state in SQLite so they survive restarts.

use crate::companion::CompanionService;
use crate::recipe::{
    AggregateOp, ErrorAction, FilterOp, RecipeStatus, RecipeStep, RecipeStore, StepResult,
    WaitCondition, resolve_vars, resolve_vars_in_json,
};
use yantrik_ml::{ChatMessage, GenerationConfig};
use std::collections::HashMap;

/// Maximum steps to execute per recipe per tick (prevents runaway recipes).
const MAX_STEPS_PER_TICK: usize = 10;

/// Maximum result size stored per step (bytes).
const MAX_RESULT_SIZE: usize = 4000;

/// Process all pending and running recipes. Call this from the background loop.
///
/// Returns the number of steps executed across all recipes.
pub fn tick(service: &mut CompanionService) -> usize {
    // 1. Resume waiting recipes whose conditions are met
    let expired = RecipeStore::get_expired_waiting(service.db.conn());
    for recipe_id in &expired {
        let step = RecipeStore::get(service.db.conn(), recipe_id)
            .map(|r| r.current_step)
            .unwrap_or(0);
        RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Running, step);
        tracing::info!(recipe_id = recipe_id.as_str(), "Recipe resumed from waiting");
    }

    // 2. Get all resumable recipes
    let resumable = RecipeStore::get_resumable(service.db.conn());
    if resumable.is_empty() {
        return 0;
    }

    let mut total_steps = 0;

    for recipe_id in resumable {
        let steps_run = execute_recipe_steps(service, &recipe_id);
        total_steps += steps_run;
    }

    if total_steps > 0 {
        tracing::info!(total_steps, "Recipe executor tick complete");
    }

    total_steps
}

/// Execute steps of a single recipe until it blocks, completes, or hits the per-tick limit.
fn execute_recipe_steps(service: &mut CompanionService, recipe_id: &str) -> usize {
    // Load recipe state upfront (immutable borrows of service.db end here)
    let (recipe, stored_steps, initial_vars) = {
        let conn = service.db.conn();
        let recipe = match RecipeStore::get(conn, recipe_id) {
            Some(r) => r,
            None => return 0,
        };
        RecipeStore::update_status(conn, recipe_id, &RecipeStatus::Running, recipe.current_step);
        let steps = RecipeStore::get_steps(conn, recipe_id);
        if steps.is_empty() {
            RecipeStore::update_status(conn, recipe_id, &RecipeStatus::Done, 0);
            return 0;
        }
        let vars = RecipeStore::get_vars(conn, recipe_id);
        (recipe, steps, vars)
    };

    let mut vars = initial_vars;
    let mut current_step = recipe.current_step;
    let mut steps_executed = 0;
    let total_steps = stored_steps.len();

    while current_step < total_steps && steps_executed < MAX_STEPS_PER_TICK {
        // Clone step data to avoid holding borrow on stored_steps during execution
        let (status, step, on_error) = match stored_steps.get(current_step) {
            Some(s) => {
                let err_action = match &s.step {
                    RecipeStep::Tool { on_error, .. } => on_error.clone(),
                    _ => ErrorAction::Fail,
                };
                (s.status.clone(), s.step.clone(), err_action)
            }
            None => break,
        };

        // Skip already-completed steps
        if status == "done" || status == "skipped" {
            current_step += 1;
            continue;
        }

        let result = execute_step(service, recipe_id, &step, &vars);
        steps_executed += 1;

        match result {
            StepResult::Continue => {
                RecipeStore::complete_step(service.db.conn(), recipe_id, current_step, "ok");
                current_step += 1;
                RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Running, current_step);
            }
            StepResult::JumpTo(target) => {
                RecipeStore::complete_step(service.db.conn(), recipe_id, current_step, &format!("jump:{}", target));
                current_step = target;
                RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Running, current_step);
            }
            StepResult::Waiting => {
                RecipeStore::complete_step(service.db.conn(), recipe_id, current_step, "waiting");
                current_step += 1;
                RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Waiting, current_step);
                break;
            }
            StepResult::Done => {
                RecipeStore::complete_step(service.db.conn(), recipe_id, current_step, "done");
                current_step = total_steps;
            }
            StepResult::Notify(msg) => {
                RecipeStore::complete_step(service.db.conn(), recipe_id, current_step, &truncate(&msg));
                deliver_notification(service, recipe_id, &msg);
                current_step += 1;
                RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Running, current_step);
            }
            StepResult::Failed(err) => {
                match handle_error(service, recipe_id, current_step, &err, &on_error, &vars) {
                    ErrorResolution::Continue => {
                        RecipeStore::skip_step(service.db.conn(), recipe_id, current_step);
                        current_step += 1;
                        RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Running, current_step);
                    }
                    ErrorResolution::JumpTo(target) => {
                        RecipeStore::fail_step(service.db.conn(), recipe_id, current_step, &err);
                        current_step = target;
                        RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Running, current_step);
                    }
                    ErrorResolution::Retry => {
                        RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Running, current_step);
                        break;
                    }
                    ErrorResolution::Abort => {
                        RecipeStore::fail_step(service.db.conn(), recipe_id, current_step, &err);
                        RecipeStore::set_error(service.db.conn(), recipe_id, &err);
                        tracing::warn!(recipe_id, step = current_step, error = %err, "Recipe failed");
                        break;
                    }
                }
            }
        }

        // Reload vars after each step (step may have added new ones)
        vars = RecipeStore::get_vars(service.db.conn(), recipe_id);
    }

    // Check if recipe completed
    if current_step >= total_steps {
        RecipeStore::update_status(service.db.conn(), recipe_id, &RecipeStatus::Done, current_step);
        tracing::info!(recipe_id, steps = total_steps, "Recipe completed");
    }

    steps_executed
}

/// Execute a single recipe step. Returns the step result.
fn execute_step(
    service: &mut CompanionService,
    recipe_id: &str,
    step: &RecipeStep,
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {
    match step {
        RecipeStep::Tool {
            tool_name,
            args,
            store_as,
            ..
        } => {
            let resolved_args = resolve_vars_in_json(args, vars);

            tracing::debug!(
                recipe_id,
                tool = tool_name.as_str(),
                "Recipe: executing tool"
            );

            let result = service.execute_tool_direct(tool_name, &resolved_args);
            let truncated = truncate(&result);

            // Store result as variable
            let val = serde_json::Value::String(truncated.clone());
            RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);

            StepResult::Continue
        }

        RecipeStep::Think { prompt, store_as, fallback_template } => {
            let resolved_prompt = resolve_vars(prompt, vars);

            let messages = vec![
                ChatMessage::system(&format!(
                    "You are {}, a personal AI companion. Answer based ONLY on the \
                     provided data. Never invent prices, ratings, or availability. \
                     If data is missing, say so. Be concise.",
                    service.config.personality.name
                )),
                ChatMessage::user(&resolved_prompt),
            ];

            let gen_config = GenerationConfig {
                max_tokens: service.config.llm.max_tokens,
                temperature: service.config.llm.temperature,
                ..Default::default()
            };

            match service.llm.chat(&messages, &gen_config, None) {
                Ok(r) => {
                    let text = strip_think_tags(&r.text);
                    let truncated = truncate(&text);
                    let val = serde_json::Value::String(truncated);
                    RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
                    StepResult::Continue
                }
                Err(e) => {
                    // Use fallback template if available when LLM is unavailable
                    if let Some(tmpl) = fallback_template {
                        let resolved = resolve_vars(tmpl, vars);
                        let val = serde_json::Value::String(truncate(&resolved));
                        RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
                        tracing::info!(recipe_id, "Think step used fallback template (LLM unavailable)");
                        StepResult::Continue
                    } else {
                        StepResult::Failed(format!("LLM error: {}", e))
                    }
                }
            }
        }

        RecipeStep::JumpIf {
            condition,
            target_step,
        } => {
            if condition.evaluate(vars) {
                StepResult::JumpTo(*target_step)
            } else {
                StepResult::Continue
            }
        }

        RecipeStep::WaitFor {
            condition,
            timeout_secs: _,
        } => {
            // Check if condition is already met
            let met = match condition {
                WaitCondition::Duration { seconds } => *seconds == 0,
                WaitCondition::Time { hour, minute } => {
                    let now = chrono_now();
                    now.0 > *hour || (now.0 == *hour && now.1 >= *minute)
                }
            };

            if met {
                StepResult::Continue
            } else {
                StepResult::Waiting
            }
        }

        RecipeStep::Notify { message } => {
            let resolved = resolve_vars(message, vars);
            StepResult::Notify(resolved)
        }

        RecipeStep::AskUser {
            question,
            store_as,
            choices: _,
        } => {
            let resolved_question = resolve_vars(question, vars);

            // Check if the user already provided an answer (via recipe vars)
            if let Some(existing) = vars.get(store_as) {
                if !existing.is_null()
                    && existing.as_str().map_or(true, |s| !s.is_empty())
                {
                    // Already answered — continue
                    return StepResult::Continue;
                }
            }

            // Deliver the question as a notification and wait
            deliver_notification(service, "", &resolved_question);
            StepResult::Waiting
        }

        RecipeStep::ThinkCited {
            prompt,
            store_as,
            source_vars,
        } => execute_think_cited(service, recipe_id, prompt, store_as, source_vars, vars),

        RecipeStep::Validate {
            input_var,
            store_as,
        } => execute_validate(service, recipe_id, input_var, store_as, vars),

        RecipeStep::Render {
            input_var,
            store_as,
            format,
        } => execute_render(service, recipe_id, input_var, store_as, format, vars),

        RecipeStep::Format {
            input_vars: _,
            template,
            store_as,
        } => {
            let resolved = resolve_vars(template, vars);
            let val = serde_json::Value::String(truncate(&resolved));
            RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
            StepResult::Continue
        }

        RecipeStep::Filter {
            input_var,
            field,
            op,
            value,
            store_as,
        } => execute_filter(service, recipe_id, input_var, field, op, value, store_as, vars),

        RecipeStep::Sort {
            input_var,
            by_field,
            descending,
            store_as,
        } => execute_sort(service, recipe_id, input_var, by_field, *descending, store_as, vars),

        RecipeStep::Aggregate {
            input_var,
            op,
            field,
            store_as,
        } => execute_aggregate(service, recipe_id, input_var, op, field.as_deref(), store_as, vars),

        RecipeStep::Extract {
            input_var,
            pattern,
            store_as,
        } => execute_extract(service, recipe_id, input_var, pattern, store_as, vars),

        RecipeStep::Branch {
            condition,
            then_steps,
            else_steps,
        } => execute_branch(service, recipe_id, condition, then_steps, else_steps, vars),
    }
}

// ── ThinkCited: LLM synthesis with per-claim citations ──

fn execute_think_cited(
    service: &mut CompanionService,
    recipe_id: &str,
    prompt: &str,
    store_as: &str,
    source_vars: &[String],
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {
    use crate::recipe::{CitedClaim, CitedOutput, EvidenceStatus};

    let resolved_prompt = resolve_vars(prompt, vars);

    // Build source context: list each source variable and its content
    let mut source_context = String::new();
    for (i, src_name) in source_vars.iter().enumerate() {
        let content = vars
            .get(src_name)
            .and_then(|v| v.as_str())
            .unwrap_or("(no data)");
        source_context.push_str(&format!(
            "\n[SOURCE:{}] (from step '{}'): {}\n",
            i + 1,
            src_name,
            content
        ));
    }

    let citation_instruction = format!(
        "You have the following sources:\n{}\n\n\
         {}\n\n\
         IMPORTANT: Output JSON with this exact structure:\n\
         {{\n\
           \"title\": \"<section title>\",\n\
           \"claims\": [\n\
             {{\"text\": \"<claim>\", \"sources\": [\"<source_var_name>\", ...]}},\n\
             ...\n\
           ]\n\
         }}\n\
         Each claim MUST reference which source(s) support it by variable name.\n\
         If a fact has no source, do NOT include it.\n\
         Output ONLY the JSON, no other text.",
        source_context, resolved_prompt
    );

    let messages = vec![
        ChatMessage::system(&format!(
            "You are {}. You produce citation-backed analysis. \
             Every claim must reference its source. Never invent facts.",
            service.config.personality.name
        )),
        ChatMessage::user(&citation_instruction),
    ];

    let gen_config = GenerationConfig {
        max_tokens: service.config.llm.max_tokens,
        temperature: 0.2, // Low temp for structured output
        ..Default::default()
    };

    match service.llm.chat(&messages, &gen_config, None) {
        Ok(r) => {
            let text = strip_think_tags(&r.text);
            let json_text = extract_json_object(&text);

            // Try to parse as CitedOutput
            let cited_output = match serde_json::from_str::<CitedOutput>(&json_text) {
                Ok(mut output) => {
                    // Compute confidence for each claim based on source count
                    for claim in &mut output.claims {
                        claim.confidence = match claim.sources.len() {
                            0 => "uncited".to_string(),
                            1 => "low".to_string(),
                            2 => "medium".to_string(),
                            _ => "high".to_string(),
                        };
                    }
                    // Compute overall evidence status
                    output.evidence_status = compute_evidence_status(&output.claims, source_vars);
                    output
                }
                Err(_) => {
                    // Fallback: wrap raw text as a single uncited claim
                    CitedOutput {
                        title: "Analysis".to_string(),
                        claims: vec![CitedClaim {
                            text: truncate(&text),
                            sources: vec![],
                            confidence: "uncited".to_string(),
                        }],
                        evidence_status: EvidenceStatus::Insufficient,
                    }
                }
            };

            let val = serde_json::to_value(&cited_output).unwrap_or_default();
            RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
            StepResult::Continue
        }
        Err(e) => StepResult::Failed(format!("LLM error in ThinkCited: {}", e)),
    }
}

fn compute_evidence_status(
    claims: &[crate::recipe::CitedClaim],
    source_vars: &[String],
) -> crate::recipe::EvidenceStatus {
    use crate::recipe::EvidenceStatus;

    if claims.is_empty() {
        return EvidenceStatus::Insufficient;
    }

    let cited_count = claims.iter().filter(|c| !c.sources.is_empty()).count();
    let total = claims.len();

    if cited_count == 0 {
        return EvidenceStatus::Insufficient;
    }

    // Check for conflicting sources (same source cited for contradictory claims)
    // Simple heuristic: if > 50% of unique sources are used, good coverage
    let unique_sources: std::collections::HashSet<&str> = claims
        .iter()
        .flat_map(|c| c.sources.iter().map(|s| s.as_str()))
        .collect();

    let source_coverage = if source_vars.is_empty() {
        0.0
    } else {
        unique_sources.len() as f64 / source_vars.len() as f64
    };

    let cite_ratio = cited_count as f64 / total as f64;

    if cite_ratio >= 0.8 && source_coverage >= 0.6 {
        EvidenceStatus::Strong
    } else if cite_ratio >= 0.5 {
        EvidenceStatus::Moderate
    } else {
        EvidenceStatus::Thin
    }
}

// ── Validate: deterministic claim verification ──

fn execute_validate(
    service: &mut CompanionService,
    recipe_id: &str,
    input_var: &str,
    store_as: &str,
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {
    use crate::recipe::{CitedOutput, EvidenceStatus};

    let input = match vars.get(input_var) {
        Some(v) => v.clone(),
        None => return StepResult::Failed(format!("Validate: variable '{}' not found", input_var)),
    };

    let mut output: CitedOutput = match serde_json::from_value(input.clone()) {
        Ok(o) => o,
        Err(_) => {
            // If input is a plain string, wrap it
            let text = input.as_str().unwrap_or("").to_string();
            crate::recipe::CitedOutput {
                title: "Validation".to_string(),
                claims: vec![crate::recipe::CitedClaim {
                    text,
                    sources: vec![],
                    confidence: "uncited".to_string(),
                }],
                evidence_status: EvidenceStatus::Insufficient,
            }
        }
    };

    // Strip uncited claims
    let before_count = output.claims.len();
    output.claims.retain(|c| !c.sources.is_empty());
    let stripped = before_count - output.claims.len();

    // Recompute evidence status after stripping
    let cited_count = output.claims.len();
    output.evidence_status = if cited_count == 0 {
        EvidenceStatus::Insufficient
    } else if cited_count >= 3 {
        EvidenceStatus::Strong
    } else if cited_count >= 2 {
        EvidenceStatus::Moderate
    } else {
        EvidenceStatus::Thin
    };

    // Store validation report alongside
    let report = serde_json::json!({
        "total_claims": before_count,
        "cited_claims": cited_count,
        "stripped_uncited": stripped,
        "evidence_status": format!("{:?}", output.evidence_status),
    });
    let report_key = format!("{}_report", store_as);
    RecipeStore::set_var(service.db.conn(), recipe_id, &report_key, &report);

    // Store cleaned output
    let val = serde_json::to_value(&output).unwrap_or_default();
    RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);

    tracing::debug!(
        recipe_id,
        before = before_count,
        after = cited_count,
        stripped,
        "Validate step: claims filtered"
    );

    StepResult::Continue
}

// ── Render: format validated data for presentation ──

fn execute_render(
    service: &mut CompanionService,
    recipe_id: &str,
    input_var: &str,
    store_as: &str,
    format: &crate::recipe::RenderFormat,
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {
    use crate::recipe::{CitedOutput, EvidenceStatus, RenderFormat};

    let input = match vars.get(input_var) {
        Some(v) => v.clone(),
        None => return StepResult::Failed(format!("Render: variable '{}' not found", input_var)),
    };

    // Try to parse as CitedOutput; fall back to plain text
    let rendered = match serde_json::from_value::<CitedOutput>(input.clone()) {
        Ok(output) => render_cited_output(&output, format),
        Err(_) => {
            // Plain string — just pass through
            input.as_str().unwrap_or("(no data)").to_string()
        }
    };

    let val = serde_json::Value::String(rendered);
    RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
    StepResult::Continue
}

fn render_cited_output(
    output: &crate::recipe::CitedOutput,
    format: &crate::recipe::RenderFormat,
) -> String {
    use crate::recipe::{EvidenceStatus, RenderFormat};

    if output.claims.is_empty() {
        return format!(
            "**{}**\n\nNo verified information available.",
            output.title
        );
    }

    let evidence_label = match &output.evidence_status {
        EvidenceStatus::Strong => "Well-supported",
        EvidenceStatus::Moderate => "Moderately supported",
        EvidenceStatus::Thin => "Limited evidence",
        EvidenceStatus::Conflicting => "Conflicting sources",
        EvidenceStatus::Insufficient => "Insufficient evidence",
    };

    let mut result = format!("**{}** _({}__)_\n\n", output.title, evidence_label);

    match format {
        RenderFormat::Summary => {
            for claim in &output.claims {
                let marker = match claim.confidence.as_str() {
                    "high" => "",
                    "medium" => "",
                    "low" => " _(limited source)_",
                    _ => " _(unverified)_",
                };
                result.push_str(&format!("- {}{}\n", claim.text, marker));
            }
        }
        RenderFormat::Table => {
            result.push_str("| Finding | Confidence | Sources |\n");
            result.push_str("|---|---|---|\n");
            for claim in &output.claims {
                result.push_str(&format!(
                    "| {} | {} | {} |\n",
                    claim.text,
                    claim.confidence,
                    claim.sources.join(", ")
                ));
            }
        }
        RenderFormat::Comparison => {
            // Group claims by their first source for side-by-side
            for (i, claim) in output.claims.iter().enumerate() {
                result.push_str(&format!(
                    "**{}. {}**\n  Sources: {}\n  Confidence: {}\n\n",
                    i + 1,
                    claim.text,
                    if claim.sources.is_empty() {
                        "none".to_string()
                    } else {
                        claim.sources.join(", ")
                    },
                    claim.confidence
                ));
            }
        }
        RenderFormat::Cards => {
            for (i, claim) in output.claims.iter().enumerate() {
                result.push_str(&format!(
                    "┌─ {} ─────────────────────\n│ {}\n│ Sources: {} | Confidence: {}\n└─────────────────────────────\n\n",
                    i + 1,
                    claim.text,
                    claim.sources.join(", "),
                    claim.confidence
                ));
            }
        }
    }

    result
}

// ── Filter: filter JSON array by field comparison ──

fn execute_filter(
    service: &mut CompanionService,
    recipe_id: &str,
    input_var: &str,
    field: &str,
    op: &FilterOp,
    value: &str,
    store_as: &str,
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {

    let input = match vars.get(input_var) {
        Some(v) => v.clone(),
        None => return StepResult::Failed(format!("Filter: variable '{}' not found", input_var)),
    };

    // Parse the input as a JSON array (may be a string containing JSON or a direct array)
    let arr = match parse_json_array(&input) {
        Some(a) => a,
        None => return StepResult::Failed(format!("Filter: '{}' is not a JSON array", input_var)),
    };

    let filtered: Vec<serde_json::Value> = arr
        .into_iter()
        .filter(|item| {
            let field_val = item.get(field);
            match op {
                FilterOp::Equals => {
                    field_val.map_or(false, |v| value_matches_str(v, value))
                }
                FilterOp::NotEquals => {
                    field_val.map_or(true, |v| !value_matches_str(v, value))
                }
                FilterOp::Contains => {
                    field_val
                        .and_then(|v| v.as_str())
                        .map_or(false, |s| s.contains(value))
                }
                FilterOp::GreaterThan => {
                    compare_field_value(field_val, value)
                        .map_or(false, |ord| ord == std::cmp::Ordering::Greater)
                }
                FilterOp::LessThan => {
                    compare_field_value(field_val, value)
                        .map_or(false, |ord| ord == std::cmp::Ordering::Less)
                }
            }
        })
        .collect();

    let result = serde_json::to_string(&filtered).unwrap_or_else(|_| "[]".to_string());
    let val = serde_json::Value::String(truncate(&result));
    RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
    StepResult::Continue
}

// ── Sort: sort JSON array by field ──

fn execute_sort(
    service: &mut CompanionService,
    recipe_id: &str,
    input_var: &str,
    by_field: &str,
    descending: bool,
    store_as: &str,
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {
    let input = match vars.get(input_var) {
        Some(v) => v.clone(),
        None => return StepResult::Failed(format!("Sort: variable '{}' not found", input_var)),
    };

    let mut arr = match parse_json_array(&input) {
        Some(a) => a,
        None => return StepResult::Failed(format!("Sort: '{}' is not a JSON array", input_var)),
    };

    arr.sort_by(|a, b| {
        let va = a.get(by_field);
        let vb = b.get(by_field);
        let ord = compare_json_values(va, vb);
        if descending { ord.reverse() } else { ord }
    });

    let result = serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string());
    let val = serde_json::Value::String(truncate(&result));
    RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
    StepResult::Continue
}

// ── Aggregate: count/sum/min/max/avg over JSON array ──

fn execute_aggregate(
    service: &mut CompanionService,
    recipe_id: &str,
    input_var: &str,
    op: &AggregateOp,
    field: Option<&str>,
    store_as: &str,
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {

    let input = match vars.get(input_var) {
        Some(v) => v.clone(),
        None => return StepResult::Failed(format!("Aggregate: variable '{}' not found", input_var)),
    };

    let arr = match parse_json_array(&input) {
        Some(a) => a,
        None => return StepResult::Failed(format!("Aggregate: '{}' is not a JSON array", input_var)),
    };

    let result_str = match op {
        AggregateOp::Count => arr.len().to_string(),
        AggregateOp::Sum | AggregateOp::Min | AggregateOp::Max | AggregateOp::Avg => {
            let values: Vec<f64> = arr
                .iter()
                .filter_map(|item| {
                    let v = match field {
                        Some(f) => item.get(f)?,
                        None => item,
                    };
                    json_to_f64(v)
                })
                .collect();

            if values.is_empty() {
                "0".to_string()
            } else {
                match op {
                    AggregateOp::Sum => values.iter().sum::<f64>().to_string(),
                    AggregateOp::Min => values.iter().cloned().fold(f64::INFINITY, f64::min).to_string(),
                    AggregateOp::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max).to_string(),
                    AggregateOp::Avg => {
                        let sum: f64 = values.iter().sum();
                        (sum / values.len() as f64).to_string()
                    }
                    _ => unreachable!(),
                }
            }
        }
    };

    let val = serde_json::Value::String(result_str);
    RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
    StepResult::Continue
}

// ── Extract: key path traversal or regex extraction ──

fn execute_extract(
    service: &mut CompanionService,
    recipe_id: &str,
    input_var: &str,
    pattern: &str,
    store_as: &str,
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {
    let input = match vars.get(input_var) {
        Some(v) => v.clone(),
        None => return StepResult::Failed(format!("Extract: variable '{}' not found", input_var)),
    };

    if pattern.starts_with('/') {
        // Regex extraction — pattern is "/regex/"
        let regex_str = pattern.trim_start_matches('/').trim_end_matches('/');
        match regex::Regex::new(regex_str) {
            Ok(re) => {
                let text = match &input {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                let extracted = re
                    .find(&text)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                let val = serde_json::Value::String(extracted);
                RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
                StepResult::Continue
            }
            Err(e) => StepResult::Failed(format!("Extract: invalid regex '{}': {}", regex_str, e)),
        }
    } else {
        // Dot-notation key path traversal (e.g. "data.name" or "items.0.title")
        let parsed = match &input {
            serde_json::Value::String(s) => {
                serde_json::from_str::<serde_json::Value>(s).unwrap_or(input.clone())
            }
            other => other.clone(),
        };

        let mut current = &parsed;
        for key in pattern.split('.') {
            current = if let Ok(idx) = key.parse::<usize>() {
                match current.get(idx) {
                    Some(v) => v,
                    None => {
                        let val = serde_json::Value::String(String::new());
                        RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
                        return StepResult::Continue;
                    }
                }
            } else {
                match current.get(key) {
                    Some(v) => v,
                    None => {
                        let val = serde_json::Value::String(String::new());
                        RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
                        return StepResult::Continue;
                    }
                }
            };
        }

        let result = match current {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        let val = serde_json::Value::String(truncate(&result));
        RecipeStore::set_var(service.db.conn(), recipe_id, store_as, &val);
        StepResult::Continue
    }
}

// ── Branch: conditional execution of step lists ──

fn execute_branch(
    service: &mut CompanionService,
    recipe_id: &str,
    condition: &str,
    then_steps: &[RecipeStep],
    else_steps: &[RecipeStep],
    vars: &HashMap<String, serde_json::Value>,
) -> StepResult {
    // Evaluate condition: check if the variable named by `condition` is truthy.
    // A value is truthy if it exists, is non-empty, and is not "false" or "0".
    let is_truthy = vars.get(condition).map_or(false, |v| {
        match v {
            serde_json::Value::Null => false,
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Number(n) => n.as_f64().map_or(false, |f| f != 0.0),
            serde_json::Value::String(s) => {
                !s.is_empty() && s != "false" && s != "0"
            }
            serde_json::Value::Array(a) => !a.is_empty(),
            serde_json::Value::Object(o) => !o.is_empty(),
        }
    });

    let steps_to_run = if is_truthy { then_steps } else { else_steps };

    // Execute each sub-step inline, reloading vars between steps
    let mut current_vars = vars.clone();
    for sub_step in steps_to_run {
        let result = execute_step(service, recipe_id, sub_step, &current_vars);
        match result {
            StepResult::Continue | StepResult::Notify(_) => {
                if let StepResult::Notify(msg) = &result {
                    deliver_notification(service, recipe_id, msg);
                }
                // Reload vars after each sub-step
                current_vars = RecipeStore::get_vars(service.db.conn(), recipe_id);
            }
            StepResult::Failed(err) => return StepResult::Failed(err),
            StepResult::Waiting => return StepResult::Waiting,
            StepResult::Done => return StepResult::Done,
            StepResult::JumpTo(t) => return StepResult::JumpTo(t),
        }
    }

    StepResult::Continue
}

// ── JSON helpers for Filter/Sort/Aggregate ──

/// Parse a serde_json::Value into a Vec. Handles both direct arrays and strings containing JSON.
fn parse_json_array(val: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    match val {
        serde_json::Value::Array(a) => Some(a.clone()),
        serde_json::Value::String(s) => {
            serde_json::from_str::<Vec<serde_json::Value>>(s).ok()
        }
        _ => None,
    }
}

/// Check if a JSON value matches a string representation (handles numbers and strings).
fn value_matches_str(v: &serde_json::Value, s: &str) -> bool {
    match v {
        serde_json::Value::String(vs) => vs == s,
        serde_json::Value::Number(n) => n.to_string() == s,
        serde_json::Value::Bool(b) => b.to_string() == s,
        serde_json::Value::Null => s.is_empty() || s == "null",
        _ => false,
    }
}

/// Compare a JSON field value against a string threshold numerically.
fn compare_field_value(
    field_val: Option<&serde_json::Value>,
    threshold: &str,
) -> Option<std::cmp::Ordering> {
    let fv = json_to_f64(field_val?)?;
    let tv: f64 = threshold.parse().ok()?;
    fv.partial_cmp(&tv)
}

/// Compare two optional JSON values for sorting.
fn compare_json_values(
    a: Option<&serde_json::Value>,
    b: Option<&serde_json::Value>,
) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(va), Some(vb)) => {
            // Try numeric comparison first
            if let (Some(na), Some(nb)) = (json_to_f64(va), json_to_f64(vb)) {
                return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
            }
            // Fall back to string comparison
            let sa = va.as_str().map(|s| s.to_string())
                .unwrap_or_else(|| serde_json::to_string(va).unwrap_or_default());
            let sb = vb.as_str().map(|s| s.to_string())
                .unwrap_or_else(|| serde_json::to_string(vb).unwrap_or_default());
            sa.cmp(&sb)
        }
    }
}

/// Extract an f64 from a JSON value (handles numbers and numeric strings).
fn json_to_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Deliver a recipe notification as a proactive message.
fn deliver_notification(service: &mut CompanionService, _recipe_id: &str, message: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    service.set_proactive_message(crate::types::ProactiveMessage {
        text: message.to_string(),
        urge_ids: Vec::new(),
        generated_at: now,
    });
}

// ── Error Handling ──

enum ErrorResolution {
    Continue,
    JumpTo(usize),
    Retry,
    Abort,
}

fn handle_error(
    service: &mut CompanionService,
    recipe_id: &str,
    step_index: usize,
    error: &str,
    on_error: &ErrorAction,
    vars: &HashMap<String, serde_json::Value>,
) -> ErrorResolution {
    match on_error {
        ErrorAction::Fail => ErrorResolution::Abort,
        ErrorAction::Skip => ErrorResolution::Continue,
        ErrorAction::Retry { max } => {
            // Check retry count
            let retry_key = format!("_retry_{}", step_index);
            let current = vars
                .get(&retry_key)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if current < *max as u64 {
                let new_count = serde_json::Value::Number((current + 1).into());
                RecipeStore::set_var(service.db.conn(), recipe_id, &retry_key, &new_count);
                tracing::info!(
                    recipe_id, step = step_index,
                    retry = current + 1, max = *max,
                    "Recipe step retry"
                );
                ErrorResolution::Retry
            } else {
                tracing::warn!(
                    recipe_id, step = step_index,
                    "Recipe step exceeded max retries"
                );
                ErrorResolution::Abort
            }
        }
        ErrorAction::JumpTo { step } => ErrorResolution::JumpTo(*step),
        ErrorAction::Replan => {
            // Use LLM to diagnose and generate replacement steps
            match replan(service, recipe_id, step_index, error) {
                true => ErrorResolution::Continue, // Steps replaced, retry from current
                false => ErrorResolution::Abort,
            }
        }
    }
}

/// Use the LLM to diagnose a step failure and generate replacement steps.
fn replan(
    service: &mut CompanionService,
    recipe_id: &str,
    step_index: usize,
    error: &str,
) -> bool {
    let steps = RecipeStore::get_steps(service.db.conn(), recipe_id);
    let remaining: Vec<String> = steps
        .iter()
        .skip(step_index)
        .map(|s| serde_json::to_string(&s.step).unwrap_or_default())
        .collect();

    let prompt = format!(
        "A recipe step failed.\n\
         Failed step index: {}\n\
         Error: {}\n\
         Remaining steps: {}\n\n\
         Diagnose the failure and suggest fixed replacement steps as JSON array. \
         Each step must have a 'type' field (Tool, Think, Notify). \
         If the failure is unrecoverable, respond with an empty array [].",
        step_index,
        error,
        remaining.join(", ")
    );

    let messages = vec![
        ChatMessage::system(
            "You are a recipe debugger. Output ONLY a JSON array of replacement steps.",
        ),
        ChatMessage::user(&prompt),
    ];

    let gen_config = GenerationConfig {
        max_tokens: 1000,
        temperature: 0.2,
        ..Default::default()
    };

    match service.llm.chat(&messages, &gen_config, None) {
        Ok(r) => {
            let text = strip_think_tags(&r.text);
            // Try to extract JSON array from response
            let json_text = extract_json_array(&text);
            match serde_json::from_str::<Vec<RecipeStep>>(&json_text) {
                Ok(new_steps) if !new_steps.is_empty() => {
                    RecipeStore::replace_remaining_steps(
                        service.db.conn(),
                        recipe_id,
                        step_index,
                        &new_steps,
                    );

                    // Record learning
                    let step_info = steps
                        .get(step_index)
                        .map(|s| {
                            match &s.step {
                                RecipeStep::Tool { tool_name, .. } => tool_name.clone(),
                                _ => "unknown".to_string(),
                            }
                        })
                        .unwrap_or_default();

                    RecipeStore::record_failure_learning(
                        service.db.conn(),
                        recipe_id,
                        step_index,
                        &step_info,
                        error,
                        &format!("Replanned with {} new steps", new_steps.len()),
                    );

                    tracing::info!(
                        recipe_id,
                        step = step_index,
                        new_steps = new_steps.len(),
                        "Recipe replanned after failure"
                    );
                    true
                }
                Ok(_) => {
                    tracing::warn!(recipe_id, "Replan produced empty steps — aborting");
                    false
                }
                Err(e) => {
                    tracing::warn!(recipe_id, error = %e, "Replan failed to parse steps");
                    false
                }
            }
        }
        Err(e) => {
            tracing::warn!(recipe_id, error = %e, "Replan LLM call failed");
            false
        }
    }
}

// ── Helpers ──

fn truncate(s: &str) -> String {
    if s.len() > MAX_RESULT_SIZE {
        format!(
            "{}...(truncated)",
            &s[..s.floor_char_boundary(MAX_RESULT_SIZE)]
        )
    } else {
        s.to_string()
    }
}

fn strip_think_tags(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + "<think>".len()..];
        if let Some(end) = after.find("</think>") {
            remaining = &after[end + "</think>".len()..];
        } else {
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}

fn extract_json_array(text: &str) -> String {
    if let Some(start) = text.find('[') {
        if let Some(end) = text.rfind(']') {
            return text[start..=end].to_string();
        }
    }
    "[]".to_string()
}

fn extract_json_object(text: &str) -> String {
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return text[start..=end].to_string();
        }
    }
    "{}".to_string()
}

fn chrono_now() -> (u8, u8) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hour = ((secs % 86400) / 3600) as u8;
    let minute = ((secs % 3600) / 60) as u8;
    (hour, minute)
}
