//! Provider Registry — unified multi-provider LLM backend management.
//!
//! Manages multiple LLM providers (Ollama, OpenAI, Anthropic, Gemini, etc.)
//! with health monitoring, task routing, and automatic failover.

mod registry;
mod descriptor;
mod catalogue;
mod generic_openai;
mod anthropic;
mod gemini;
mod health;
mod routing;
pub mod key_validation;
pub mod pool;

pub use registry::{ProviderRegistry, ProviderEntry as RegisteredProvider, ProviderId, UsageStats};
pub use descriptor::{ProviderDescriptor, ProviderKind, AuthScheme, SetupTier, KNOWN_PROVIDERS};
pub use generic_openai::{GenericOpenAIBackend, MetaSink, ResponseMeta};
pub use anthropic::AnthropicBackend;
pub use gemini::GoogleGeminiBackend;
pub use health::{ProviderHealth, HealthStatus};
pub use routing::TaskType;
pub use key_validation::{KeyValidator, KeyValidationResult, KeyValidationError};
