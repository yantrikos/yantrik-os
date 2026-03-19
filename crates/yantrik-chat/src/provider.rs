//! ChatProvider trait — implemented by each platform adapter.
//!
//! Single trait with capability flags and default `Unsupported` impls.
//! Each provider runs in its own thread using blocking I/O.

use crate::model::*;

// ── Errors ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ChatError {
    /// This operation isn't supported by the provider.
    Unsupported,
    /// Authentication failure (bad token, expired, etc.).
    Auth(String),
    /// Network / transport error.
    Network(String),
    /// Rate limited — retry after this many seconds.
    RateLimited(u64),
    /// Provider-specific error.
    Provider(String),
    /// Internal error.
    Internal(String),
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => write!(f, "operation not supported by this provider"),
            Self::Auth(msg) => write!(f, "auth error: {msg}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::RateLimited(secs) => write!(f, "rate limited, retry after {secs}s"),
            Self::Provider(msg) => write!(f, "provider error: {msg}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for ChatError {}

// ── Capabilities ────────────────────────────────────────────────────

/// How a provider receives inbound messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressMode {
    /// Long-poll or periodic poll (Telegram, Matrix).
    Polling,
    /// Persistent WebSocket connection (Discord, Slack Socket Mode).
    WebSocket,
    /// HTTP webhook callbacks (WhatsApp Cloud API).
    Webhook,
}

/// What a provider supports — checked before calling optional methods.
#[derive(Debug, Clone)]
pub struct ProviderCapabilities {
    pub ingress: IngressMode,
    pub media: bool,
    pub voice: bool,
    pub reactions: bool,
    pub typing: bool,
    pub threads: bool,
    pub groups: bool,
    pub channels: bool,
    pub read_receipts: bool,
    pub edits: bool,
    pub deletes: bool,
}

impl ProviderCapabilities {
    /// Minimal capabilities — text only, polling.
    pub fn minimal() -> Self {
        Self {
            ingress: IngressMode::Polling,
            media: false,
            voice: false,
            reactions: false,
            typing: false,
            threads: false,
            groups: false,
            channels: false,
            read_receipts: false,
            edits: false,
            deletes: false,
        }
    }
}

// ── The trait ────────────────────────────────────────────────────────

/// A chat platform adapter. Implemented by each provider (Telegram, Discord, etc.).
///
/// Providers are transport-only — no AI logic, no memory, no tools.
/// The router handles all business logic after receiving events.
pub trait ChatProvider: Send {
    /// Unique provider identifier (e.g. "telegram", "discord").
    fn id(&self) -> &'static str;

    /// What this provider supports.
    fn capabilities(&self) -> ProviderCapabilities;

    // ── Lifecycle ───────────────────────────────────────────────────

    /// Open connection / authenticate. Called once when manager starts.
    fn connect(&mut self) -> Result<(), ChatError>;

    /// Cleanly close connection.
    fn disconnect(&mut self) -> Result<(), ChatError>;

    /// Current connection health.
    fn health(&self) -> ProviderHealth;

    // ── Inbound ─────────────────────────────────────────────────────

    /// Poll for new events (blocking, may use long-poll timeout).
    /// For Polling and WebSocket providers.
    /// Returns empty vec if no new events.
    fn poll(&mut self) -> Result<Vec<InboundEvent>, ChatError> {
        Err(ChatError::Unsupported)
    }

    /// Handle an incoming webhook payload.
    /// Returns (HTTP status, response body, parsed events).
    fn handle_webhook(
        &mut self,
        body: &[u8],
        headers: &[(String, String)],
    ) -> Result<(u16, Vec<u8>, Vec<InboundEvent>), ChatError> {
        let _ = (body, headers);
        Err(ChatError::Unsupported)
    }

    // ── Outbound ────────────────────────────────────────────────────

    /// Send a message to a conversation.
    fn send(
        &mut self,
        target: &ConversationRef,
        msg: &OutboundMessage,
    ) -> Result<SendReceipt, ChatError>;

    /// Show typing indicator.
    fn send_typing(&mut self, _target: &ConversationRef) -> Result<(), ChatError> {
        Err(ChatError::Unsupported)
    }

    /// Send a reaction emoji on a message.
    fn send_reaction(
        &mut self,
        _target: &MessageRef,
        _emoji: &str,
    ) -> Result<(), ChatError> {
        Err(ChatError::Unsupported)
    }

    /// Mark messages as read up to a given point.
    fn mark_read(
        &mut self,
        _target: &ConversationRef,
        _up_to: Option<&MessageRef>,
    ) -> Result<(), ChatError> {
        Err(ChatError::Unsupported)
    }

    /// Download media by reference.
    fn download_media(&mut self, _media: &MediaRef) -> Result<Vec<u8>, ChatError> {
        Err(ChatError::Unsupported)
    }

    /// Get info about a conversation (participant count, title, etc.).
    fn conversation_info(&self, _id: &str) -> Result<ConversationInfo, ChatError> {
        Err(ChatError::Unsupported)
    }
}
