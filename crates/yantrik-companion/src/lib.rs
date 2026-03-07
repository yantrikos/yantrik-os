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
pub mod bond;
pub mod companion;
pub mod config;
pub mod context;
pub mod cron_mini;
pub mod evolution;
pub mod instincts;
pub mod learning;
pub mod memory_evolution;
pub mod narrative;
pub mod offline;
pub mod proactive;
pub mod proactive_templates;
pub mod sanitize;
pub mod scheduler;
pub mod resonance;
pub mod synthesis_gate;
pub mod security;
pub mod telegram;
pub mod task_manager;
pub mod recipe;
pub mod task_queue;
pub mod tool_cache;
pub mod tool_traces;
pub mod email;
pub mod calendar;
pub mod life_assistant;
pub mod cortex;
pub mod connectors;
pub mod skills;
pub mod tools;
pub mod types;
pub mod urges;
pub mod user_model;
pub mod voice;

pub use companion::CompanionService;
pub use config::CompanionConfig;
pub use types::{AgentResponse, CompanionState, ProactiveMessage, Urge, UrgeSpec};
pub use voice::{VoiceProfile, VoiceTurnResult};
