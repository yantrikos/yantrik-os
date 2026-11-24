//! A request too large for an account is adjusted, not given up (#1390).
//!
//! The gateway knows each account's room and sits on every model call, so it fits the call there,
//! once, for every harness. In this order, each step only when the one before cannot make the call
//! fit:
//!
//! 1. **Know before sending**: estimate the call's tokens ([`estimate`]) against the account's
//!    room ([`Room`]); a call that clearly fits is sent untouched. An upstream 413 counts the same
//!    as a predicted one ([`Gateway::refit_after_413`]).
//! 2. **Wait, when it is the minute**: the call would fit an empty minute but not what this minute
//!    has used: wait the window out by [`crate::wait`]'s budget, then send.
//! 3. **Shrink, when it is the size**: the largest earlier tool results are cut to their head, on
//!    a line boundary, each ending in a line that says so ([`shrink`]). The system prompt, the
//!    person's messages and the newest message are never cut.
//! 4. **Move, when it cannot fit this account at all**: answered from the private lane by the
//!    rules of [`crate::fall`] (only where the person allowed it, only where `policy::decide`
//!    agrees, routed again before the send), when the lane has room.
//! 5. **Only then say why**: the gateway's own error, naming the account ([`too_long`]).
//!
//! Nothing of the content reaches a log, a header or an error: only counts.

use std::time::Duration;

use serde_json::Value;

use crate::fall::{Cause, Fall};
use crate::log::{Call, Shrunk};
use crate::policy::{Refusal, Target};
use crate::server::Gateway;
use crate::upstream::Reply;

/// Bytes counted as one token. Prose is about four bytes a token and code or JSON fewer, so a
/// count by this errs on the side of a call looking smaller than it is for the densest text; it is
/// the gateway's plain estimate, and an upstream 413 ([`Gateway::refit_after_413`]) is the
/// backstop for what it misses. It is rounded up, never down.
pub const BYTES_PER_TOKEN: u64 = 4;
/// The response header naming a shrink: counts only.
pub const SHRUNK: &str = "x-yantrik-shrunk";
/// The head of a cut tool result that is always kept, when it is that long.
const KEEP_HEAD: usize = 512;
/// Room counted for the line that says a result was cut, so one cut is mostly enough.
const MARKER_ROOM: usize = 128;

/// What an account's model takes in one request. Each is `None` where it is not known, and a call
/// is then not held to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Room {
    /// Tokens a minute (`tpm`): one call larger than this can never be taken.
    pub per_minute: Option<u64>,
    /// The model's context window, in tokens.
    pub context: Option<u64>,
}

impl Room {
    /// The most tokens one request may carry: the smaller of what is known.
    pub fn per_request(&self) -> Option<u64> {
        match (self.per_minute, self.context) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// The tokens a request is counted as: its whole body at [`BYTES_PER_TOKEN`], plus the answer
/// length it asks for (`max_tokens`), which providers count against the allowance too.
pub fn estimate(body: &Value) -> u64 {
    let bytes = serde_json::to_vec(body).map_or(0, |b| b.len() as u64);
    let asked = ["max_tokens", "max_completion_tokens"].iter().find_map(|k| body.get(*k).and_then(Value::as_u64)).unwrap_or(0);
    bytes.div_ceil(BYTES_PER_TOKEN).saturating_add(asked)
}

/// What a shrink says in the response header.
pub fn shrunk_words(s: Shrunk) -> String {
    format!("{} results, {} bytes", s.results, s.bytes)
}

/// The sentence that ends a turn nothing could fit, naming the account and the fix.
fn too_long_words(provider: &str) -> String {
    format!("{provider} cannot take this conversation: it is too long for it. Start a new chat, or use an account with more room.")
}

/// The refusal for a call that cannot fit before it is sent.
pub(crate) fn too_long(provider: &str) -> Refusal {
    Refusal::new(413, "upstream_413", too_long_words(provider))
}

/// An upstream 413 that goes back to the caller: the provider's body is dropped, the gateway's own
/// error is in its place (the shape of [`crate::wait::in_our_words`]).
fn too_long_reply(provider: &str) -> Reply {
    let out = serde_json::json!({"error": {"message": too_long_words(provider), "type": "invalid_request_error", "code": "upstream_413"}});
    Reply { status: 413, content_type: "application/json".into(), retry_after: None, body: Box::new(std::io::Cursor::new(out.to_string().into_bytes())) }
}

/// `s` cut to at most `at` bytes, on a line end when there is one in it, never inside a character.
fn head_on_line(s: &str, at: usize) -> &str {
    let mut end = at.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    match s[..end].rfind('\n') {
        Some(i) if i > 0 => &s[..i],
        _ => &s[..end],
    }
}

/// Cut earlier tool results in `body` until it is at most `limit` tokens, the largest first: each
/// to its head, with a last line saying it was cut, by how much, and that it can be asked for
/// again. Never touches a role, an order, a tool call, the system prompt, a person's message or the
/// newest message. `None`, with `body` as it was, when nothing more can be cut and it still does
/// not fit.
pub fn shrink(body: &mut Value, limit: u64) -> Option<Shrunk> {
    let mut work = body.clone();
    let mut done = Shrunk::default();
    let mut tried: Vec<usize> = Vec::new();
    while estimate(&work) > limit {
        let over = (estimate(&work) - limit).saturating_mul(BYTES_PER_TOKEN) as usize;
        let messages = work.get("messages")?.as_array()?;
        let newest = messages.len().checked_sub(1)?;
        let (at, len) = messages
            .iter()
            .enumerate()
            .filter(|(i, m)| *i != newest && !tried.contains(i) && m["role"] == "tool")
            .filter_map(|(i, m)| m["content"].as_str().map(|c| (i, c.len())))
            .max_by_key(|(_, len)| *len)?;
        tried.push(at);
        let text = work["messages"][at]["content"].as_str()?.to_string();
        let keep = len.saturating_sub(over + MARKER_ROOM).max(KEEP_HEAD.min(len));
        let head = head_on_line(&text, keep);
        let left_out = len - head.len();
        let cut = format!("{head}\n[Cut by the gateway to fit this account: {left_out} of {len} bytes left out. Ask again for the part you need.]");
        if cut.len() >= len {
            continue;
        }
        done.results += 1;
        done.bytes += (len - cut.len()) as u64;
        work["messages"][at]["content"] = Value::String(cut);
    }
    if done.results > 0 {
        *body = work;
    }
    Some(done)
}

/// What [`Gateway::fit_before_send`] settled.
pub(crate) enum Fitted {
    /// Send these bytes to the account.
    Send(Vec<u8>),
    /// The call was moved: this is the lane's answer.
    Fell(Fall),
}

/// What [`Gateway::refit_after_413`] settled.
pub(crate) enum Refit {
    /// The reply to go back: the account's answer to the smaller call, or the gateway's own error.
    Sent(Reply),
    Fell(Fall),
}

/// Add a shrink to what the call's log row already says.
fn note_shrunk(call: &mut Call, more: Shrunk) {
    if more.results == 0 {
        return;
    }
    let was = call.shrunk.unwrap_or_default();
    call.shrunk = Some(Shrunk { results: was.results + more.results, bytes: was.bytes + more.bytes });
}

impl Gateway {
    fn encode(body: &Value) -> Result<Vec<u8>, Refusal> {
        serde_json::to_vec(body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))
    }

    /// Steps 1 to 5 before the first send. `body` is the call as this account takes it; a shrink
    /// changes it in place. `pristine` is what a move to the lane would send (only when the
    /// person allowed a fall).
    pub(crate) fn fit_before_send(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: Option<&Value>, body: &mut Value, call: &mut Call) -> Result<Fitted, Refusal> {
        let room = self.accounts.room(&target.account, &target.model);
        let Some(limit) = room.per_request() else { return Self::encode(body).map(Fitted::Send) };
        let mut tokens = estimate(body);
        if tokens > limit {
            match shrink(body, limit) {
                Some(done) => {
                    note_shrunk(call, done);
                    tokens = estimate(body);
                    tracing::info!(account = %target.account, results = done.results, bytes = done.bytes, "the call was too large for the account; earlier tool results were cut");
                }
                None => return self.cannot_fit(grant, asked, target, pristine, call, tokens),
            }
        }
        let key = format!("{}/{}", target.account, target.model);
        if let Some(per_minute) = room.per_minute {
            let wait = self.holds.until_room(&key, tokens, per_minute);
            if wait > Duration::ZERO && self.sit_out_minute(wait, call) {
                // After a wait the call is routed again, as every resend is.
                self.resolve_again(grant, asked, target)?;
            }
            self.holds.spend(&key, tokens);
        }
        Self::encode(body).map(Fitted::Send)
    }

    /// An upstream 413: the call is cut to about half of what it was counted as (or of the room, if
    /// that is less) and sent once more. A call that cannot be cut, or that is refused again, is
    /// moved or ended in the gateway's own words.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refit_after_413(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: Option<&Value>, body: &mut Value, call: &mut Call, refused: Reply) -> Result<Refit, Refusal> {
        drop(refused);
        let tokens = estimate(body);
        let room = self.accounts.room(&target.account, &target.model).per_request();
        let goal = room.map_or(tokens, |r| r.min(tokens)) / 2;
        let again = match shrink(body, goal) {
            Some(done) if done.results > 0 => {
                note_shrunk(call, done);
                let fresh = self.resolve_again(grant, asked, target)?;
                let bytes = Self::encode(body)?;
                self.upstream.chat(&fresh.base_url, fresh.key.as_deref(), &bytes).map_err(|u| Refusal::new(502, u.code, u.message))?
            }
            _ => return self.moved_or_said(grant, asked, target, pristine, call, tokens),
        };
        if again.status == 413 {
            return self.moved_or_said(grant, asked, target, pristine, call, tokens);
        }
        Ok(Refit::Sent(again))
    }

    /// Step 4 or 5 for a call that is already out: the lane, or the gateway's words as a reply.
    fn moved_or_said(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: Option<&Value>, call: &mut Call, tokens: u64) -> Result<Refit, Refusal> {
        match pristine.map(|p| self.fall(grant, asked, target, p, call, Cause::too_large(tokens))).transpose()?.flatten() {
            Some(fall) => Ok(Refit::Fell(fall)),
            None => Ok(Refit::Sent(too_long_reply(&target.provider))),
        }
    }

    /// Step 4 or 5 for a call that cannot be made to fit before it is sent.
    fn cannot_fit(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: Option<&Value>, call: &mut Call, tokens: u64) -> Result<Fitted, Refusal> {
        match pristine.map(|p| self.fall(grant, asked, target, p, call, Cause::too_large(tokens))).transpose()?.flatten() {
            Some(fall) => Ok(Fitted::Fell(fall)),
            None => Err(too_long(&target.provider)),
        }
    }
}
