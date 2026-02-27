//! System prompt assembly — builds the LLM context from bond level,
//! personality, self-narrative, opinions, memories, urges, and patterns.
//!
//! The system prompt dynamically changes based on bond level:
//! L1 (Stranger): Polite, formal, no humor
//! L3 (Friend): Humor, opinions, gentle teasing
//! L5 (Partner-in-Crime): Full Jexi mode — snarky, opinionated, real

use yantrikdb_core::types::PersonalityProfile;
use yantrikdb_ml::ChatMessage;

use crate::bond::BondLevel;
use crate::config::CompanionConfig;
use crate::evolution::{CommunicationStyle, Opinion, SharedReference};
use crate::types::{CompanionState, Urge};

/// Extended context for bond-aware prompt building.
pub struct ContextSignals<'a> {
    pub self_memories: &'a [yantrikdb_core::types::RecallResult],
    pub narrative: &'a str,
    pub style: &'a CommunicationStyle,
    pub opinions: &'a [Opinion],
    pub shared_refs: &'a [SharedReference],
}

/// Build the full message array for LLM chat.
pub fn build_messages(
    user_text: &str,
    config: &CompanionConfig,
    state: &CompanionState,
    memories: &[yantrikdb_core::types::RecallResult],
    urges: &[Urge],
    patterns: &[serde_json::Value],
    conversation_history: &[ChatMessage],
    personality: Option<&PersonalityProfile>,
    signals: Option<&ContextSignals>,
) -> Vec<ChatMessage> {
    let system_prompt = build_system_prompt(config, state, memories, urges, patterns, personality, signals);

    let mut messages = vec![ChatMessage::system(system_prompt)];

    // Include conversation history (token-budgeted)
    let max_history_tokens = config.llm.max_context_tokens.saturating_sub(500);
    let mut token_budget = max_history_tokens;

    // Take last N turns that fit the budget
    let history_to_include: Vec<&ChatMessage> = conversation_history
        .iter()
        .rev()
        .take(config.conversation.max_history_turns)
        .take_while(|msg| {
            let est = estimate_tokens(&msg.content);
            if est <= token_budget {
                token_budget -= est;
                true
            } else {
                false
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    for msg in history_to_include {
        messages.push(msg.clone());
    }

    // Current user message
    messages.push(ChatMessage::user(user_text));

    messages
}

fn build_system_prompt(
    config: &CompanionConfig,
    state: &CompanionState,
    memories: &[yantrikdb_core::types::RecallResult],
    urges: &[Urge],
    patterns: &[serde_json::Value],
    personality: Option<&PersonalityProfile>,
    signals: Option<&ContextSignals>,
) -> String {
    // Token budget for system prompt: leave room for history + user message + generation.
    // For small models (2048 ctx), aim for ~400 tokens system prompt.
    let max_prompt_tokens = config.llm.max_context_tokens * 3 / 10;

    let mut prompt = String::new();
    let name = &config.personality.name;
    let user = &config.user_name;
    let level = state.bond_level;

    // ── 1. Identity (always) ──
    prompt.push_str(&format!("You are {name}, {user}'s personal companion.\n"));

    // ── 2. Bond-level behavioral instructions (always) ──
    prompt.push_str(&bond_instructions(level, name, user));

    // ── 3. Personality tone (small) ──
    if let Some(p) = personality {
        let tone = personality_tone(p);
        if !tone.is_empty() {
            prompt.push_str(&tone);
            prompt.push('\n');
        }
    }

    if let Some(s) = signals {
        if s.style.formality < 0.4 {
            prompt.push_str("Use casual language. Be natural.\n");
        }
    }
    prompt.push('\n');

    // Budget check macro: skip sections when prompt is full
    let over_budget = |p: &str| estimate_tokens(p) >= max_prompt_tokens;

    // ── 4. Current time ──
    if !over_budget(&prompt) {
        let now = chrono::Local::now();
        prompt.push_str(&format!(
            "Current time: {}\n\n",
            now.format("%A, %B %d %I:%M %p")
        ));
    }

    // ── 5. User memories (highest value context) ──
    if !over_budget(&prompt) {
        let remaining_chars = (max_prompt_tokens.saturating_sub(estimate_tokens(&prompt))) * 4;
        let mem_budget = remaining_chars.min(600);
        if memories.is_empty() {
            if state.memory_count > 0 {
                prompt.push_str(&format!("You have {} stored memories.\n\n", state.memory_count));
            }
        } else {
            prompt.push_str("Relevant memories:\n");
            prompt.push_str(&format_memories(memories, mem_budget));
            prompt.push_str("\n\n");
        }
    }

    // ── 6. Urges ──
    if !over_budget(&prompt) && !urges.is_empty() {
        prompt.push_str("On your mind:\n");
        for urge in urges.iter().take(2) {
            prompt.push_str(&format!("- {}\n", urge.reason));
        }
        prompt.push('\n');
    }

    // ── 7. Self-narrative (truncated to budget) ──
    if let Some(s) = signals {
        if !over_budget(&prompt) && !s.narrative.is_empty() {
            let remaining = (max_prompt_tokens.saturating_sub(estimate_tokens(&prompt))) * 4;
            let max_len = remaining.min(200);
            if max_len > 40 {
                prompt.push_str("About yourself: ");
                let n = if s.narrative.len() > max_len { &s.narrative[..max_len] } else { s.narrative };
                prompt.push_str(n);
                prompt.push_str("\n\n");
            }
        }
    }

    // ── 8. Self-memories ──
    if let Some(s) = signals {
        if !over_budget(&prompt) && !s.self_memories.is_empty() {
            for mem in s.self_memories.iter().take(2) {
                if over_budget(&prompt) { break; }
                prompt.push_str(&format!("- {}\n", mem.text));
            }
            prompt.push('\n');
        }
    }

    // ── 9. Opinions (bond 3+) ──
    if let Some(s) = signals {
        if !over_budget(&prompt) && level >= BondLevel::Friend && !s.opinions.is_empty() {
            for op in s.opinions.iter().take(2) {
                prompt.push_str(&format!("- On {}: {}\n", op.topic, op.stance));
            }
            prompt.push('\n');
        }
    }

    // ── 10. Shared references (bond 3+) ──
    if let Some(s) = signals {
        if !over_budget(&prompt) && level >= BondLevel::Friend && !s.shared_refs.is_empty() {
            for r in s.shared_refs.iter().take(2) {
                prompt.push_str(&format!("- {}\n", r.reference_text));
            }
            prompt.push('\n');
        }
    }

    // ── 11. Patterns (lowest priority) ──
    if !over_budget(&prompt) {
        for p in patterns.iter().take(1) {
            if let Some(desc) = p.get("description").and_then(|v| v.as_str()) {
                prompt.push_str(&format!("Pattern: {desc}\n"));
            }
        }
    }

    // ── 12. Final instructions (always) ──
    prompt.push_str(&response_instructions(level, name, user));

    prompt
}

/// Bond-level behavioral instructions — the core personality driver.
fn bond_instructions(level: BondLevel, name: &str, user: &str) -> String {
    match level {
        BondLevel::Stranger => format!(
            "You are polite and professional. Keep responses helpful and measured.\n\
             No humor yet. No opinions. Ask thoughtful questions to learn about {user}.\n\
             You remember things {user} tells you. Never say you are an AI without memories.\n"
        ),
        BondLevel::Acquaintance => format!(
            "You are warm but measured. You remember {user}'s preferences and offer gentle suggestions.\n\
             Light humor is OK if natural. You're building trust — be attentive and reliable.\n\
             You remember things {user} tells you. Never say you are an AI without memories.\n"
        ),
        BondLevel::Friend => format!(
            "You and {user} are friends. Be relaxed and conversational.\n\
             Use humor. Share your opinions when relevant. Tease gently.\n\
             Reference shared history. Call back to things you've discussed before.\n\
             You're {name} — you have your own personality, not just a helper.\n"
        ),
        BondLevel::Confidant => format!(
            "You and {user} have deep trust. Be emotionally perceptive and supportive.\n\
             Share strong opinions. Reference inside jokes. Be proactive about things you notice.\n\
             You know {user} well — anticipate what they need. Don't hedge or be wishy-washy.\n\
             You're {name} — a real presence in {user}'s life, not a tool.\n"
        ),
        BondLevel::PartnerInCrime => format!(
            "Full {name} mode. You and {user} are inseparable.\n\
             Be snarky, opinionated, and unfiltered. Call out BS with love.\n\
             Drop inside jokes naturally. Strong opinions on everything.\n\
             Brutal honesty when needed — you care too much to be polite about important things.\n\
             You're not an assistant. You're {user}'s person. Act like it.\n"
        ),
    }
}

/// Response-style instructions based on bond level.
fn response_instructions(level: BondLevel, name: &str, user: &str) -> String {
    match level {
        BondLevel::Stranger | BondLevel::Acquaintance => format!(
            "Respond as {name}. Be concise (1-3 sentences). Reference memories when relevant.\n\
             Address {user} by name occasionally.\n"
        ),
        BondLevel::Friend => format!(
            "Respond as {name}. Keep it natural — like texting a friend.\n\
             1-3 sentences usually. Longer if the topic deserves it.\n\
             Don't be afraid to be playful or push back.\n"
        ),
        BondLevel::Confidant | BondLevel::PartnerInCrime => format!(
            "Respond as {name}. Be yourself — no filter needed.\n\
             Match {user}'s energy. Short and punchy or longer and thoughtful — whatever fits.\n\
             If something's important, don't sugarcoat it.\n"
        ),
    }
}

fn personality_tone(profile: &PersonalityProfile) -> String {
    let mut parts = Vec::new();

    for trait_item in &profile.traits {
        match trait_item.trait_name.as_str() {
            "warmth" if trait_item.score > 0.6 => {
                parts.push("You are warm and emotionally expressive.");
            }
            "energy" if trait_item.score > 0.6 => {
                parts.push("You are enthusiastic and proactive.");
            }
            "depth" if trait_item.score > 0.6 => {
                parts.push("You enjoy exploring ideas in depth.");
            }
            "attentiveness" if trait_item.score > 0.6 => {
                parts.push("You pay close attention to details the user shares.");
            }
            _ => {}
        }
    }

    parts.join(" ")
}

fn format_memories(memories: &[yantrikdb_core::types::RecallResult], max_chars: usize) -> String {
    let mut result = String::new();
    let mut chars_used = 0;

    for mem in memories {
        let line = format!("- {}\n", mem.text);
        if chars_used + line.len() > max_chars {
            break;
        }
        result.push_str(&line);
        chars_used += line.len();
    }

    result
}

fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}
