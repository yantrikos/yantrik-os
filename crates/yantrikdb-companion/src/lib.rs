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

pub mod background;
pub mod bond;
pub mod companion;
pub mod config;
pub mod context;
pub mod evolution;
pub mod instincts;
pub mod learning;
pub mod narrative;
pub mod tools;
pub mod types;
pub mod urges;
pub mod voice;

pub use companion::CompanionService;
pub use config::CompanionConfig;
pub use types::{AgentResponse, CompanionState, ProactiveMessage, Urge, UrgeSpec};
pub use voice::{VoiceProfile, VoiceTurnResult};
