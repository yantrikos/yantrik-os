//! What each model can do and how hard it can be asked to think — the model half of the AI
//! accounts catalogue (Settings → AI & Intelligence, the picker, the local model gateway).
//!
//! - `effort`: the person's four words (easy, medium, high, xhigh) and each provider's knob.
//! - `caps`: a model's capabilities from its provider and family.
//! - `curated`: the models of providers with no model-list API.
//!
//! Not behind `api-llm`: the gateway (crates/yantrik-gateway) needs this and none of the backends.

pub mod caps;
pub mod curated;
pub mod effort;

pub use caps::{caps_for, family, reasoning_for, ModelCaps};
pub use curated::{curated, CuratedModel};
pub use effort::{params as effort_params, Effort, EffortParams, Reasoning};
