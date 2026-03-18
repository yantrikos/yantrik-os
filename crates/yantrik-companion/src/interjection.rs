//! Interjection Classifier — rule-based classification of user input
//! during active recipe execution.
//!
//! Pure pattern matching + keyword detection. No LLM call needed.
//! Must be fast enough to run on every incoming message when a recipe is active.

use crate::recipe::{RecipeStatus, RecipeStore};
use rusqlite::Connection;

/// Classification of a user message during active recipe execution.
#[derive(Debug, Clone, PartialEq)]
pub enum Interjection {
    /// User wants to stop the recipe entirely.
    /// Keywords: "cancel", "stop", "abort", "nevermind", "forget it"
    Cancel,
    /// User wants to pause the recipe and resume later.
    /// Keywords: "pause", "hold on", "wait", "not now", "later"
    Pause,
    /// User is answering an AskUser prompt from the recipe.
    /// Detected when a recipe is in Waiting state with an AskUser step.
    AnswerAskUser {
        recipe_id: String,
        answer: String,
    },
    /// User wants to change parameters and restart the recipe.
    /// Keywords: "change", "actually", "instead", "modify", "redo"
    ModifyAndRestart {
        modification: String,
    },
    /// User's message is unrelated to the active recipe.
    /// Handle normally without disrupting the recipe.
    OutOfBandChat,
}

/// Classify user input in the context of active recipes.
///
/// Returns `None` if no recipe is active (normal message flow).
/// Returns `Some(Interjection)` if a recipe is running/waiting.
pub fn classify(conn: &Connection, user_text: &str) -> Option<Interjection> {
    // Check if any recipe is active
    let active = RecipeStore::list(conn, Some("running"), 1);
    let waiting = RecipeStore::list(conn, Some("waiting"), 1);

    if active.is_empty() && waiting.is_empty() {
        return None; // No active recipe — normal message flow
    }

    let text_lower = user_text.trim().to_lowercase();

    // 1. Check for cancel intent (highest priority)
    if is_cancel(&text_lower) {
        return Some(Interjection::Cancel);
    }

    // 2. Check for pause intent
    if is_pause(&text_lower) {
        return Some(Interjection::Pause);
    }

    // 3. Check for AskUser answer (recipe is waiting for user input)
    if let Some(recipe) = waiting.first() {
        let steps = RecipeStore::get_steps(conn, &recipe.id);
        // The current step is the one the recipe is waiting on
        // (current_step was advanced past the WaitFor/AskUser step)
        let wait_step = recipe.current_step.saturating_sub(1);
        if let Some(stored) = steps.get(wait_step) {
            if let crate::recipe::RecipeStep::AskUser { store_as, .. } = &stored.step {
                // This looks like an answer to the AskUser prompt
                // Unless it's clearly a cancel/pause/modify command
                if !is_modify(&text_lower) {
                    return Some(Interjection::AnswerAskUser {
                        recipe_id: recipe.id.clone(),
                        answer: user_text.to_string(),
                    });
                }
            }
        }
    }

    // 4. Check for modify/restart intent
    if is_modify(&text_lower) {
        return Some(Interjection::ModifyAndRestart {
            modification: user_text.to_string(),
        });
    }

    // 5. Default: out-of-band chat
    // The message doesn't seem related to recipe control — handle normally
    Some(Interjection::OutOfBandChat)
}

/// Handle the classified interjection. Returns an optional response message.
pub fn handle(conn: &Connection, interjection: &Interjection) -> Option<String> {
    match interjection {
        Interjection::Cancel => {
            // Cancel all running/waiting recipes
            let running = RecipeStore::list(conn, Some("running"), 10);
            let waiting = RecipeStore::list(conn, Some("waiting"), 10);
            let mut cancelled = 0;
            for recipe in running.iter().chain(waiting.iter()) {
                RecipeStore::set_error(conn, &recipe.id, "Cancelled by user");
                cancelled += 1;
            }
            if cancelled > 0 {
                Some(format!("Cancelled {} active recipe(s).", cancelled))
            } else {
                Some("No active recipes to cancel.".to_string())
            }
        }
        Interjection::Pause => {
            let running = RecipeStore::list(conn, Some("running"), 10);
            let mut paused = 0;
            for recipe in &running {
                RecipeStore::update_status(
                    conn,
                    &recipe.id,
                    &RecipeStatus::Waiting,
                    recipe.current_step,
                );
                paused += 1;
            }
            if paused > 0 {
                Some(format!(
                    "Paused {} recipe(s). Say 'resume' or 'continue' to restart.",
                    paused
                ))
            } else {
                Some("No running recipes to pause.".to_string())
            }
        }
        Interjection::AnswerAskUser {
            recipe_id,
            answer,
        } => {
            // Store the answer in recipe vars and resume
            let recipe = RecipeStore::get(conn, recipe_id)?;
            let steps = RecipeStore::get_steps(conn, recipe_id);
            let wait_step = recipe.current_step.saturating_sub(1);

            if let Some(stored) = steps.get(wait_step) {
                if let crate::recipe::RecipeStep::AskUser { store_as, .. } = &stored.step {
                    // Store the answer
                    let val = serde_json::Value::String(answer.clone());
                    RecipeStore::set_var(conn, recipe_id, store_as, &val);
                    // Mark step done and resume
                    RecipeStore::complete_step(conn, recipe_id, wait_step, answer);
                    RecipeStore::update_status(
                        conn,
                        recipe_id,
                        &RecipeStatus::Running,
                        recipe.current_step,
                    );
                    return Some(format!("Got it. Resuming recipe '{}'...", recipe.name));
                }
            }
            None
        }
        Interjection::ModifyAndRestart { modification } => {
            Some(format!(
                "To modify and restart a recipe, use: run_recipe with updated variables. \
                 Your modification: {}",
                modification
            ))
        }
        Interjection::OutOfBandChat => {
            // Don't interfere — let normal message handling proceed
            None
        }
    }
}

// ── Pattern Matching ──

fn is_cancel(text: &str) -> bool {
    let exact = [
        "cancel",
        "stop",
        "abort",
        "nevermind",
        "never mind",
        "forget it",
        "quit",
        "exit",
        "stop it",
        "cancel that",
        "stop that",
        "cancel recipe",
        "stop recipe",
    ];
    if exact.iter().any(|p| text == *p) {
        return true;
    }

    // Prefix patterns
    let prefixes = ["cancel ", "stop the ", "abort the "];
    if prefixes.iter().any(|p| text.starts_with(p)) {
        return true;
    }

    false
}

fn is_pause(text: &str) -> bool {
    let exact = [
        "pause",
        "hold on",
        "hold",
        "wait",
        "not now",
        "later",
        "pause recipe",
        "hold that",
        "one sec",
        "one moment",
        "hang on",
    ];
    if exact.iter().any(|p| text == *p) {
        return true;
    }

    let prefixes = ["pause ", "hold on ", "wait "];
    if prefixes.iter().any(|p| text.starts_with(p)) {
        return true;
    }

    false
}

fn is_modify(text: &str) -> bool {
    let prefixes = [
        "actually ",
        "instead ",
        "change ",
        "modify ",
        "redo ",
        "restart with ",
        "try again with ",
        "change it to ",
        "use a different ",
    ];
    prefixes.iter().any(|p| text.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancel_detection() {
        assert!(is_cancel("cancel"));
        assert!(is_cancel("stop"));
        assert!(is_cancel("abort"));
        assert!(is_cancel("nevermind"));
        assert!(is_cancel("cancel that"));
        assert!(is_cancel("cancel recipe"));
        assert!(!is_cancel("cancel my appointment")); // This is ambiguous but we catch it via prefix
        assert!(!is_cancel("how do i cancel")); // Not a cancel command
    }

    #[test]
    fn test_pause_detection() {
        assert!(is_pause("pause"));
        assert!(is_pause("hold on"));
        assert!(is_pause("not now"));
        assert!(is_pause("hang on"));
        assert!(!is_pause("how long will it pause"));
    }

    #[test]
    fn test_modify_detection() {
        assert!(is_modify("actually use python instead"));
        assert!(is_modify("change it to morning"));
        assert!(is_modify("redo with fewer steps"));
        assert!(!is_modify("that's actually great"));
    }
}
