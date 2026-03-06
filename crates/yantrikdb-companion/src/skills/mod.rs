//! Skill Store — manifest-driven skill registry for Yantrik.
//!
//! Skills are the unit of optional functionality. Each skill declares:
//! - Tools it provides (registered in the tool registry)
//! - Instincts it activates (proactive behaviors)
//! - Cortex rules it enables (cross-system intelligence)
//! - Services it represents (for service-gated logic)
//!
//! Core functionality (memory, files, browser, shell) is always on and
//! not exposed in the Skill Store UI.

pub mod bridge;
pub mod manifest;
pub mod registry;

pub use bridge::{load_skill_snapshot, load_skill_snapshot_with_services, SkillSnapshot};
pub use manifest::{SkillCategory, SkillManifest, SkillPermission};
pub use registry::{SkillEntry, SkillRegistry};
