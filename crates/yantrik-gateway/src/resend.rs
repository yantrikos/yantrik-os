//! A 400 that says the model's own tool call was malformed (Groq's `tool_use_failed`, which gpt-oss
//! brings on now and then by writing its channel markup into the tool's name): the same call is sent
//! once more. Nothing has gone to the client yet, since a 400 arrives whole before any stream starts.

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
    serde_json::from_slice::<Value>(raw).ok().is_some_and(|v| v["error"]["code"] == "tool_use_failed")
}

impl Gateway {
    /// A 400 `tool_use_failed` is answered by sending the same bytes again, once, routed again as every
    /// resend is. The first try keeps its own row in the call log (same `rid`); the call itself carries
    /// the second, marked `resent_why: model_tool_call`. Any other reply is returned as it came, and a
    /// second refusal is returned like any reply.
    pub(crate) fn resend_bad_tool_call(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, body: &Value, mut reply: Reply, call: &mut Call) -> Result<Reply, Refusal> {
        if reply.status != 400 {
            return Ok(reply);
        }
        let mut peek = Vec::new();
        let _ = (&mut reply.body).take(PEEK).read_to_end(&mut peek);
        let failed = is_tool_use_failed(&peek);
        let rest = std::mem::replace(&mut reply.body, Box::new(std::io::empty()));
        reply.body = Box::new(Cursor::new(peek).chain(rest));
        if !failed {
            return Ok(reply);
        }
        let fresh = self.resolve_again(grant, asked, target)?;
        let bytes = serde_json::to_vec(body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
        tracing::info!(account = %target.account, "the model wrote a malformed tool call; sending the call once more");
        // The first try, as it ended: counts and names only.
        let mut first = call.clone();
        first.status = 400;
        first.outcome = "tool_use_failed".into();
        self.log.record(first);
        call.resent_why = Some(MODEL_TOOL_CALL.into());
        self.upstream.chat(&fresh.base_url, fresh.key.as_deref(), &bytes).map_err(|u| Refusal::new(502, u.code, u.message))
    }
}
