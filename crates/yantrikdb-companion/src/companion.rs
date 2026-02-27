//! CompanionService — the main agent brain.
//!
//! Ties together YantrikDB (memory), LLMEngine (inference), instincts (drives),
//! urges (action queue), learning (memory extraction), bond tracking,
//! personality evolution, and self-narrative into a single 9-step pipeline.

use yantrikdb_core::YantrikDB;
use yantrikdb_ml::{
    ChatMessage, GenerationConfig, LLMBackend,
    format_tools, parse_tool_calls, extract_text_content,
};

use crate::bond::{BondLevel, BondTracker};
use crate::config::CompanionConfig;
use crate::context::{self, ContextSignals};
use crate::evolution::Evolution;
use crate::instincts::{self, Instinct};
use crate::learning;
use crate::narrative::Narrative;
use crate::tools;
use crate::types::{AgentResponse, CompanionState, ProactiveMessage};
use crate::urges::UrgeQueue;

/// The companion agent — memory + inference + instincts + bond + evolution in one struct.
pub struct CompanionService {
    pub db: YantrikDB,
    pub llm: Box<dyn LLMBackend>,
    pub config: CompanionConfig,
    pub urge_queue: UrgeQueue,
    instincts: Vec<Box<dyn Instinct>>,

    // Conversation state
    conversation_history: Vec<ChatMessage>,
    last_interaction_ts: f64,
    session_turn_count: usize,
    proactive_message: Option<ProactiveMessage>,

    // Cached from last think()
    pending_triggers: Vec<serde_json::Value>,
    active_patterns: Vec<serde_json::Value>,
    open_conflicts_count: usize,
    recent_valence_avg: Option<f64>,

    // Soul state — cached from DB per interaction
    bond_level: BondLevel,
    bond_score: f64,
    bond_level_changed: bool,
}

impl CompanionService {
    /// Create a new companion from pre-built YantrikDB and LLM backend.
    pub fn new(db: YantrikDB, llm: Box<dyn LLMBackend>, config: CompanionConfig) -> Self {
        // Ensure soul tables exist
        BondTracker::ensure_tables(db.conn());
        Evolution::ensure_tables(db.conn());
        Narrative::ensure_table(db.conn());

        let urge_queue = UrgeQueue::new(db.conn(), config.urges.clone());
        let instincts = instincts::load_instincts(&config.instincts);

        // Load current bond state
        let bond_state = BondTracker::get_state(db.conn());

        Self {
            db,
            llm,
            config,
            urge_queue,
            instincts,
            conversation_history: Vec::new(),
            last_interaction_ts: now_ts(),
            session_turn_count: 0,
            proactive_message: None,
            pending_triggers: Vec::new(),
            active_patterns: Vec::new(),
            open_conflicts_count: 0,
            recent_valence_avg: None,
            bond_level: bond_state.bond_level,
            bond_score: bond_state.bond_score,
            bond_level_changed: false,
        }
    }

    /// The 9-step message pipeline.
    pub fn handle_message(&mut self, user_text: &str) -> AgentResponse {
        // Step 1: Check session timeout
        self.check_session_timeout();

        // Step 2: Recall relevant user memories
        let memories = self
            .db
            .recall_text(user_text, 5)
            .unwrap_or_default();

        // Step 3: Recall self-memories (reflections about the companion itself)
        let self_memories = self
            .db
            .recall_text(&format!("self: {user_text}"), 10)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| {
                r.source == "self" || r.domain == "self-reflection"
            })
            .take(3)
            .collect::<Vec<_>>();

        // Step 4: Pop urges for this interaction
        let urges = self
            .urge_queue
            .pop_for_interaction(self.db.conn(), 2);
        let urge_ids: Vec<String> = urges.iter().map(|u| u.urge_id.clone()).collect();

        // Detect humor reaction from previous exchange
        learning::detect_humor_reaction(self.db.conn(), user_text);

        // Step 5: Evaluate instincts on interaction
        let state = self.build_state();
        for instinct in &self.instincts {
            let specs = instinct.on_interaction(&state, user_text);
            for spec in specs {
                self.urge_queue.push(self.db.conn(), &spec);
            }
        }

        // Step 6: Build bond-aware LLM context
        let personality = self.db.get_personality().ok();
        let patterns_json: Vec<serde_json::Value> = self
            .active_patterns
            .iter()
            .cloned()
            .collect();

        // Gather soul signals
        let narrative_text = Narrative::get(self.db.conn());
        let style = Evolution::get_style(self.db.conn());
        let opinions = Evolution::get_opinions(self.db.conn(), 3);
        let shared_refs = Evolution::get_shared_references(self.db.conn(), 3);

        let signals = ContextSignals {
            self_memories: &self_memories,
            narrative: &narrative_text,
            style: &style,
            opinions: &opinions,
            shared_refs: &shared_refs,
        };

        let mut messages = context::build_messages(
            user_text,
            &self.config,
            &state,
            &memories,
            &urges,
            &patterns_json,
            &self.conversation_history,
            personality.as_ref(),
            Some(&signals),
        );

        // Add tool definitions to system message if tools enabled
        if self.config.tools.enabled {
            let tool_defs = tools::companion_tool_defs();
            let tool_text = format_tools(&tool_defs);
            if let Some(sys_msg) = messages.first_mut() {
                sys_msg.content.push_str(&tool_text);
            }
        }

        // Step 7: Call LLM with tool loop
        let gen_config = GenerationConfig {
            max_tokens: self.config.llm.max_tokens,
            temperature: self.config.llm.temperature,
            top_p: Some(0.9),
            ..Default::default()
        };

        let mut tool_calls_made = Vec::new();
        let mut response_text = String::new();

        for _round in 0..self.config.tools.max_tool_rounds {
            let llm_response = match self.llm.chat(&messages, &gen_config) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("LLM chat failed: {e}");
                    response_text = format!("I'm having trouble thinking right now. ({e})");
                    break;
                }
            };

            let tool_calls = parse_tool_calls(&llm_response.text);

            if tool_calls.is_empty() {
                // No tool calls — final response
                response_text = llm_response.text;
                break;
            }

            // Execute tool calls
            let text_part = extract_text_content(&llm_response.text);
            messages.push(ChatMessage::assistant(&llm_response.text));

            for call in &tool_calls {
                tool_calls_made.push(call.name.clone());
                let result = tools::execute_tool(&self.db, &call.name, &call.arguments);

                // Add tool result as user message (tool response)
                messages.push(ChatMessage::user(format!(
                    "Tool result for {}: {result}",
                    call.name
                )));
            }

            // If this is the last round, use what we have
            if !text_part.is_empty() {
                response_text = text_part;
            }
        }

        // If we exhausted rounds without a clean response, use last text
        if response_text.is_empty() {
            response_text = "I'm here. How can I help?".to_string();
        }

        // Clean up tool call XML from final response
        response_text = extract_text_content(&response_text);

        // Update conversation history
        self.conversation_history
            .push(ChatMessage::user(user_text));
        self.conversation_history
            .push(ChatMessage::assistant(&response_text));

        // Trim history to max turns
        let max = self.config.conversation.max_history_turns * 2;
        if self.conversation_history.len() > max {
            let drain = self.conversation_history.len() - max;
            self.conversation_history.drain(..drain);
        }

        self.last_interaction_ts = now_ts();
        self.session_turn_count += 1;

        // Step 8: Learn from this exchange (includes self-reflection)
        learning::extract_and_learn(&self.db, &*self.llm, user_text, &response_text);

        // Step 9: Score bond + tick evolution
        if self.config.bond.enabled {
            let (new_level, level_changed) = BondTracker::score_interaction(
                self.db.conn(),
                user_text,
                &response_text,
                memories.len(),
            );
            self.bond_level = new_level;
            self.bond_level_changed = level_changed;

            let bond_state = BondTracker::get_state(self.db.conn());
            self.bond_score = bond_state.bond_score;

            // Tick personality evolution
            Evolution::tick(
                self.db.conn(),
                new_level,
                self.config.evolution.formality_alpha,
            );

            // Check if narrative needs updating
            let needs_narrative = Narrative::tick_interaction(
                self.db.conn(),
                self.config.narrative.update_interval_interactions,
            );
            if needs_narrative {
                let self_texts: Vec<String> = self_memories
                    .iter()
                    .map(|m| m.text.clone())
                    .collect();
                Narrative::update(
                    self.db.conn(),
                    &*self.llm,
                    &self.config.user_name,
                    new_level,
                    bond_state.bond_score,
                    &self_texts,
                    self.config.narrative.max_tokens,
                );
            }
        }

        AgentResponse {
            message: response_text,
            memories_recalled: memories.len(),
            urges_delivered: urge_ids,
            tool_calls_made,
        }
    }

    /// Streaming version of handle_message — calls `on_token` for each text fragment.
    pub fn handle_message_streaming<F>(
        &mut self,
        user_text: &str,
        mut on_token: F,
    ) -> AgentResponse
    where
        F: FnMut(&str),
    {
        // Steps 1-6 are identical to handle_message
        self.check_session_timeout();

        let memories = self.db.recall_text(user_text, 5).unwrap_or_default();

        let self_memories = self
            .db
            .recall_text(&format!("self: {user_text}"), 10)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| {
                r.source == "self" || r.domain == "self-reflection"
            })
            .take(3)
            .collect::<Vec<_>>();

        let urges = self.urge_queue.pop_for_interaction(self.db.conn(), 2);
        let urge_ids: Vec<String> = urges.iter().map(|u| u.urge_id.clone()).collect();

        learning::detect_humor_reaction(self.db.conn(), user_text);

        let state = self.build_state();
        for instinct in &self.instincts {
            let specs = instinct.on_interaction(&state, user_text);
            for spec in specs {
                self.urge_queue.push(self.db.conn(), &spec);
            }
        }

        let personality = self.db.get_personality().ok();
        let patterns_json: Vec<serde_json::Value> =
            self.active_patterns.iter().cloned().collect();

        let narrative_text = Narrative::get(self.db.conn());
        let style = Evolution::get_style(self.db.conn());
        let opinions = Evolution::get_opinions(self.db.conn(), 3);
        let shared_refs = Evolution::get_shared_references(self.db.conn(), 3);

        let signals = ContextSignals {
            self_memories: &self_memories,
            narrative: &narrative_text,
            style: &style,
            opinions: &opinions,
            shared_refs: &shared_refs,
        };

        let mut messages = context::build_messages(
            user_text,
            &self.config,
            &state,
            &memories,
            &urges,
            &patterns_json,
            &self.conversation_history,
            personality.as_ref(),
            Some(&signals),
        );

        if self.config.tools.enabled {
            let tool_defs = tools::companion_tool_defs();
            let tool_text = format_tools(&tool_defs);
            if let Some(sys_msg) = messages.first_mut() {
                sys_msg.content.push_str(&tool_text);
            }
        }

        // Step 7: Call LLM with streaming
        let gen_config = GenerationConfig {
            max_tokens: self.config.llm.max_tokens,
            temperature: self.config.llm.temperature,
            top_p: Some(0.9),
            ..Default::default()
        };

        let mut tool_calls_made = Vec::new();
        let mut response_text = String::new();

        let llm_response = self.llm.chat_streaming(&messages, &gen_config, &mut |token| {
            on_token(token);
        });

        match llm_response {
            Ok(r) => {
                let tool_calls = parse_tool_calls(&r.text);
                if tool_calls.is_empty() {
                    response_text = r.text;
                } else {
                    // Tool calls found — execute them non-streaming
                    let text_part = extract_text_content(&r.text);
                    messages.push(ChatMessage::assistant(&r.text));

                    for call in &tool_calls {
                        tool_calls_made.push(call.name.clone());
                        let result =
                            tools::execute_tool(&self.db, &call.name, &call.arguments);
                        messages.push(ChatMessage::user(format!(
                            "Tool result for {}: {result}",
                            call.name
                        )));
                    }

                    // Follow-up generation (non-streaming — tool results are fast)
                    match self.llm.chat(&messages, &gen_config) {
                        Ok(r2) => {
                            on_token(&r2.text);
                            response_text = r2.text;
                        }
                        Err(_) if !text_part.is_empty() => {
                            response_text = text_part;
                        }
                        Err(e) => {
                            response_text =
                                format!("I'm having trouble thinking right now. ({e})");
                        }
                    }
                }
            }
            Err(e) => {
                response_text = format!("I'm having trouble thinking right now. ({e})");
            }
        }

        if response_text.is_empty() {
            response_text = "I'm here. How can I help?".to_string();
        }
        response_text = extract_text_content(&response_text);

        // Update conversation history
        self.conversation_history.push(ChatMessage::user(user_text));
        self.conversation_history
            .push(ChatMessage::assistant(&response_text));

        let max = self.config.conversation.max_history_turns * 2;
        if self.conversation_history.len() > max {
            let drain = self.conversation_history.len() - max;
            self.conversation_history.drain(..drain);
        }

        self.last_interaction_ts = now_ts();
        self.session_turn_count += 1;

        // Step 8: Learn + self-reflect
        learning::extract_and_learn(&self.db, &*self.llm, user_text, &response_text);

        // Step 9: Score bond + tick evolution
        if self.config.bond.enabled {
            let (new_level, level_changed) = BondTracker::score_interaction(
                self.db.conn(),
                user_text,
                &response_text,
                memories.len(),
            );
            self.bond_level = new_level;
            self.bond_level_changed = level_changed;

            let bond_state = BondTracker::get_state(self.db.conn());
            self.bond_score = bond_state.bond_score;

            Evolution::tick(
                self.db.conn(),
                new_level,
                self.config.evolution.formality_alpha,
            );

            let needs_narrative = Narrative::tick_interaction(
                self.db.conn(),
                self.config.narrative.update_interval_interactions,
            );
            if needs_narrative {
                let self_texts: Vec<String> = self_memories
                    .iter()
                    .map(|m| m.text.clone())
                    .collect();
                Narrative::update(
                    self.db.conn(),
                    &*self.llm,
                    &self.config.user_name,
                    new_level,
                    bond_state.bond_score,
                    &self_texts,
                    self.config.narrative.max_tokens,
                );
            }
        }

        AgentResponse {
            message: response_text,
            memories_recalled: memories.len(),
            urges_delivered: urge_ids,
            tool_calls_made,
        }
    }

    /// Build a state snapshot for instinct evaluation.
    pub fn build_state(&self) -> CompanionState {
        let memory_count = self
            .db
            .stats(None)
            .map(|s| s.active_memories)
            .unwrap_or(0);

        CompanionState {
            last_interaction_ts: self.last_interaction_ts,
            current_ts: now_ts(),
            session_active: self.session_turn_count > 0,
            conversation_turn_count: self.session_turn_count,
            recent_valence_avg: self.recent_valence_avg,
            pending_triggers: self.pending_triggers.clone(),
            active_patterns: self.active_patterns.clone(),
            open_conflicts_count: self.open_conflicts_count,
            memory_count,
            config_user_name: self.config.user_name.clone(),
            // Soul state
            bond_level: self.bond_level,
            bond_score: self.bond_score,
            formality: Evolution::get_style(self.db.conn()).formality,
            opinions_count: Evolution::count_opinions(self.db.conn()),
            shared_references_count: Evolution::count_shared_references(self.db.conn()),
            bond_level_changed: self.bond_level_changed,
        }
    }

    /// Evaluate all instincts against current state. Used by background cognition.
    pub fn evaluate_instincts(&self, state: &CompanionState) -> Vec<crate::types::UrgeSpec> {
        let mut all_urges = Vec::new();
        for instinct in &self.instincts {
            all_urges.extend(instinct.evaluate(state));
        }
        all_urges
    }

    /// Take the pending proactive message (if any).
    pub fn take_proactive_message(&mut self) -> Option<ProactiveMessage> {
        self.proactive_message.take()
    }

    /// Set a proactive message (called by background cognition).
    pub fn set_proactive_message(&mut self, msg: ProactiveMessage) {
        self.proactive_message = Some(msg);
    }

    /// Update cached cognition state (called after think()).
    pub fn update_cognition_cache(
        &mut self,
        triggers: Vec<serde_json::Value>,
        patterns: Vec<serde_json::Value>,
        conflicts_count: usize,
        valence_avg: Option<f64>,
    ) {
        self.pending_triggers = triggers;
        self.active_patterns = patterns;
        self.open_conflicts_count = conflicts_count;
        self.recent_valence_avg = valence_avg;
    }

    /// Get conversation history.
    pub fn history(&self) -> &[ChatMessage] {
        &self.conversation_history
    }

    /// Seconds since last interaction.
    pub fn idle_seconds(&self) -> f64 {
        now_ts() - self.last_interaction_ts
    }

    /// Get current bond level.
    pub fn bond_level(&self) -> BondLevel {
        self.bond_level
    }

    /// Get current bond score.
    pub fn bond_score(&self) -> f64 {
        self.bond_score
    }

    fn check_session_timeout(&mut self) {
        let idle = self.idle_seconds();
        let timeout = self.config.conversation.session_timeout_minutes as f64 * 60.0;

        if idle > timeout && self.session_turn_count > 0 {
            tracing::info!(
                idle_minutes = idle / 60.0,
                turns = self.session_turn_count,
                "Session timeout — resetting history"
            );
            self.conversation_history.clear();
            self.session_turn_count = 0;
        }
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
