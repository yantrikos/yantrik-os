//! Canonical event model — shared across all providers.
//!
//! Normalized just enough for the router/brain to work, with `raw` escape
//! hatches for platform-specific metadata.

use serde::{Deserialize, Serialize};

// ── Conversations ───────────────────────────────────────────────────

/// How a conversation is structured on the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversationKind {
    Direct,
    Group,
    Channel,
    Thread,
    Room,
}

/// A reference to a specific conversation on a specific provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRef {
    pub provider: String,
    pub kind: ConversationKind,
    pub id: String,
    /// Parent conversation (e.g. channel for a thread).
    pub parent_id: Option<String>,
    /// Human-readable title if available.
    pub title: Option<String>,
}

impl ConversationRef {
    pub fn direct(provider: &str, id: &str) -> Self {
        Self {
            provider: provider.to_string(),
            kind: ConversationKind::Direct,
            id: id.to_string(),
            parent_id: None,
            title: None,
        }
    }

    pub fn group(provider: &str, id: &str, title: Option<&str>) -> Self {
        Self {
            provider: provider.to_string(),
            kind: ConversationKind::Group,
            id: id.to_string(),
            parent_id: None,
            title: title.map(|s| s.to_string()),
        }
    }

    pub fn thread(provider: &str, id: &str, parent_id: &str) -> Self {
        Self {
            provider: provider.to_string(),
            kind: ConversationKind::Thread,
            id: id.to_string(),
            parent_id: Some(parent_id.to_string()),
            title: None,
        }
    }
}

// ── Messages ────────────────────────────────────────────────────────

/// Unique reference to a message on a platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRef {
    pub provider: String,
    pub id: String,
}

/// Who sent a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorRef {
    pub id: String,
    pub display_name: String,
    pub is_bot: bool,
}

/// Reference to downloadable media.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    pub provider: String,
    pub id: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub filename: Option<String>,
}

// ── Message content ─────────────────────────────────────────────────

/// Content of an inbound or outbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageContent {
    Text {
        text: String,
    },
    Voice {
        media: MediaRef,
        duration_ms: Option<u64>,
    },
    Image {
        media: MediaRef,
        caption: Option<String>,
    },
    Video {
        media: MediaRef,
        caption: Option<String>,
    },
    File {
        media: MediaRef,
        caption: Option<String>,
    },
    Location {
        lat: f64,
        lon: f64,
        label: Option<String>,
    },
    Sticker {
        media: MediaRef,
    },
    /// Platform-specific content we don't model.
    Unsupported {
        kind: String,
    },
}

impl MessageContent {
    /// Extract text content (including captions) for AI processing.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::Image { caption, .. }
            | Self::Video { caption, .. }
            | Self::File { caption, .. } => caption.as_deref(),
            _ => None,
        }
    }

    /// Short description for logging.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Voice { .. } => "voice",
            Self::Image { .. } => "image",
            Self::Video { .. } => "video",
            Self::File { .. } => "file",
            Self::Location { .. } => "location",
            Self::Sticker { .. } => "sticker",
            Self::Unsupported { .. } => "unsupported",
        }
    }
}

// ── Inbound events ──────────────────────────────────────────────────

/// Event received from a chat provider.
#[derive(Debug, Clone)]
pub enum InboundEvent {
    Message(InboundMessage),
    MessageEdited(MessageEditEvent),
    MessageDeleted(MessageDeleteEvent),
    Reaction(ReactionEvent),
    Typing(TypingEvent),
}

impl InboundEvent {
    /// Dedupe key: provider + event_id.
    pub fn event_id(&self) -> Option<&str> {
        match self {
            Self::Message(m) => Some(&m.event_id),
            Self::MessageEdited(e) => Some(&e.event_id),
            Self::MessageDeleted(d) => Some(&d.event_id),
            Self::Reaction(r) => Some(&r.event_id),
            Self::Typing(_) => None, // Don't dedupe typing events
        }
    }

    pub fn provider(&self) -> &str {
        match self {
            Self::Message(m) => &m.conversation.provider,
            Self::MessageEdited(e) => &e.conversation.provider,
            Self::MessageDeleted(d) => &d.conversation.provider,
            Self::Reaction(r) => &r.conversation.provider,
            Self::Typing(t) => &t.conversation.provider,
        }
    }
}

/// An inbound message from a chat provider.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Provider-unique dedupe key (e.g. update_id for Telegram).
    pub event_id: String,
    pub conversation: ConversationRef,
    pub message: MessageRef,
    pub sender: ActorRef,
    pub timestamp_ms: i64,
    pub content: MessageContent,
    pub reply_to: Option<MessageRef>,
    /// Whether the AI bot was explicitly mentioned.
    pub mentions_ai: bool,
    /// Platform-specific raw payload for escape hatch.
    pub raw: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MessageEditEvent {
    pub event_id: String,
    pub conversation: ConversationRef,
    pub message: MessageRef,
    pub new_content: MessageContent,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub struct MessageDeleteEvent {
    pub event_id: String,
    pub conversation: ConversationRef,
    pub message: MessageRef,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ReactionEvent {
    pub event_id: String,
    pub conversation: ConversationRef,
    pub message: MessageRef,
    pub actor: ActorRef,
    pub emoji: String,
    pub added: bool, // true = added, false = removed
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub struct TypingEvent {
    pub conversation: ConversationRef,
    pub actor: ActorRef,
}

// ── Outbound messages ───────────────────────────────────────────────

/// Message to send through a provider.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub content: OutboundContent,
    pub reply_to: Option<MessageRef>,
    /// Provider-specific metadata (e.g. thread_ts for Slack).
    pub meta: serde_json::Value,
}

impl OutboundMessage {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: OutboundContent::Text(text.into()),
            reply_to: None,
            meta: serde_json::Value::Null,
        }
    }

    pub fn with_reply(mut self, msg_ref: MessageRef) -> Self {
        self.reply_to = Some(msg_ref);
        self
    }
}

#[derive(Debug, Clone)]
pub enum OutboundContent {
    Text(String),
    Voice { path: String },
    Image { path: String, caption: Option<String> },
    File { path: String, caption: Option<String> },
    Reaction { target: MessageRef, emoji: String },
}

// ── Provider health ─────────────────────────────────────────────────

/// Health status of a provider connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderHealth {
    Connected,
    Connecting,
    Disconnected,
    Error,
}

/// Info about a conversation (returned by conversation_info).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationInfo {
    pub conversation: ConversationRef,
    pub participant_count: Option<u32>,
    pub last_activity_ms: Option<i64>,
}

/// Result of sending a message.
#[derive(Debug, Clone)]
pub struct SendReceipt {
    pub message: MessageRef,
    pub timestamp_ms: i64,
}
