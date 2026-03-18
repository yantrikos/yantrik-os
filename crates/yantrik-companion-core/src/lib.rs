//! Shared types, config, and traits for the Yantrik companion agent.
//!
//! This crate contains the foundational types that all companion sub-modules
//! depend on: data structs, enums, configuration, and trait definitions.
//! It has zero database or LLM runtime dependencies.

pub mod bond;
pub mod config;
pub mod connectors;
pub mod evolution;
pub mod instincts;
pub mod permission;
pub mod sanitize;
pub mod tools;
pub mod types;
pub mod urge_defaults;

// Convenience re-exports
pub use bond::{BondLevel, BondState};
pub use config::CompanionConfig;
pub use connectors::{Connector, SeedEntity};
pub use instincts::Instinct;
pub use permission::{PermissionLevel, parse_permission};
pub use types::{
    AgentResponse, AutonomyTier, CompanionState, InstinctCategory, ProactiveMessage,
    TimeSensitivity, Urge, UrgeSpec,
};
