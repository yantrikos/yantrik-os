//! Recipe Executor — processes pending/running recipes step by step.
//!
//! Called from the background cognition loop. Each tick processes one step
//! of each active recipe. Tool steps bypass the LLM; Think steps use it.
//! Recipes persist state in SQLite so they survive restarts.

use crate::companion::CompanionService;
use crate::recipe::{
    ErrorAction, RecipeStatus, RecipeStep, RecipeStore, StepResult,
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

        RecipeStep::Think { prompt, store_as } => {
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
                Err(e) => StepResult::Failed(format!("LLM error: {}", e)),
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
