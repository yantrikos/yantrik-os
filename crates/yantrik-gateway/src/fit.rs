//! A request too large for an account is adjusted, not given up (#1390).
//!
//! The gateway knows each account's room and sits on every model call, so it fits the call there,
//! once, for every harness. In this order, each step only when the one before cannot make the call
//! fit:
//!
//! 1. **Know before sending**: estimate the call's tokens ([`estimate`]) against the account's
//!    room ([`Room`]); a call that clearly fits is sent untouched. The estimate is a guess: it
//!    never refuses or moves a call, and one that still looks too large after step 3 is sent. The
//!    account's own 413 decides, and counts as a predicted overflow ([`Gateway::refit_after_413`]).
//! 2. **Wait, when it is the minute**: the call would fit an empty minute but not what this minute
//!    has used: wait the window out by [`crate::wait`]'s budget, then send.
//! 3. **Shrink, when it is the size**: the largest earlier tool results are cut to their head, on
//!    a line boundary, each ending in a line that says so, the newest tool result last
//!    ([`shrink`]). The system prompt and the person's messages are never cut.
//! 4. **Move, when it cannot fit this account at all**: answered from the private lane by the
//!    rules of [`crate::fall`] (only where the person allowed it, only where `policy::decide`
//!    agrees, routed again before the send), when the lane has room.
//! 5. **Only then say why** (after the account's own 413): the gateway's own error, naming the account ([`too_long`]).
//!
//! Nothing of the content reaches a log, a header or an error: only counts.

use std::io::Read as _;
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

/// The tokens a request's prompt is counted as: its whole body at [`BYTES_PER_TOKEN`]. This is
/// a guess and never decides alone that a call cannot be sent: a call that still looks too large
/// after a shrink is sent, and only the account's own 413 ends it.
pub fn estimate(body: &Value) -> u64 {
    serde_json::to_vec(body).map_or(0, |b| b.len() as u64).div_ceil(BYTES_PER_TOKEN)
}

/// The answer length a call asks for (`max_tokens` / `max_completion_tokens`). It counts against the
/// context window, not the per-minute allowance (a harness may ask for far more than it uses, and
/// accounts accept such calls).
pub fn answer_asked(body: &Value) -> u64 {
    ["max_tokens", "max_completion_tokens"].iter().find_map(|k| body.get(*k).and_then(Value::as_u64)).unwrap_or(0)
}

impl Room {
    /// The most prompt tokens a call asking for `answer` more may carry: the per-minute allowance
    /// as it is, and the context window less the answer.
    pub fn prompt_limit(&self, answer: u64) -> Option<u64> {
        let context = self.context.map(|c| c.saturating_sub(answer));
        match (self.per_minute, context) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// Lower the answer length a call asks for (`max_tokens` / `max_completion_tokens`) to `ceiling`
/// tokens when it asks for more: what the account can still give, but never below `floor`, the least
/// answer worth sending (a thinking model spends a few tokens before it says anything). Never raises it, and
/// a call that names none is left to the account's own default. `Some((asked, now))` when it changed.
pub fn clamp_answer(body: &mut Value, ceiling: u64, floor: u64) -> Option<(u64, u64)> {
    let now = ceiling.max(floor).max(1);
    let map = body.as_object_mut()?;
    let mut changed = None;
    for key in ["max_tokens", "max_completion_tokens"] {
        if let Some(asked) = map.get(key).and_then(Value::as_u64).filter(|a| *a > now) {
            map.insert(key.into(), Value::from(now));
            changed = Some((asked, now));
        }
    }
    changed
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

/// The tokens an account said it counted and its limit, from the words of its 413 (OpenAI-shaped
/// bodies: Groq's "Limit 8000, Requested 9500", OpenAI's "maximum context length is 8192 tokens ...
/// resulted in 9500 tokens"). Numbers only, each where the account stated it and none otherwise.
pub(crate) fn counted_by(body: &str) -> (Option<u64>, Option<u64>) {
    let after = |marker: &str| -> Option<u64> {
        let rest = &body[body.find(marker)? + marker.len()..];
        let digits: String = rest.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit() || *c == ',').filter(|c| *c != ',').collect();
        digits.parse().ok()
    };
    let counted = after("Requested ").or_else(|| after("resulted in ")).or_else(|| after("requested "));
    let limit = after("Limit ").or_else(|| after("maximum context length is "));
    (counted, limit)
}

/// What an upstream 413 said, read for its two numbers and then dropped (the body is the provider's
/// own words, which never go on): at most the first 8 KiB of it.
fn note_refusal(call: &mut Call, refused: Reply) {
    let mut text = Vec::new();
    let _ = std::io::Read::take(refused.body, 8192).read_to_end(&mut text);
    (call.counted, call.limit) = counted_by(&String::from_utf8_lossy(&text));
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

/// The text of a tool message that can be cut: its string content, or each text part of an array
/// content. `(part, bytes)`; `part` is the index in the array.
fn slots(message: &Value) -> Vec<(Option<usize>, usize)> {
    match &message["content"] {
        Value::String(c) => vec![(None, c.len())],
        Value::Array(parts) => parts.iter().enumerate().filter_map(|(i, p)| p["text"].as_str().map(|t| (Some(i), t.len()))).collect(),
        _ => Vec::new(),
    }
}

fn slot_mut(message: &mut Value, part: Option<usize>) -> Option<&mut Value> {
    match part {
        None => message.get_mut("content"),
        Some(i) => message.get_mut("content")?.get_mut(i)?.get_mut("text"),
    }
}

/// Cut tool results in `body` until its prompt is at most `limit` tokens: earlier results first,
/// the largest first, then the newest tool result last; each to its head on a line end, with a
/// last line saying it was cut, by how much, and that it can be asked for again. Text parts of an
/// array content are cut the same way. Never touches a role, an order, a tool call, the system
/// prompt or a person's message. `None`, with `body` as it was, when nothing more can be cut and it
/// still does not fit.
pub fn shrink(body: &mut Value, limit: u64) -> Option<Shrunk> {
    let mut work = body.clone();
    let mut done = Shrunk::default();
    let mut tried: Vec<(usize, Option<usize>)> = Vec::new();
    while estimate(&work) > limit {
        let over = (estimate(&work) - limit).saturating_mul(BYTES_PER_TOKEN) as usize;
        let messages = work.get("messages")?.as_array()?;
        let newest = messages.len().checked_sub(1)?;
        let (at, part, len) = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m["role"] == "tool")
            .flat_map(|(i, m)| slots(m).into_iter().map(move |(p, n)| (i, p, n)))
            .filter(|(i, p, _)| !tried.contains(&(*i, *p)))
            .min_by_key(|(i, _, n)| (*i == newest, std::cmp::Reverse(*n)))?;
        tried.push((at, part));
        let text = slot_mut(&mut work["messages"][at], part)?.as_str()?.to_string();
        let keep = len.saturating_sub(over + MARKER_ROOM).max(KEEP_HEAD.min(len));
        let head = head_on_line(&text, keep);
        let left_out = len - head.len();
        let cut = format!("{head}\n[Cut by the gateway to fit this account: {left_out} of {len} bytes left out. Ask again for the part you need.]");
        if cut.len() >= len {
            continue;
        }
        done.results += 1;
        done.bytes += (len - cut.len()) as u64;
        *slot_mut(&mut work["messages"][at], part)? = Value::String(cut);
    }
    if done.results > 0 {
        *body = work;
    }
    Some(done)
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

    /// Steps 1 to 3 before the first send. `body` is the call as this account takes it; a shrink
    /// changes it in place. The estimate never refuses or moves a call: one that still looks too
    /// large after the shrink is sent, and only the account's own 413 sends it on to
    /// [`Self::refit_after_413`].
    pub(crate) fn fit_before_send(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, body: &mut Value, call: &mut Call) -> Result<Vec<u8>, Refusal> {
        let room = self.accounts.room(&target.account, &target.model);
        let mut tokens = estimate(body);
        if let Some(limit) = room.prompt_limit(answer_asked(body)) {
            if tokens > limit {
                if let Some(done) = shrink(body, limit) {
                    note_shrunk(call, done);
                    tokens = estimate(body);
                    tracing::info!(account = %target.account, results = done.results, bytes = done.bytes, "the call looked too large for the account; earlier tool results were cut");
                }
            }
        }
        let key = format!("{}/{}", target.account, target.model);
        if let Some(per_minute) = room.per_minute {
            // A call larger than a whole minute never fits one: it is sent, not waited for.
            let wait = if tokens <= per_minute { self.holds.until_room(&key, tokens, per_minute) } else { Duration::ZERO };
            if wait > Duration::ZERO && self.sit_out_minute(wait, call) {
                // After a wait the call is routed again, as every resend is.
                self.resolve_again(grant, asked, target)?;
            }
            self.holds.spend(&key, call.rid, tokens);
        }
        // The answer length fitted to the day and the context, and the call counted against the day.
        self.day_fit(grant, target, body, tokens, call);
        Self::encode(body)
    }

    /// The reply's real prompt tokens, once they are known, in place of the estimate in the
    /// minute's ledger.
    pub(crate) fn learn_spent(&self, target: &Target, call: &Call) {
        if call.prompt_tokens > 0 {
            self.holds.correct(&format!("{}/{}", target.account, target.model), call.rid, call.prompt_tokens);
        }
    }

    /// An upstream 413: the call is cut to about half of what it was counted as (or of the room, if
    /// that is less) and sent once more. A call that cannot be cut, or that is refused again, is
    /// moved or ended in the gateway's own words.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refit_after_413(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: Option<&Value>, body: &mut Value, call: &mut Call, refused: Reply) -> Result<Refit, Refusal> {
        note_refusal(call, refused);
        let tokens = estimate(body);
        let room = self.accounts.room(&target.account, &target.model).prompt_limit(answer_asked(body));
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
}
