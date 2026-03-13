//! Provider Registry — unified multi-provider LLM backend management.
//!
//! Manages multiple LLM providers (Ollama, OpenAI, Anthropic, Gemini, etc.)
//! with health monitoring, task routing, automatic failover, and secure
//! credential storage.

mod registry;
mod descriptor;
mod generic_openai;
mod anthropic;
mod gemini;
mod health;
mod routing;
pub mod secret_store;
pub mod key_validation;

pub use registry::{ProviderRegistry, ProviderEntry as RegisteredProvider, ProviderId, UsageStats};
pub use descriptor::{ProviderDescriptor, ProviderKind, AuthScheme, SetupTier, KNOWN_PROVIDERS};
pub use generic_openai::GenericOpenAIBackend;
pub use anthropic::AnthropicBackend;
pub use gemini::GoogleGeminiBackend;
pub use health::{ProviderHealth, HealthStatus};
pub use routing::TaskType;
pub use secret_store::{SecretStore, AutoSecretStore, KeyringSecretStore, EncryptedFileStore, SecretRef};
pub use key_validation::{KeyValidator, KeyValidationResult, KeyValidationError};
