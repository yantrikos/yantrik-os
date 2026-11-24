//! The fall to the private lane (#1304): a call whose account is down before the first byte is
//! answered by the model the person chose for it, on this computer or their network, and the log
//! and a header say so.
//!
//! Only to [`Accounts::private_lane`], only when the person allows it ([`Accounts::fall_to_lane`])
//! and `policy::decide` has not refused, and only before a byte is written back: a stream that
//! was cut half-way is not sent again. The call is routed again before the second send
//! ([`Gateway::resolve_again`]), so a setting changed in between is honoured.

use std::io::Write;
use std::time::Duration;

use serde_json::Value;

use crate::body;
use crate::health::Admit;
use crate::http;
use crate::log::Call;
use crate::policy::{self, Decision, Refusal, Target};
use crate::route;
use crate::server::Gateway;
use crate::upstream::Reply;

/// The log outcome of a call the private lane answered because its account was down: not
/// `lane_for` alone, which a privacy lane sets too.
pub const OUTCOME: &str = "fell_to_lane";
/// The response header naming the `<account>/<model>` that answered.
pub const ANSWERED_BY: &str = "x-yantrik-answered-by";
/// On a fall: the `<account>/<model>` that was asked for and did not answer.
pub const FELL_FROM: &str = "x-yantrik-fell-from";

/// A lane's answer, and the lane that gave it.
pub(crate) struct Fall {
    pub reply: Reply,
    pub lane: Target,
    /// The status the account answered with before the fall; none when it was unreachable.
    pub from_status: Option<u16>,
}

/// Why a call fell: the account could not be reached, or answered with a 5xx.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cause {
    pub status: Option<u16>,
    pub(crate) retry_after: Option<Duration>,
    /// For a call too large for its account ([`Cause::too_large`]): the tokens it takes, so the
    /// lane is chosen only if it has the room.
    pub(crate) need: Option<u64>,
}

impl Cause {
    pub(crate) fn unreachable() -> Cause {
        Cause { status: None, retry_after: None, need: None }
    }

    pub(crate) fn answered(reply: &Reply) -> Cause {
        Cause { status: Some(reply.status), retry_after: reply.retry_after, need: None }
    }

    /// The call cannot fit this account, however it is cut ([`crate::fit`]): answered by the lane
    /// by the same rules as a fall, when the lane has room for `need` tokens. Not remembered as
    /// down: the account is well, the call was too big for it.
    pub(crate) fn too_large(need: u64) -> Cause {
        Cause { status: Some(413), retry_after: None, need: Some(need) }
    }

    /// Whether the account is remembered as down for it: unreachable, timed out, or one of the
    /// statuses that mean the service is down (502, 503, 504, 529). A bare 500 is as often a
    /// caller's own bad request that a provider answers badly, and one harness's must not move
    /// every harness: it falls that one call and is forgotten.
    fn remembered(&self) -> bool {
        self.status.is_none_or(|s| matches!(s, 502 | 503 | 504 | 529))
    }
}

/// A reply that means the account is down, not that it refused: a 5xx.
pub(crate) fn outage(status: u16) -> bool {
    (500..600).contains(&status)
}

impl Gateway {
    /// The lane `target`'s call may fall to now: `None` when the setting is off, there is no lane,
    /// it is not on this computer or network, it is the account that is down, or it is not
    /// allowed to take the call. A refusal when the call, routed again, is refused or goes
    /// elsewhere: what policy refused is never turned into a fall.
    fn lane_to_fall_to(&self, grant: &crate::tokens::Grant, asked: &str, mark: Option<&str>, target: &Target, need: Option<u64>) -> Result<Option<Target>, Refusal> {
        if !self.accounts.fall_to_lane() {
            return Ok(None);
        }
        let Some(lane) = self.accounts.private_lane().and_then(|id| route::parse(&id)) else { return Ok(None) };
        if lane.account == target.account {
            return Ok(None);
        }
        let Ok(lane) = self.accounts.target(&lane.account, &lane.model) else { return Ok(None) };
        if !lane.local || policy::decide(grant, &lane, self.accounts.private_mode()) != Ok(Decision::Send) {
            return Ok(None);
        }
        // The rule that keeps a tight account for what is asked holds for the lane too.
        if crate::asked::kept_for_asked(grant, mark, &lane).is_some() {
            return Ok(None);
        }
        // A call that is too large for its account is not moved to a lane that cannot take it either.
        let room = self.accounts.room(&lane.account, &lane.model).per_request();
        if need.zip(room).is_some_and(|(n, r)| n > r) {
            return Ok(None);
        }
        self.resolve_again(grant, asked, target)?;
        Ok(Some(lane))
    }

    /// Before the first send: when the account is remembered as down and not due a try, answer
    /// from the lane without sending. `None` sends the call to the account as usual.
    pub(crate) fn skip_dead(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: &Value, call: &mut Call) -> Result<Option<Fall>, Refusal> {
        if !self.health.is_down(&target.account) {
            return Ok(None);
        }
        let Some(lane) = self.lane_to_fall_to(grant, asked, call.asked.as_deref(), target, None)? else { return Ok(None) };
        if self.health.admit(&target.account) == Admit::Send {
            return Ok(None);
        }
        self.answer_from_lane(target, lane, pristine, call, Cause::unreachable()).map(Some)
    }

    /// The account failed before the first byte: remember it as down and answer from the lane.
    /// `None` when this call may not fall; the caller then fails as it always did.
    pub(crate) fn fall(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: &Value, call: &mut Call, cause: Cause) -> Result<Option<Fall>, Refusal> {
        let Some(lane) = self.lane_to_fall_to(grant, asked, call.asked.as_deref(), target, cause.need)? else { return Ok(None) };
        if cause.remembered() {
            self.health.failed(&target.account, cause.retry_after);
        }
        tracing::info!(account = %target.account, lane = %lane.account, status = ?cause.status, "the account is down; answering from the private lane");
        self.answer_from_lane(target, lane, pristine, call, cause).map(Some)
    }

    /// The call as the lane's model takes it (its own effort words and parameters), sent once.
    fn answer_from_lane(&self, target: &Target, lane: Target, pristine: &Value, call: &mut Call, cause: Cause) -> Result<Fall, Refusal> {
        let mut body = pristine.clone();
        let effort = body::rewrite(&mut body, &lane)?;
        body::drop_params(&mut body, body::known_refused(&lane));
        if let Some(map) = body.as_object_mut() {
            body::fit_sampling_to_thinking(map);
        }
        let bytes = serde_json::to_vec(&body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
        let both_down = || {
            // What the account did is said as it was: an answer with an error is not a connection
            // that failed, and the person fixes a different thing.
            let what = match cause.status {
                Some(s) => format!("{} answered with an error ({s})", target.account),
                None => format!("{} could not be reached", target.account),
            };
            Refusal::new(
                502,
                "account_and_lane_down",
                format!(
                    "{what}, and your private model ({}/{}) did not answer either. Nothing was answered. Check your network or start the private model, then ask again.",
                    lane.account, lane.model
                ),
            )
        };
        match self.upstream.chat(&lane.base_url, lane.key.as_deref(), &bytes) {
            Ok(reply) if !outage(reply.status) => {
                call.lane_for = Some(format!("{}/{}", target.account, target.model));
                call.account = lane.account.clone();
                call.model = lane.model.clone();
                call.effort = effort.map(|e| e.as_str().to_string());
                Ok(Fall { reply, lane, from_status: cause.status })
            }
            _ => Err(both_down()),
        }
    }

    /// Write the lane's answer back, naming who gave it.
    pub(crate) fn finish_fall(&self, fall: Fall, target: &Target, out: &mut impl Write, call: &mut Call) -> Result<(), Refusal> {
        call.status = fall.reply.status;
        // The status the account gave stays in the row, which `status` (the lane's) cannot hold.
        call.outcome = match fall.from_status {
            Some(s) => format!("{OUTCOME}+upstream_{s}"),
            None => OUTCOME.into(),
        };
        let fell = call.outcome.clone();
        let answered = format!("{}/{}", fall.lane.account, fall.lane.model);
        let asked = format!("{}/{}", target.account, target.model);
        let _ = http::write_head_with(out, fall.reply.status, &fall.reply.content_type, &[(ANSWERED_BY, &answered), (FELL_FROM, &asked)]);
        crate::server::pump(fall.reply.body, out, call);
        if call.outcome != fell {
            call.outcome = format!("{fell}+{}", call.outcome);
        }
        Ok(())
    }
}

/// After a stream was cut half-way: one more event that says so in a sentence, since it cannot be
/// sent again. It starts with a blank line, which ends any event the cut left half-written;
/// without it the error would join that partial line and no parser would read it. Only for an event stream, and only where a fall was possible, so a gateway with
/// the setting off writes what it always did.
pub(crate) fn say_cut(out: &mut impl Write, content_type: &str, call: &Call) {
    if call.outcome == "upstream_cut_off" && content_type.contains("event-stream") {
        let why = "The answer stopped part-way and was not sent again, because some of it had already reached you. Ask again to get a whole answer.";
        let event = serde_json::json!({ "error": { "message": why, "type": "yantrik_gateway", "code": "stream_cut" } });
        let _ = write!(out, "\n\ndata: {event}\n\n").and_then(|_| out.flush());
    }
}
