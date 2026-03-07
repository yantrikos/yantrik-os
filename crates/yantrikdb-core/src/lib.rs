// ── Directory modules ──
mod base;
pub mod engine;
mod cognition;
mod distributed;
mod knowledge;
mod vector;

// ── Re-exports at original crate paths ──
pub use base::{bench_utils, compression, encryption, error, hlc, schema, scoring, serde_helpers, types, vault};
pub use cognition::{consolidate, patterns, personality, triggers};
pub use distributed::{conflict, replication, sync};
pub use knowledge::{graph, graph_index};
pub use vector::hnsw;

// ── Convenience re-exports ──
pub use engine::YantrikDB;
pub use engine::tenant::{TenantManager, TenantConfig};
pub use error::YantrikDbError;
pub use types::*;
pub use consolidate::{consolidate, find_consolidation_candidates};
pub use triggers::{check_decay_triggers, check_consolidation_triggers, check_all_triggers};
pub use conflict::{scan_conflicts, detect_edge_conflicts, create_conflict};
pub use patterns::mine_patterns;
pub use personality::{derive_personality, get_personality, set_personality_trait};
