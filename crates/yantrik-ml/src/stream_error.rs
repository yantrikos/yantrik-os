//! An error a model server sends inside a streamed reply, after it has answered 200.
//!
//! A streaming server has already sent its status line when something goes wrong mid-reply, so it
//! reports the failure as one more event: `data: {"error": {"type": "server_error", "message":
//! "…"}}`, then `data: [DONE]`. The stream readers skipped any event without `choices`, so the
//! reply ended with no words and no call, and the companion said "I'm here. How can I help?" to
//! "Use the calculate tool to work out 17 times 23" (VM 542, 15 Oct 2026: the server rejected the
//! model's own tool call as "malformed tool call: {…}"). The event is now the reply's failure, in
//! the server's words, the way a non-streamed error status already was.

/// The person-facing words for a failure the server named.
pub fn said(e: &StreamError) -> String {
    format!(
        "The model's server reported an error: {}. Try again or choose another model.",
        e.message.trim_end_matches('.')
    )
}

/// What the server said went wrong, from its `error` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    pub message: String,
}

impl StreamError {
    /// The event's `error`: an object with a `message` (OpenAI's shape and llama.cpp's), or a
    /// bare string. Clipped, since a server may quote a whole model output back.
    pub fn of(error: &serde_json::Value) -> StreamError {
        let message = error["message"]
            .as_str()
            .or_else(|| error.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        StreamError { message: message.chars().take(300).collect() }
    }
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the model's server reported an error mid-reply: {}", self.message)
    }
}

impl std::error::Error for StreamError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_servers_words_are_kept_from_either_shape() {
        let object = serde_json::json!({"type": "server_error", "message": "malformed tool call: {\"name\": \"calculate\"}"});
        assert_eq!(StreamError::of(&object).message, "malformed tool call: {\"name\": \"calculate\"}");
        assert_eq!(StreamError::of(&serde_json::json!("model not loaded")).message, "model not loaded");
        let long = serde_json::json!({"message": "x".repeat(1000)});
        assert_eq!(StreamError::of(&long).message.chars().count(), 300);
    }

    #[test]
    fn the_person_is_told_what_the_server_said_and_what_to_do() {
        let e = StreamError { message: "malformed tool call.".into() };
        assert_eq!(said(&e), "The model's server reported an error: malformed tool call. Try again or choose another model.");
    }
}
