//! Per-conversation policy engine — controls when the AI replies.
//!
//! Each conversation can have a different policy: auto-reply, mention-only,
//! monitor-only (feeds brain but no reply), or muted.

use serde::{Deserialize, Serialize};
use crate::model::{ConversationKind, InboundMessage};

/// How the AI should handle messages in this conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplyMode {
    /// Respond to all messages.
    AutoReply,
    /// Only respond when @mentioned or in a DM.
    MentionOnly,
    /// Feed signals to brain but never reply. Silent intelligence.
    MonitorOnly,
    /// Ignore completely — don't even feed to brain.
    Muted,
}

impl Default for ReplyMode {
    fn default() -> Self {
        Self::MentionOnly
    }
}

/// Policy for a specific conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPolicy {
    pub mode: ReplyMode,
    /// Quiet hours: don't auto-reply between these hours (24h format, local time).
    pub quiet_start_hour: Option<u8>,
    pub quiet_end_hour: Option<u8>,
    /// Whether the AI can use tools when responding in this conversation.
    pub allow_tool_use: bool,
    /// Whether sensitive tools (file write, shell, etc.) are allowed.
    pub trusted: bool,
    /// Maximum response length in characters (None = unlimited).
    pub max_reply_length: Option<usize>,
    /// Optional persona override for this conversation.
    pub personality_override: Option<String>,
}

impl Default for ConversationPolicy {
    fn default() -> Self {
        Self {
            mode: ReplyMode::MentionOnly,
            quiet_start_hour: None,
            quiet_end_hour: None,
            allow_tool_use: true,
            trusted: false,
            max_reply_length: None,
            personality_override: None,
        }
    }
}

impl ConversationPolicy {
    /// Full auto-reply with tool access (for trusted DMs).
    pub fn trusted_dm() -> Self {
        Self {
            mode: ReplyMode::AutoReply,
            allow_tool_use: true,
            trusted: true,
            ..Default::default()
        }
    }

    /// Monitor-only: feed brain, never reply.
    pub fn monitor() -> Self {
        Self {
            mode: ReplyMode::MonitorOnly,
            allow_tool_use: false,
            trusted: false,
            ..Default::default()
        }
    }
}

/// Evaluate whether the AI should reply to this message given the policy.
pub fn should_ai_reply(
    msg: &InboundMessage,
    policy: &ConversationPolicy,
    current_hour: u8,
) -> bool {
    match policy.mode {
        ReplyMode::Muted | ReplyMode::MonitorOnly => return false,
        ReplyMode::AutoReply => {}
        ReplyMode::MentionOnly => {
            // Auto-reply in DMs, require mention in groups/channels
            let is_dm = msg.conversation.kind == ConversationKind::Direct;
            if !is_dm && !msg.mentions_ai {
                return false;
            }
        }
    }

    // Check quiet hours
    if let (Some(start), Some(end)) = (policy.quiet_start_hour, policy.quiet_end_hour) {
        if start <= end {
            // Simple range: e.g. 23-07 doesn't wrap
            if current_hour >= start && current_hour < end {
                return false;
            }
        } else {
            // Wrapping range: e.g. 23-07 means 23,0,1,2,3,4,5,6
            if current_hour >= start || current_hour < end {
                return false;
            }
        }
    }

    true
}

/// Evaluate whether this message should feed the brain (even if we don't reply).
pub fn should_feed_brain(policy: &ConversationPolicy) -> bool {
    // Everything except Muted feeds the brain
    policy.mode != ReplyMode::Muted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn make_msg(kind: ConversationKind, mentions_ai: bool) -> InboundMessage {
        InboundMessage {
            event_id: "1".into(),
            conversation: ConversationRef {
                provider: "test".into(),
                kind,
                id: "conv1".into(),
                parent_id: None,
                title: None,
            },
            message: MessageRef { provider: "test".into(), id: "m1".into() },
            sender: ActorRef { id: "u1".into(), display_name: "Alice".into(), is_bot: false },
            timestamp_ms: 0,
            content: MessageContent::Text { text: "hello".into() },
            reply_to: None,
            mentions_ai,
            raw: None,
        }
    }

    #[test]
    fn auto_reply_always_replies() {
        let policy = ConversationPolicy { mode: ReplyMode::AutoReply, ..Default::default() };
        assert!(should_ai_reply(&make_msg(ConversationKind::Group, false), &policy, 12));
    }

    #[test]
    fn mention_only_requires_mention_in_group() {
        let policy = ConversationPolicy::default(); // MentionOnly
        assert!(!should_ai_reply(&make_msg(ConversationKind::Group, false), &policy, 12));
        assert!(should_ai_reply(&make_msg(ConversationKind::Group, true), &policy, 12));
    }

    #[test]
    fn mention_only_auto_replies_in_dm() {
        let policy = ConversationPolicy::default();
        assert!(should_ai_reply(&make_msg(ConversationKind::Direct, false), &policy, 12));
    }

    #[test]
    fn monitor_never_replies() {
        let policy = ConversationPolicy::monitor();
        assert!(!should_ai_reply(&make_msg(ConversationKind::Direct, true), &policy, 12));
        assert!(should_feed_brain(&policy));
    }

    #[test]
    fn muted_blocks_everything() {
        let policy = ConversationPolicy { mode: ReplyMode::Muted, ..Default::default() };
        assert!(!should_ai_reply(&make_msg(ConversationKind::Direct, true), &policy, 12));
        assert!(!should_feed_brain(&policy));
    }

    #[test]
    fn quiet_hours_wrapping() {
        let policy = ConversationPolicy {
            mode: ReplyMode::AutoReply,
            quiet_start_hour: Some(23),
            quiet_end_hour: Some(7),
            ..Default::default()
        };
        assert!(!should_ai_reply(&make_msg(ConversationKind::Direct, false), &policy, 23));
        assert!(!should_ai_reply(&make_msg(ConversationKind::Direct, false), &policy, 2));
        assert!(should_ai_reply(&make_msg(ConversationKind::Direct, false), &policy, 12));
    }
}
