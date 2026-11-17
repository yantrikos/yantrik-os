//! The fall to the private lane (#1304): a call whose account is down before the first byte is
//! answered by the model the person chose for it, on this computer or their network, and the log
//! and a header say so.
//!
//! Only to [`Accounts::private_lane`], only when the person allows it ([`Accounts::fall_to_lane`])
//! and `policy::decide` has not refused, and only before a byte is written back: a stream that
//! was cut half-way is not sent again. The call is routed again before the second send
//! ([`Gateway::resolve_again`]), so a setting changed in between is honoured.

use std::io::Write;

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
    fn lane_to_fall_to(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target) -> Result<Option<Target>, Refusal> {
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
        self.resolve_again(grant, asked, target)?;
        Ok(Some(lane))
    }

    /// Before the first send: when the account is remembered as down and not due a try, answer
    /// from the lane without sending. `None` sends the call to the account as usual.
    pub(crate) fn skip_dead(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: &Value, call: &mut Call) -> Result<Option<Fall>, Refusal> {
        if !self.health.is_down(&target.account) {
            return Ok(None);
        }
        let Some(lane) = self.lane_to_fall_to(grant, asked, target)? else { return Ok(None) };
        if self.health.admit(&target.account) == Admit::Send {
            return Ok(None);
        }
        self.answer_from_lane(target, lane, pristine, call).map(Some)
    }

    /// The account failed before the first byte: remember it as down and answer from the lane.
    /// `None` when this call may not fall; the caller then fails as it always did.
    pub(crate) fn fall(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, pristine: &Value, call: &mut Call) -> Result<Option<Fall>, Refusal> {
        let Some(lane) = self.lane_to_fall_to(grant, asked, target)? else { return Ok(None) };
        self.health.failed(&target.account);
        tracing::info!(account = %target.account, lane = %lane.account, "the account is down; answering from the private lane");
        self.answer_from_lane(target, lane, pristine, call).map(Some)
    }

    /// The call as the lane's model takes it (its own effort words and parameters), sent once.
    fn answer_from_lane(&self, target: &Target, lane: Target, pristine: &Value, call: &mut Call) -> Result<Fall, Refusal> {
        let mut body = pristine.clone();
        let effort = body::rewrite(&mut body, &lane)?;
        body::drop_params(&mut body, body::known_refused(&lane));
        if let Some(map) = body.as_object_mut() {
            body::fit_sampling_to_thinking(map);
        }
        let bytes = serde_json::to_vec(&body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
        let both_down = || {
            Refusal::new(
                502,
                "account_and_lane_down",
                format!(
                    "{} could not be reached, and neither could your private model ({}/{}). Nothing was answered. Check your network or start the private model, then ask again.",
                    target.account, lane.account, lane.model
                ),
            )
        };
        match self.upstream.chat(&lane.base_url, lane.key.as_deref(), &bytes) {
            Ok(reply) if !outage(reply.status) => {
                call.lane_for = Some(format!("{}/{}", target.account, target.model));
                call.account = lane.account.clone();
                call.model = lane.model.clone();
                call.effort = effort.map(|e| e.as_str().to_string());
                Ok(Fall { reply, lane })
            }
            _ => Err(both_down()),
        }
    }

    /// Write the lane's answer back, naming who gave it.
    pub(crate) fn finish_fall(&self, fall: Fall, target: &Target, out: &mut impl Write, call: &mut Call) -> Result<(), Refusal> {
        call.status = fall.reply.status;
        call.outcome = OUTCOME.into();
        let answered = format!("{}/{}", fall.lane.account, fall.lane.model);
        let asked = format!("{}/{}", target.account, target.model);
        let _ = http::write_head_with(out, fall.reply.status, &fall.reply.content_type, &[(ANSWERED_BY, &answered), (FELL_FROM, &asked)]);
        crate::server::pump(fall.reply.body, out, call);
        if call.outcome != OUTCOME {
            call.outcome = format!("{OUTCOME}+{}", call.outcome);
        }
        Ok(())
    }
}

/// After a stream was cut half-way: one more event that says so in a sentence, since it cannot be
/// sent again. Only for an event stream, and only where a fall was possible, so a gateway with
/// the setting off writes what it always did.
pub(crate) fn say_cut(out: &mut impl Write, content_type: &str, call: &Call) {
    if call.outcome == "upstream_cut_off" && content_type.contains("event-stream") {
        let why = "The answer stopped part-way and was not sent again, because some of it had already reached you. Ask again to get a whole answer.";
        let event = serde_json::json!({ "error": { "message": why, "type": "yantrik_gateway", "code": "stream_cut" } });
        let _ = write!(out, "data: {event}\n\n").and_then(|_| out.flush());
    }
}
