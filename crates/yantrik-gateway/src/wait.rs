//! A 429 whose provider named a short wait: the gateway sits it out and sends the call again,
//! so a window that closes in seconds does not fail a turn.

use std::time::Duration;

use serde_json::Value;

use crate::log::Call;
use crate::policy::{Refusal, Target};
use crate::server::Gateway;
use crate::upstream::Reply;

/// The most one call waits for in all, the named waits added up: a fixed one-minute window never
/// names more than 60 s.
pub(crate) const BUDGET: Duration = Duration::from_secs(65);
/// Added to each named wait: the provider's seconds are whole, its window may close a moment later.
const PAD: Duration = Duration::from_millis(250);
/// The most waits one call makes: a provider that keeps naming 0 s would otherwise never use up
/// the budget, and the call would be sent four times a second for as long as it said so.
pub(crate) const MAX_WAITS: u32 = 3;

/// The wait a 429 names: `retry-after` in whole seconds, else `ratelimit-reset` in whole seconds.
/// A `retry-after` that is a date, negative or not a number names none, and does not fall back.
pub(crate) fn named_wait(retry_after: Option<&str>, reset: Option<&str>) -> Option<Duration> {
    let secs = |v: &str| v.trim().parse::<u64>().ok().map(Duration::from_secs);
    secs(retry_after.or(reset)?)
}

impl Gateway {
    /// While the reply is a 429 that names a wait the call's budget still holds, [`MAX_WAITS`]
    /// times at most: wait it (and a quarter second), route the call again as every resend does,
    /// and send the same bytes again. Any other reply, or a 429 naming no wait or too long a
    /// one, or the one after the last wait, is returned as it is; so is a 400 that answers a
    /// resend (the 400 handling ran before the first wait and is not run again).
    pub(crate) fn wait_out_429(&self, grant: &crate::tokens::Grant, asked: &str, target: &Target, body: &Value, mut reply: Reply, call: &mut Call) -> Result<Reply, Refusal> {
        let mut named = Duration::ZERO;
        let mut times = 0;
        // A call somebody asked that meets a 429 holds the account for the companion's own calls.
        let asked_call = call.asked.is_none();
        while reply.status == 429 && times < MAX_WAITS {
            if asked_call {
                self.holds.hold(&target.account, reply.retry_after);
            }
            // The seconds are the provider's own: added with a check, since a number near the
            // largest there is would otherwise panic this thread in the sum.
            let Some(d) = reply.retry_after.filter(|d| named.checked_add(*d).is_some_and(|all| all <= BUDGET)) else { break };
            named += d;
            times += 1;
            call.waited_ms = (named + PAD * times).as_millis() as u64;
            tracing::info!(account = %target.account, seconds = d.as_secs(), wait = times, "the provider named a wait on a 429; waiting it out");
            self.upstream.wait(d + PAD);
            let fresh = self.resolve_again(grant, asked, target)?;
            let bytes = serde_json::to_vec(body).map_err(|_| Refusal::new(400, "bad_request", "The request could not be re-encoded."))?;
            reply = self.upstream.chat(&fresh.base_url, fresh.key.as_deref(), &bytes).map_err(|u| Refusal::new(502, u.code, u.message))?;
        }
        if asked_call && reply.status == 429 {
            self.holds.hold(&target.account, reply.retry_after);
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_a_429_names_comes_from_two_headers() {
        let s = Duration::from_secs;
        let table: [(Option<&str>, Option<&str>, Option<Duration>); 11] = [
            (Some("4"), None, Some(s(4))),
            (Some(" 4 "), None, Some(s(4))),
            (None, Some("9"), Some(s(9))),
            (Some("4"), Some("9"), Some(s(4))),
            (Some("0"), None, Some(s(0))),
            (Some("Wed, 21 Oct 2026 07:28:00 GMT"), Some("9"), None),
            (Some("-3"), Some("9"), None),
            (Some("soon"), None, None),
            (Some("1.5"), None, None),
            (None, Some("x"), None),
            (None, None, None),
        ];
        for (retry, reset, want) in table {
            assert_eq!(named_wait(retry, reset), want, "retry-after {retry:?}, ratelimit-reset {reset:?}");
        }
    }
}
