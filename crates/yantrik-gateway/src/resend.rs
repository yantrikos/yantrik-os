//! The provider saying the model's own tool call was malformed (Groq's `tool_use_failed`, which
//! gpt-oss brings on now and then by writing its channel markup into the tool's name): the same call
//! is sent once more. It comes as a 400, or as a 200 whose stream's first event is the error. The
//! stream's head is not written until its first data event is read, so nothing has gone to the
//! client when the call is sent again; once a data event has gone, it never is.

use std::io::{Cursor, Read};

use serde_json::Value;

use crate::log::Call;
use crate::policy::{Refusal, Target};
use crate::server::Gateway;
use crate::upstream::Reply;

/// The most of a 400 that is read to find its code.
const PEEK: u64 = 16 * 1024;
/// What the second try is logged with.
pub(crate) const MODEL_TOOL_CALL: &str = "model_tool_call";

/// Whether an upstream error body is the provider saying the model wrote a malformed tool call.
pub(crate) fn is_tool_use_failed(raw: &[u8]) -> bool {
    serde_json::from_slice::<Value>(raw).ok().is_some_and(|v| says_malformed(&v))
}

/// An `error` object that is `tool_use_failed`, or whose message starts as Groq's does.
fn says_malformed(v: &Value) -> bool {
    v["error"]["code"] == "tool_use_failed" || v["error"]["message"].as_str().is_some_and(|m| m.trim_start().starts_with("Tool call validation failed"))
}

/// The most of a stream read to find its first data event.
const STREAM_PEEK: usize = 64 * 1024;

/// Reads a stream up to and including its first event with a `data:` line (comments before it are
/// kept), and whether that event is the malformed-tool-call error. The bytes read are put back in
/// front of the rest of the body.
fn first_event_is_refusal(reply: &mut Reply) -> bool {
    let mut seen = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut refused = false;
    let mut from = 0;
    'read: while seen.len() < STREAM_PEEK {
        let Ok(n) = reply.body.read(&mut chunk) else { break };
        if n == 0 {
            break;
        }
        seen.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&seen).replace("\r\n", "\n");
        while let Some(end) = text[from..].find("\n\n") {
            let event = &text[from..from + end];
            from += end + 2;
            let data: Vec<&str> = event.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim).collect();
            if data.is_empty() {
                continue;
            }
            refused = serde_json::from_str::<Value>(&data.join("\n")).is_ok_and(|v| says_malformed(&v));
            break 'read;
        }
    }
    let rest = std::mem::replace(&mut reply.body, Box::new(std::io::empty()));
    reply.body = Box::new(Cursor::new(seen).chain(rest));
    refused
}

impl Gateway {
    /// A 400 `tool_use_failed`, or a stream whose first event says it, is answered by sending the same bytes again, once, routed again as every
    /// resend is. The first try keeps its own row in the call log (same `rid`); the call itself carries
    /// the second, marked `resent_why: model_tool_call`. Any other reply is returned as it came, and a
    /// second refusal is returned like any reply.
    pub(crate) fn resend_bad_tool_call(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, body: &Value, mut reply: Reply, call: &mut Call) -> Result<Reply, Refusal> {
        let streamed = reply.status == 200 && reply.content_type.contains("event-stream");
        let failed = if streamed {
            first_event_is_refusal(&mut reply)
        } else if reply.status == 400 {
            let mut peek = Vec::new();
            let _ = (&mut reply.body).take(PEEK).read_to_end(&mut peek);
            let failed = is_tool_use_failed(&peek);
            let rest = std::mem::replace(&mut reply.body, Box::new(std::io::empty()));
            reply.body = Box::new(Cursor::new(peek).chain(rest));
            failed
        } else {
            false
        };
        if !failed {
            return Ok(reply);
        }
        let fresh = self.resolve_again(grant, asked, target)?;
        let bytes = serde_json::to_vec(body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
        tracing::info!(account = %target.account, "the model wrote a malformed tool call; sending the call once more");
        // The first try, as it ended: counts and names only.
        let mut first = call.clone();
        first.status = reply.status;
        first.outcome = "tool_use_failed".into();
        self.log.record(first);
        call.resent_why = Some(MODEL_TOOL_CALL.into());
        self.upstream.chat(&fresh.base_url, fresh.key.as_deref(), &bytes).map_err(|u| Refusal::new(502, u.code, u.message))
    }
}
