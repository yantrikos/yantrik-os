//! Synthesis Gate — pre-delivery filter for proactive messages.
//!
//! Runs before any proactive message is delivered. Checks:
//! 1. Semantic similarity against recent messages (anti-repetition)
//! 2. Daily message budget (bond-scaled)
//! 3. Conversation suppression (no interrupting active conversation)
//! 4. Strategic silence (probabilistic suppression of low-urgency urges)
//! 5. Multi-urge merging (narrative braiding)
//!
//! No LLM calls — pure Rust logic. Zero latency cost.

use crate::bond::BondLevel;

/// Result of the synthesis gate evaluation.
#[derive(Debug)]
pub enum GateDecision {
    /// Deliver the message as-is.
    Deliver,
    /// Suppress — don't deliver.
    Suppress { reason: String },
    /// Merge multiple urge texts into a braiding prompt for the LLM.
    Braid { merged_prompt: String },
}

/// Check if a new message is too similar to any recently sent message.
///
/// Uses simple word-overlap Jaccard similarity (no embeddings needed).
/// Returns the highest similarity score found.
pub fn check_similarity(new_message: &str, recent_messages: &[String]) -> f64 {
    if recent_messages.is_empty() || new_message.is_empty() {
        return 0.0;
    }

    let new_words: std::collections::HashSet<&str> = new_message
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .into_iter()
        .collect();

    // We need to own the lowercased strings
    let new_lower = new_message.to_lowercase();
    let new_words: std::collections::HashSet<&str> = new_lower.split_whitespace().collect();

    let mut max_sim = 0.0f64;
    for msg in recent_messages {
        let msg_lower = msg.to_lowercase();
        let msg_words: std::collections::HashSet<&str> = msg_lower.split_whitespace().collect();

        let intersection = new_words.intersection(&msg_words).count();
        let union = new_words.union(&msg_words).count();
        if union > 0 {
            let jaccard = intersection as f64 / union as f64;
            max_sim = max_sim.max(jaccard);
        }
    }

    max_sim
}

/// Check if the message starts similarly to recent messages.
/// Returns true if the opening words (first 4) match any recent message.
pub fn check_opening_similarity(new_message: &str, recent_messages: &[String]) -> bool {
    let new_opening: String = new_message
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    if new_opening.is_empty() {
        return false;
    }

    for msg in recent_messages {
        let msg_opening: String = msg
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if msg_opening == new_opening {
            return true;
        }
    }

    false
}

/// Get the daily message budget for a bond level.
pub fn daily_budget(bond_level: BondLevel) -> u32 {
    match bond_level {
        BondLevel::Stranger => 3,
        BondLevel::Acquaintance => 5,
        BondLevel::Friend => 8,
        BondLevel::Confidant => 10,
        BondLevel::PartnerInCrime => 14,
    }
}

/// Apply jitter to a cooldown value (±20% random variation).
pub fn jitter_cooldown(base_secs: f64) -> f64 {
    // Simple deterministic jitter using system time fractional seconds
    let frac = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as f64
        / 1_000_000_000.0;
    let factor = 0.8 + frac * 0.4; // 0.8 to 1.2
    base_secs * factor
}

/// Run the full synthesis gate on a proactive message before delivery.
///
/// Returns a `GateDecision` indicating whether to deliver, suppress, or braid.
pub fn evaluate(
    message_text: &str,
    recent_messages: &[String],
    daily_count: u32,
    bond_level: BondLevel,
    idle_seconds: f64,
    conversation_active: bool,
    urgency: f64,
) -> GateDecision {
    // 1. Conversation suppression — don't interrupt active chats
    if conversation_active && idle_seconds < 300.0 {
        return GateDecision::Suppress {
            reason: "User in active conversation".into(),
        };
    }

    // 2. Daily budget check
    let budget = daily_budget(bond_level);
    if daily_count >= budget {
        return GateDecision::Suppress {
            reason: format!("Daily budget exceeded ({}/{})", daily_count, budget),
        };
    }

    // 3. Similarity check — block repetitive messages
    let similarity = check_similarity(message_text, recent_messages);
    if similarity > 0.6 {
        return GateDecision::Suppress {
            reason: format!("Too similar to recent message (sim={:.2})", similarity),
        };
    }

    // 4. Opening similarity — don't start messages the same way
    if check_opening_similarity(message_text, recent_messages) {
        return GateDecision::Suppress {
            reason: "Same opening as recent message".into(),
        };
    }

    // 5. Strategic silence — probabilistic suppression of low-urgency messages
    //    Suppress ~30% of messages below urgency 0.5
    if urgency < 0.5 {
        // Use fractional system time as cheap random
        let pseudo_rand = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_millis() as f64
            / 1000.0;
        if pseudo_rand < 0.3 {
            return GateDecision::Suppress {
                reason: "Strategic silence (low urgency, random suppression)".into(),
            };
        }
    }

    GateDecision::Deliver
}

/// Build a narrative braiding prompt from multiple urge texts.
///
/// Used when multiple urges fire in the same cycle — merges them into
/// one natural message via a single LLM call.
pub fn build_braid_prompt(urge_texts: &[String], recent_messages: &[String]) -> String {
    let mut prompt = String::from(
        "You have multiple observations to share. Weave them into ONE natural message \
         that flows like a single thought. Don't list them — blend them.\n\n",
    );

    prompt.push_str("Observations to weave:\n");
    for (i, text) in urge_texts.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", i + 1, text));
    }

    if !recent_messages.is_empty() {
        prompt.push_str("\nYour recent messages (DON'T repeat themes or openings):\n");
        for msg in recent_messages.iter().rev().take(3) {
            prompt.push_str(&format!("- {}\n", msg));
        }
    }

    prompt.push_str(
        "\nKeep it to 2-3 sentences. Sound natural, not like a bulletin. \
         Vary your phrasing from the recent messages above.",
    );

    prompt
}

/// Build an anti-repetition instruction to prepend to EXECUTE prompts.
/// Also includes conversational metabolism guidance based on user message length.
pub fn anti_repetition_instruction(recent_messages: &[String], avg_user_msg_length: f64) -> String {
    let mut instruction = String::new();

    // Conversational metabolism: match user's energy
    if avg_user_msg_length > 0.0 {
        if avg_user_msg_length < 30.0 {
            instruction.push_str(
                "\n[TONE: The user communicates in short messages. Keep your response brief — \
                 1 sentence max. Be light and casual, not deep.]\n",
            );
        } else if avg_user_msg_length < 80.0 {
            instruction.push_str(
                "\n[TONE: Match the user's moderate message style. Keep to 1-2 sentences.]\n",
            );
        }
        // For longer messages (>80), no constraint — companion can go deeper
    }

    if recent_messages.is_empty() {
        return instruction;
    }

    let last_n: Vec<&String> = recent_messages.iter().rev().take(5).collect();
    instruction.push_str(
        "\n[IMPORTANT: Vary your phrasing. Here are your recent messages — \
         do NOT start with similar words, use similar structure, or repeat themes:\n",
    );
    for (i, msg) in last_n.iter().enumerate() {
        let truncated = if msg.chars().count() > 80 {
            let s: String = msg.chars().take(80).collect();
            format!("{}...", s)
        } else {
            msg.to_string()
        };
        instruction.push_str(&format!("{}. {}\n", i + 1, truncated));
    }
    instruction.push_str("]\n");
    instruction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_similarity_empty() {
        assert_eq!(check_similarity("hello", &[]), 0.0);
        assert_eq!(check_similarity("", &["hello".into()]), 0.0);
    }

    #[test]
    fn test_similarity_identical() {
        let sim = check_similarity("hello world", &["hello world".into()]);
        assert!(sim > 0.9);
    }

    #[test]
    fn test_similarity_different() {
        let sim = check_similarity(
            "the weather is nice today",
            &["debugging rust code is fun".into()],
        );
        assert!(sim < 0.3);
    }

    #[test]
    fn test_opening_similarity() {
        assert!(check_opening_similarity(
            "Hey there, how are you doing today?",
            &["Hey there, how is the weather?".into()]
        ));
        assert!(!check_opening_similarity(
            "Good morning, ready to start?",
            &["Hey there, how is the weather?".into()]
        ));
    }

    #[test]
    fn test_daily_budget() {
        assert_eq!(daily_budget(BondLevel::Stranger), 3);
        assert_eq!(daily_budget(BondLevel::PartnerInCrime), 14);
    }
}
