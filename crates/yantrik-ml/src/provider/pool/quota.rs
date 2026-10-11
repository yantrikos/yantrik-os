//! How much of each free tier is left, from three sources, the strictest winning:
//! 1. the published limits (`tiers`), counted here per window and reset by the provider's rule;
//! 2. what the provider's own headers said on its last answer (`headers::Observed`);
//! 3. its refusals: a 429 cools it down for as long as it asked (`retry-after`), else for a
//!    backoff that doubles; repeated errors open a breaker; a refused key or a missing model
//!    takes that provider or model out until it is set up again.

use std::collections::{HashMap, VecDeque};

use super::headers::Observed;
use super::tiers::{Counts, FreeModel, FreeTier, Limit, Reset, Scope};

/// What one call came back with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// HTTP status; 0 for no answer at all (timeout, refused connection).
    pub status: u16,
    pub observed: Observed,
    /// Tokens the call used, prompt and completion, when known.
    pub tokens: u64,
}

/// Why a provider or model is not being used right now.
#[derive(Clone, Debug, PartialEq)]
pub enum Blocked {
    /// Until this time (unix seconds): a refusal, a full window or an open breaker.
    Until(i64),
    /// Its key was refused: not until it is set up again.
    KeyRefused,
    /// The provider no longer serves this model.
    ModelGone,
}

#[derive(Default)]
struct Window {
    /// Rolling windows: when each use happened and how much it was.
    events: VecDeque<(i64, u64)>,
    /// Calendar windows: which day, and how much of it is used.
    day: i64,
    used: u64,
}

impl Window {
    fn used(&mut self, reset: Reset, now: i64) -> u64 {
        match reset {
            Reset::Rolling(secs) => {
                while self.events.front().is_some_and(|(t, _)| now - t >= secs as i64) {
                    self.events.pop_front();
                }
                self.events.iter().fold(0u64, |sum, (_, n)| sum.saturating_add(*n))
            }
            _ => {
                let today = day_of(reset, now);
                if self.day != today {
                    self.day = today;
                    self.used = 0;
                }
                self.used
            }
        }
    }

    fn add(&mut self, reset: Reset, now: i64, n: u64) {
        self.used(reset, now);
        match reset {
            Reset::Rolling(_) => self.events.push_back((now, n)),
            _ => self.used = self.used.saturating_add(n),
        }
    }

    /// When enough of the window clears to allow one more.
    fn frees_at(&self, reset: Reset, now: i64) -> i64 {
        match reset {
            Reset::Rolling(secs) => self.events.front().map_or(now, |(t, _)| t + secs as i64),
            _ => next_day_start(reset, now),
        }
    }
}

/// The quota state of every provider and model the pool has used.
#[derive(Default)]
pub struct Quota {
    windows: HashMap<String, Window>,
    cooldown: HashMap<String, i64>,
    strikes: HashMap<String, u32>,
    /// Remaining requests a provider's headers reported, and until when that report holds.
    told: HashMap<String, (u64, i64)>,
    refused_keys: Vec<String>,
    gone: HashMap<String, i64>,
}

fn key(tier: &FreeTier, model: &FreeModel) -> String {
    format!("{}|{}", tier.id, model.id)
}

fn window_key(tier: &FreeTier, model: &FreeModel, i: usize, limit: &Limit) -> String {
    match limit.scope {
        Scope::PerModel => format!("{}|{}|{i}", tier.id, model.id),
        Scope::PerAccount => format!("{}||{i}", tier.id),
    }
}

impl Quota {
    /// The share of the tightest limit still free (0..=1), or why the model cannot be used now.
    pub fn headroom(&mut self, tier: &FreeTier, model: &FreeModel, limits: &[Limit], now: i64) -> Result<f64, Blocked> {
        if self.refused_keys.iter().any(|k| k == tier.id) {
            return Err(Blocked::KeyRefused);
        }
        let k = key(tier, model);
        if self.gone.get(&k).is_some_and(|until| *until > now) {
            return Err(Blocked::ModelGone);
        }
        for c in [k.as_str(), tier.id] {
            if let Some(until) = self.cooldown.get(c).copied().filter(|u| *u > now) {
                return Err(Blocked::Until(until));
            }
        }
        let mut share: f64 = 1.0;
        for (i, limit) in limits.iter().enumerate() {
            let w = self.windows.entry(window_key(tier, model, i, limit)).or_default();
            let used = w.used(limit.reset, now);
            if used >= limit.amount {
                return Err(Blocked::Until(w.frees_at(limit.reset, now)));
            }
            share = share.min((limit.amount - used) as f64 / limit.amount as f64);
        }
        if let Some((left, until)) = self.told.get(&k).copied().filter(|(_, until)| *until > now) {
            if left == 0 {
                return Err(Blocked::Until(until));
            }
        }
        Ok(share)
    }

    /// Count one call and learn from how it went.
    pub fn record(&mut self, tier: &FreeTier, model: &FreeModel, limits: &[Limit], now: i64, out: &Outcome) {
        let k = key(tier, model);
        if out.status != 401 && out.status != 403 && out.status != 0 {
            for (i, limit) in limits.iter().enumerate() {
                let n = match limit.counts {
                    Counts::Requests => 1,
                    Counts::Tokens => out.tokens,
                };
                self.windows.entry(window_key(tier, model, i, limit)).or_default().add(limit.reset, now, n);
            }
        }
        if let Some(left) = out.observed.remaining_requests {
            let holds = out.observed.reset_requests.map_or(60, secs).max(1);
            self.told.insert(k.clone(), (left, now.saturating_add(holds)));
        }
        match out.status {
            200..=299 => {
                self.strikes.remove(&k);
                self.strikes.remove(tier.id);
            }
            429 => {
                let strikes = self.strikes.entry(k.clone()).or_insert(0);
                *strikes += 1;
                let wait = out
                    .observed
                    .retry_after
                    .or(out.observed.reset_requests)
                    .map(secs)
                    .unwrap_or_else(|| backoff(*strikes));
                self.cooldown.insert(k, now.saturating_add(wait.max(1)));
            }
            401 | 403 => {
                if !self.refused_keys.iter().any(|r| r == tier.id) {
                    self.refused_keys.push(tier.id.to_string());
                }
            }
            404 | 410 => {
                self.gone.insert(k, now + 86_400);
            }
            _ => {
                // No answer, or a server error: three in a row open the breaker for this provider.
                let strikes = self.strikes.entry(tier.id.to_string()).or_insert(0);
                *strikes += 1;
                if *strikes >= 3 {
                    let wait = (300_i64 << (*strikes - 3).min(4)).min(3_600);
                    self.cooldown.insert(tier.id.to_string(), now.saturating_add(wait));
                }
            }
        }
    }

    /// Used this window, against the limit: for the status line ("412 of 1,000 today").
    pub fn used(&mut self, tier: &FreeTier, model: &FreeModel, limit_index: usize, limit: &Limit, now: i64) -> u64 {
        self.windows.entry(window_key(tier, model, limit_index, limit)).or_default().used(limit.reset, now)
    }

    /// The provider was set up again (a new key): forget that its old one was refused.
    pub fn forget_refusal(&mut self, provider: &str) {
        self.refused_keys.retain(|k| k != provider);
    }
}

/// 60 s, doubling with each refusal in a row, at most an hour: for a provider that refuses
/// without saying for how long.
fn backoff(strikes: u32) -> i64 {
    (60_i64 << strikes.saturating_sub(1).min(6)).min(3_600)
}

// ── Calendar days, for the daily windows ──

/// The day number `now` falls in, by the reset's clock.
pub fn day_of(reset: Reset, now: i64) -> i64 {
    match reset {
        Reset::PacificMidnight => (now + pacific_offset(now)).div_euclid(86_400),
        _ => now.div_euclid(86_400),
    }
}

/// When the next day starts by the reset's clock, in unix seconds.
pub fn next_day_start(reset: Reset, now: i64) -> i64 {
    match reset {
        Reset::PacificMidnight => {
            let local_next = (day_of(reset, now) + 1) * 86_400;
            // The offset at that moment (a DST change happens at 2 a.m., never at midnight).
            local_next - pacific_offset(local_next - pacific_offset(now))
        }
        _ => (now.div_euclid(86_400) + 1) * 86_400,
    }
}

/// Los Angeles' offset from UTC at `now`: -7 h from the second Sunday of March (2 a.m. local)
/// to the first Sunday of November (2 a.m. local), else -8 h. The US rule since 2007.
pub fn pacific_offset(now: i64) -> i64 {
    let (year, _, _) = civil(now.div_euclid(86_400));
    let start = nth_sunday(year, 3, 2) * 86_400 + 10 * 3_600; // 02:00 PST = 10:00 UTC
    let end = nth_sunday(year, 11, 1) * 86_400 + 9 * 3_600; // 02:00 PDT = 09:00 UTC
    if now >= start && now < end {
        -7 * 3_600
    } else {
        -8 * 3_600
    }
}

/// The day number of the n-th Sunday of a month.
fn nth_sunday(year: i64, month: u32, n: i64) -> i64 {
    let first = days_from_civil(year, month, 1);
    let weekday = (first + 4).rem_euclid(7); // 1970-01-01 was a Thursday (4); 0 = Sunday
    first + (7 - weekday) % 7 + (n - 1) * 7
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day number.
pub(super) fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A wait in whole seconds, never more than a day and never negative, whatever it was built from
/// (a header is bounded when parsed; this holds for every other source too).
fn secs(s: f64) -> i64 {
    s.min(super::headers::MAX_WAIT_SECS).max(0.0).ceil() as i64
}
