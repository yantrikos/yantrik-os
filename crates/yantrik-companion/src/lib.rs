//! YantrikDB Companion — the agent brain.
//!
//! This crate provides the full companion pipeline:
//! - Memory recall + recording (via YantrikDB)
//! - LLM inference (via candle, in-process)
//! - Instinct-driven urge system (9 instincts including bond/self-awareness/humor)
//! - Bond tracking and personality evolution
//! - Self-narrative and self-reflection memories
//! - Context assembly with bond-aware personality
//! - Tool execution (remember, recall, relate, set_reminder, introspect, form_opinion, etc.)
//! - Post-interaction learning + self-reflection
//! - Background cognition loop with narrative updates
//! - Voice orchestration: STT → companion → TTS with bond-adaptive voice
//!
//! Zero HTTP servers. Everything runs in a single process.

pub mod agent_loop;
pub mod audio_convert;
pub mod automation;
pub mod background;
pub mod brain_loop;
pub mod chat_bridge;
pub mod bond;
pub mod companion;
pub mod config;
pub mod mcp_client;
pub mod mcp_security;
pub mod sub_agent;
pub mod context;
pub mod cron_mini;
pub mod event_driven;
pub mod evolution;
pub mod instincts;
pub mod learning;
pub mod memory_evolution;
pub mod memory_lifecycle;
pub mod memory_repair;
pub mod narrative;
pub mod offline;
pub mod offline_nlp;
pub mod policy_engine;
pub mod proactive;
pub mod proactive_pipeline;
pub mod proactive_templates;
pub mod sanitize;
pub mod silence_policy;
pub mod scheduler;
pub mod resonance;
pub mod synthesis_gate;
pub mod security;
pub mod telegram;
pub mod task_manager;
pub mod query_planner;
pub mod interjection;
pub mod recipe;
pub mod recipe_executor;
pub mod recipe_templates;
pub mod task_queue;
pub mod trust_model;
pub mod tool_cache;
pub mod tool_metrics;
pub mod tool_traces;
pub mod types;
pub mod email;
pub mod calendar;
pub mod life_assistant;
pub mod cortex;
pub mod connectors;
pub mod skills;
pub mod tools;
pub mod urge_selector;
pub mod urges;
pub mod user_model;
pub mod voice;
pub mod whatsapp;
pub mod cognitive_router;
pub mod commitment_extractor;
pub mod workspaces;
pub mod anticipation;
pub mod relationship_intelligence;
pub mod companion_modes;
pub mod world_model;
pub mod open_loops_monitor;
pub mod world_graph;
pub mod graph_bridge;
pub mod nudge_templates;
pub mod active_context;
pub mod ck5_integration;
pub mod hallucination_firewall;
pub mod stewardship;
pub mod structured_output;

// Top-level convenience re-exports (matching previous API)
pub use companion::CompanionService;
pub use config::CompanionConfig;
pub use types::{
    AgentResponse, CompanionState, InstinctCategory, ProactiveMessage,
    TimeSensitivity, Urge, UrgeSpec,
};
pub use voice::{VoiceProfile, VoiceTurnResult};
