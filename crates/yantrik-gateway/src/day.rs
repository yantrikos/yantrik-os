//! A call fitted to the account's day (#1400): the gateway reads each tier's published daily
//! limits through [`Accounts::daily`], counts what was sent against them ([`crate::budget`]), and
//! before a call is sent it:
//!
//! 1. keeps the companion to its share of the day (the rule is [`crate::asked::over_its_share`]);
//! 2. moves a call the day cannot take to another model of the same account that has room, then to
//!    one of the person's other accounts (#1401), by the same checks a fall makes (`policy::decide`, the kept-for-asked rule, routed again); and
//!    otherwise refuses it at once, naming the account, the limit and when it frees up;
//! 3. fits the answer length to what the day and the context can still give ([`crate::fit::clamp_answer`]).
//!
//! Counts and times only: nothing of a call's content is read for any of it beyond its length.

use serde_json::Value;

use crate::budget::{short, wait_words, Daily, Today};
use crate::fit::{answer_asked, clamp_answer, estimate};
use crate::log::Call;
use crate::policy::{self, Decision, Refusal, Target};
use crate::route;
use crate::server::Gateway;
use crate::tokens::Grant;

/// The log outcome of a call refused because the day cannot take it.
pub const DAY_SPENT: &str = "day_spent";

/// The least answer, in tokens, a call is worth sending with. A thinking model spends tokens
/// reasoning before it says a word, so an answer clamped to a few dozen tokens comes back cut off
/// or empty, and the day's tokens are spent for nothing. The gate counts this much beyond the prompt,
/// and the clamp never goes below it (never above what the harness asked for, either).
pub const MIN_ANSWER: u64 = 1_024;

/// The answer this call is counted as needing at least: [`MIN_ANSWER`], or less if it asked for less.
fn min_answer(body: &Value) -> u64 {
    match answer_asked(body) {
        0 => MIN_ANSWER,
        asked => asked.min(MIN_ANSWER),
    }
}

/// What a call needs of a model's day: its prompt and the least answer worth sending.
pub(crate) fn needed(body: &Value) -> u64 {
    estimate(body).saturating_add(min_answer(body))
}

/// Whether a request carries tool definitions, so only a model that calls tools may take it.
fn wants_tools(body: &Value) -> bool {
    ["tools", "functions"].iter().any(|k| body.get(*k).and_then(Value::as_array).is_some_and(|a| !a.is_empty()))
}

/// Whether any message carries an image, so only a model that sees images may take it.
fn carries_image(body: &Value) -> bool {
    body["messages"].as_array().is_some_and(|ms| {
        ms.iter().any(|m| m["content"].as_array().is_some_and(|parts| parts.iter().any(|p| matches!(p["type"].as_str(), Some("image_url" | "image" | "input_image")))))
    })
}

/// One model's use today against its limits, for [`Gateway::day_lines`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayLine {
    pub model: String,
    pub daily: Daily,
    pub today: Today,
}

impl DayLine {
    /// "Groq (free), gpt-oss-120b: 150K of 200K tokens and 12 of 1,000 requests today; the companion
    /// has used 50K."
    pub fn words(&self, provider: &str) -> String {
        let mut uses = Vec::new();
        if let Some(t) = self.daily.tokens {
            uses.push(format!("{} of {} tokens", short(self.today.tokens), short(t)));
        }
        if let Some(r) = self.daily.requests {
            uses.push(format!("{} of {} requests", short(self.today.requests), short(r)));
        }
        let name = if self.daily.per_account { provider.to_string() } else { format!("{provider}, {}", self.model) };
        let companion = match (self.daily.tokens, self.daily.requests) {
            (Some(_), _) => short(self.today.itself_tokens) + " tokens",
            (None, _) => short(self.today.itself_requests) + " requests",
        };
        format!("{name}: {} today; the companion has used {companion}.", uses.join(" and "))
    }
}

impl Gateway {
    fn today_of(&self, target: &Target, daily: &Daily) -> Today {
        self.holds.day.today(&daily.key(&target.account, &target.model), daily.end)
    }

    /// The companion's own call to a model whose share of the day it has used up, as a refusal.
    pub(crate) fn share_refusal(&self, grant: &Grant, mark: Option<&str>, target: &Target) -> Option<Refusal> {
        if !crate::asked::by_itself(grant, mark) {
            return None;
        }
        let daily = self.accounts.daily(&target.account, &target.model);
        if daily.is_none() {
            return None;
        }
        crate::asked::over_its_share(grant, mark, target, &daily, &self.today_of(target, &daily))
    }

    /// Know before sending: `None` when the day takes the call where it is going; the model it was
    /// moved to (`<account>/<model>` and its target) when another model of the account has room;
    /// else the refusal, naming the account, the limit and when it frees up.
    pub(crate) fn day_gate(&self, grant: &Grant, target: &Target, body: &Value, call: &Call) -> Result<Option<(String, Target)>, Refusal> {
        let daily = self.accounts.daily(&target.account, &target.model);
        if daily.is_none() {
            return Ok(None);
        }
        // What the call will weigh once it is fitted to the minute's room.
        let room = self.accounts.room(&target.account, &target.model);
        let prompt = room.prompt_limit(answer_asked(body)).map_or(estimate(body), |l| estimate(body).min(l));
        let need = prompt.saturating_add(min_answer(body));
        let today = self.today_of(target, &daily);
        if daily.takes(&today, need) {
            return Ok(None);
        }
        // The same account first (when its limit is per model), then the person's other accounts.
        let mark = call.asked.as_deref();
        let same = (!daily.per_account).then(|| self.model_with_room(grant, target, need, body, mark, true)).flatten();
        if let Some(moved) = same.or_else(|| self.model_with_room(grant, target, need, body, mark, false)) {
            tracing::info!(account = %target.account, from = %target.model, to = %moved.0, "the day cannot take the call on this model; moved to a model with room");
            return Ok(Some(moved));
        }
        Err(spent(target, &daily, &today, need))
    }

    /// A model the call may go to instead of `target`: within `target`'s account (`same_account`),
    /// or on one of the person's other accounts, in the picker's order (the order of
    /// [`Accounts::models`]). It has the day's room and the context, tools and sight the call
    /// needs; policy sends the call there as it would to the account the person picked (so a call
    /// that may go only to the private lane never goes to a cloud account); the companion's rules
    /// still allow it; and the call, routed again by that name, goes there and nowhere else. The
    /// companion's own calls never go to another account, and neither does one to an account
    /// remembered as down. The private lane is not here: it is chosen after these, by [`crate::fall`].
    pub(crate) fn model_with_room(&self, grant: &Grant, target: &Target, need: u64, body: &Value, mark: Option<&str>, same_account: bool) -> Option<(String, Target)> {
        // The companion's own calls wait for their account; a call on this computer or network
        // never goes to a less private place; and a model here is the private lane's business.
        if !same_account && (crate::asked::by_itself(grant, mark) || target.local) {
            return None;
        }
        let lane = self.accounts.private_lane();
        // (tight, room left today, picker order, model, where): ranked below for another account.
        let mut fit: Vec<(bool, u64, usize, String, Target)> = Vec::new();
        for (order, m) in self.accounts.models().into_iter().enumerate() {
            let Some(r) = route::parse(&m.id) else { continue };
            if (r.account == target.account) != same_account || (same_account && r.model == target.model) {
                continue;
            }
            if !same_account && (m.local || lane.as_deref() == Some(m.id.as_str()) || self.health.is_down(&r.account)) {
                continue;
            }
            // An account that just refused a call (its 429's window), or whose minute of requests is
            // spent, is not moved to until it frees up.
            if !same_account && (self.holds.refusing(&r.account).is_some() || self.minute_spent(&r.account, &r.model)) {
                continue;
            }
            // A model that cannot do what the call needs does not take it, whatever room it has.
            if (wants_tools(body) && !m.caps.tools) || (carries_image(body) && !m.caps.vision) || (m.caps.context > 0 && need > u64::from(m.caps.context)) {
                continue;
            }
            let Ok(other) = self.accounts.target(&r.account, &r.model) else { continue };
            if policy::decide(grant, &other, self.accounts.private_mode()) != Ok(Decision::Send) {
                continue;
            }
            if crate::asked::kept_for_asked(grant, mark, &other).is_some() {
                continue;
            }
            let daily = self.accounts.daily(&r.account, &r.model);
            let today = self.today_of(&other, &daily);
            // An account with no published limit has no day to be spent.
            if !daily.is_none() && (!daily.takes(&today, need) || crate::asked::over_its_share(grant, mark, &other, &daily, &today).is_some()) {
                continue;
            }
            if self.accounts.room(&r.account, &r.model).per_request().is_some_and(|room| need > room) {
                continue;
            }
            if self.resolve_again(grant, &m.id, &other).is_err() {
                continue;
            }
            if same_account {
                return Some((m.id.clone(), other));
            }
            let room = if daily.is_none() { u64::MAX } else { daily.tokens_left(&today).unwrap_or(u64::MAX) };
            fit.push((other.tight, room, order, m.id.clone(), other));
        }
        // Not a tight account (a few requests a minute or a day cannot carry an agent's turn) unless
        // it is the only one; then the most room today; then the picker's order.
        fit.sort_by_key(|(tight, room, order, _, _)| (*tight, std::cmp::Reverse(*room), *order));
        fit.into_iter().next().map(|(_, _, _, id, t)| (id, t))
    }

    /// Whether the requests `account`'s model takes a minute (where its tier publishes them) have
    /// all been sent in the last minute.
    fn minute_spent(&self, account: &str, model: &str) -> bool {
        self.accounts.room(account, model).requests_per_minute.is_some_and(|n| self.holds.requests_last_minute(&format!("{account}/{model}")) >= n)
    }

    /// Just before the send: the answer length fitted to what the day and the context can still
    /// give after `prompt` tokens, then the call counted against the day.
    pub(crate) fn day_fit(&self, grant: &Grant, target: &Target, body: &mut Value, prompt: u64, call: &Call) {
        let daily = self.accounts.daily(&target.account, &target.model);
        let context = self.accounts.room(&target.account, &target.model).context;
        let from_day = daily.tokens_left(&self.today_of(target, &daily)).map(|left| left.saturating_sub(prompt));
        let from_context = context.map(|c| c.saturating_sub(prompt));
        if let Some(ceiling) = [from_day, from_context].into_iter().flatten().min() {
            if let Some((asked, now)) = clamp_answer(body, ceiling, min_answer(body)) {
                tracing::info!(account = %target.account, asked, now, "the answer length asked for was more than the account can give; it was lowered");
            }
        }
        if !daily.is_none() {
            let itself = crate::asked::by_itself(grant, call.asked.as_deref());
            self.holds.day.reserve(call.rid, &daily.key(&target.account, &target.model), daily.end, prompt, itself);
        }
    }

    /// The call is over: its real tokens replace its estimate in the day's ledger; a call that was
    /// not answered gives its estimate back, and a reply that said no usage keeps it.
    pub(crate) fn settle_day(&self, call: &Call) {
        let real = call.prompt_tokens + call.completion_tokens;
        self.holds.day.settle(call.rid, if !call.ok() { Some(0) } else { (real > 0).then_some(real) });
    }

    /// A 413 or 429 reply read at its start for what it says the account counts today, and passed on
    /// as it was ([`Self::learn_from_refusal`]).
    pub(crate) fn learn_from_reply(&self, target: &Target, reply: &mut crate::upstream::Reply, rid: u64) {
        use std::io::Read as _;
        if !matches!(reply.status, 413 | 429) {
            return;
        }
        let mut peek = Vec::new();
        let _ = (&mut reply.body).take(16 * 1024).read_to_end(&mut peek);
        self.learn_from_refusal(target, rid, reply.retry_after, &String::from_utf8_lossy(&peek));
        let rest = std::mem::replace(&mut reply.body, Box::new(std::io::empty()));
        reply.body = Box::new(std::io::Cursor::new(peek).chain(rest));
    }

    /// An account's refusal (429 or 413) that states what it counts today: that is the ledger's
    /// count for it from now on, in place of this gateway's own (a fresh install, calls made outside
    /// the gateway, a day that does not end at our midnight). Numbers only are read from `text`.
    pub(crate) fn learn_from_refusal(&self, target: &Target, rid: u64, retry_after: Option<std::time::Duration>, text: &str) {
        let daily = self.accounts.daily(&target.account, &target.model);
        if daily.is_none() {
            return;
        }
        let used = crate::budget::day_used(text);
        if used.tokens.is_some() || used.requests.is_some() {
            tracing::info!(account = %target.account, tokens = ?used.tokens, requests = ?used.requests, "the account stated what it has counted today; the day's ledger now says the same");
            self.holds.day.learn(rid, &daily.key(&target.account, &target.model), daily.end, used, retry_after);
        }
    }

    /// Each of `account`'s models' use today against its published daily limits; one line for a
    /// limit the whole account shares. For `yos describe shell`.
    pub fn day_lines(&self, account: &str) -> Vec<DayLine> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for m in self.accounts.models() {
            let Some(r) = route::parse(&m.id).filter(|r| r.account == account) else { continue };
            let daily = self.accounts.daily(&r.account, &r.model);
            if daily.is_none() || !seen.insert(daily.key(&r.account, &r.model)) {
                continue;
            }
            let today = self.holds.day.today(&daily.key(&r.account, &r.model), daily.end);
            out.push(DayLine { model: r.model, daily, today });
        }
        out
    }
}

/// The gateway's own words for a call the day cannot take: the account, the limit, the counts and
/// when it frees up.
fn spent(target: &Target, daily: &Daily, today: &Today, need: u64) -> Refusal {
    let when = format!("It frees up at {}, in {}.", daily.end.words(), wait_words(today.resets_in));
    let what = match daily.tokens.filter(|t| today.tokens.saturating_add(need) > *t) {
        Some(limit) => format!(
            "{} has {} of its {} tokens left today and this call needs about {}",
            target.provider,
            short(limit.saturating_sub(today.tokens)),
            short(limit),
            short(need)
        ),
        None => format!("{} has used all {} of today's requests", target.provider, short(daily.requests.unwrap_or(0))),
    };
    Refusal::new(429, DAY_SPENT, format!("{what}, so it was not sent. {when}"))
}
