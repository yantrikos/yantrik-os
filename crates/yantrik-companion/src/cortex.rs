//! Thin re-export of `yantrik-companion-cortex`.
//!
//! The cortex implementation lives in its own crate for faster incremental builds.
//! This module preserves the `crate::cortex::*` import paths for existing code.

pub use yantrik_companion_cortex::*;

// Re-export submodules so `crate::cortex::playbook::X` paths still work.
pub use yantrik_companion_cortex::baselines;
pub use yantrik_companion_cortex::entity;
pub use yantrik_companion_cortex::focus;
pub use yantrik_companion_cortex::patterns;
pub use yantrik_companion_cortex::playbook;
pub use yantrik_companion_cortex::pulse;
pub use yantrik_companion_cortex::reasoner;
pub use yantrik_companion_cortex::rules;
pub use yantrik_companion_cortex::schema;
pub use yantrik_companion_cortex::situation;
