//! Wire protocol types for the native mobile WebSocket provider.
//!
//! All frames are JSON text messages. Binary frames reserved for future
//! raw audio optimization (V2).

use serde::{Deserialize, Serialize};

// ── Client → Server ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Must be the first message after WS connect.
    #[serde(rename = "auth")]
    Auth {
        token: String,
        client_id: String,
        #[serde(default)]
        client_name: Option<String>,
    },

    /// Text chat message.
    #[serde(rename = "text")]
    Text {
        id: String,
        text: String,
    },

    /// Begin a streaming voice turn.
    #[serde(rename = "voice_start")]
    VoiceStart {
        id: String,
        #[serde(default = "default_sample_rate")]
        sample_rate: u32,
        #[serde(default = "default_codec")]
        codec: String,
    },

    /// One opus audio chunk (~20ms). `data` is base64-encoded.
    #[serde(rename = "voice_chunk")]
    VoiceChunk {
        id: String,
        seq: u32,
        data: String,
    },

    /// Client-side VAD says speech ended.
    #[serde(rename = "voice_end")]
    VoiceEnd {
        id: String,
    },

    /// Typing indicator.
    #[serde(rename = "typing")]
    Typing,

    /// Keepalive ping.
    #[serde(rename = "ping")]
    Ping {
        #[serde(default)]
        ts: Option<i64>,
    },
}

fn default_sample_rate() -> u32 { 16000 }
fn default_codec() -> String { "opus".into() }

// ── Server → Client ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// Authentication succeeded.
    #[serde(rename = "auth_ok")]
    AuthOk {
        session_id: String,
        companion_name: String,
        bond_level: String,
    },

    /// Authentication failed.
    #[serde(rename = "auth_fail")]
    AuthFail {
        reason: String,
    },

    /// Complete text response.
    #[serde(rename = "text")]
    Text {
        id: String,
        text: String,
        #[serde(rename = "final")]
        is_final: bool,
    },

    /// Streaming text chunk (incremental token).
    #[serde(rename = "text_chunk")]
    TextChunk {
        id: String,
        delta: String,
    },

    /// Begin TTS voice response.
    #[serde(rename = "voice_start")]
    VoiceStart {
        id: String,
        sample_rate: u32,
        codec: String,
    },

    /// One TTS audio chunk. `data` is base64-encoded.
    #[serde(rename = "voice_chunk")]
    VoiceChunk {
        id: String,
        seq: u32,
        data: String,
    },

    /// TTS voice response complete.
    #[serde(rename = "voice_end")]
    VoiceEnd {
        id: String,
    },

    /// What the STT heard from the client's voice.
    #[serde(rename = "transcription")]
    Transcription {
        voice_id: String,
        text: String,
    },

    /// Companion is typing a response.
    #[serde(rename = "typing")]
    Typing,

    /// Companion is thinking (processing with tools, etc.).
    #[serde(rename = "thinking")]
    Thinking,

    /// Server detected speech start via VAD.
    #[serde(rename = "listening")]
    Listening,

    /// Keepalive pong.
    #[serde(rename = "pong")]
    Pong {
        ts: i64,
    },

    /// Session terminated.
    #[serde(rename = "session_expired")]
    SessionExpired {
        reason: String,
    },
}

impl ServerMessage {
    /// Serialize to JSON string for WS text frame.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}
