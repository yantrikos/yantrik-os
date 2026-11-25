//! The day's ledger (#1400): what each account has been sent today, against the limits its tier
//! publishes for a day (tokens and requests, `yantrik_ml::provider::pool::tiers`).
//!
//! It extends what [`crate::hold`] keeps for the minute: the same estimate at the send, replaced by
//! the real `prompt_tokens + completion_tokens` when the reply says them. Per account or per model,
//! as the tier scopes the limit, and reset where the tier resets it (midnight UTC or Pacific).
//! Counts only: a key, a day number and four counts, kept across a restart in a small state file
//! (mode 0600) under the gateway's state directory. No word of any call is in it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The most ledger entries kept: more than a person has accounts and models.
const MAX_ENTRIES: usize = 128;
/// Seconds in a day.
const DAY: i64 = 86_400;
/// Midnight in Los Angeles in winter time, which is when the one tier that resets there (Gemini)
/// has its day end for most of the year. An hour off for the rest is a wait that is a little long
/// or short, never a call let through that the account refuses.
const PACIFIC_OFFSET_SECS: i64 = -8 * 3600;

/// Where a day ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DayEnd {
    #[default]
    UtcMidnight,
    PacificMidnight,
}

impl DayEnd {
    fn offset(self) -> i64 {
        match self {
            DayEnd::UtcMidnight => 0,
            DayEnd::PacificMidnight => PACIFIC_OFFSET_SECS,
        }
    }

    /// The number of the day `secs` (since the epoch) falls in.
    fn day_of(self, secs: i64) -> i64 {
        (secs + self.offset()).div_euclid(DAY)
    }

    /// How long until this day ends.
    fn until_end(self, secs: i64) -> Duration {
        let end = (self.day_of(secs) + 1) * DAY - self.offset();
        Duration::from_secs((end - secs).max(0) as u64)
    }

    /// "00:00 UTC", as the person reads when it frees up.
    pub fn words(self) -> &'static str {
        match self {
            DayEnd::UtcMidnight => "00:00 UTC",
            DayEnd::PacificMidnight => "midnight Pacific time",
        }
    }
}

/// An account's published limits for a day, for one model: each `None` where the tier publishes
/// none. Filled by [`crate::server::Accounts::daily`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Daily {
    pub tokens: Option<u64>,
    pub requests: Option<u64>,
    /// The limit is shared by every model of the account (otherwise each model has its own).
    pub per_account: bool,
    pub end: DayEnd,
}

impl Daily {
    pub fn is_none(&self) -> bool {
        self.tokens.is_none() && self.requests.is_none()
    }

    /// The ledger's key for `account`/`model`.
    pub fn key(&self, account: &str, model: &str) -> String {
        if self.per_account { account.to_string() } else { format!("{account}/{model}") }
    }

    /// Whether one more call of about `need` tokens fits what is left of `today`.
    pub fn takes(&self, today: &Today, need: u64) -> bool {
        self.tokens.is_none_or(|t| today.tokens.saturating_add(need) <= t) && self.requests.is_none_or(|r| today.requests < r)
    }

    /// The tokens still to be had today, when the tier publishes a limit.
    pub fn tokens_left(&self, today: &Today) -> Option<u64> {
        self.tokens.map(|t| t.saturating_sub(today.tokens))
    }
}

/// What has been sent today, as counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Today {
    pub tokens: u64,
    pub requests: u64,
    /// The part of both that the companion started by itself.
    pub itself_tokens: u64,
    pub itself_requests: u64,
    /// How long until the day ends.
    pub resets_in: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Count {
    day: i64,
    tokens: u64,
    requests: u64,
    #[serde(default)]
    itself_tokens: u64,
    #[serde(default)]
    itself_requests: u64,
    /// Seconds since the epoch until which this count is kept past its day's end: the account said
    /// (in a refusal) when its own count frees up, which is not always midnight.
    #[serde(default)]
    held_until: i64,
}

/// One call's estimate in the ledger, until its real count replaces it.
struct Pending {
    key: String,
    day: i64,
    tokens: u64,
    itself: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    counts: HashMap<String, Count>,
}

type Wall = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct Budget {
    path: Option<PathBuf>,
    wall: Wall,
    counts: Mutex<HashMap<String, Count>>,
    /// Calls sent and not yet answered, by the call's own key ([`crate::log::Call::rid`]), so
    /// calls that overlap each take off their own estimate and no other's.
    pending: Mutex<HashMap<u64, Pending>>,
}

fn system_wall() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Default for Budget {
    fn default() -> Self {
        Budget::in_memory()
    }
}

impl Budget {
    pub fn in_memory() -> Budget {
        Budget { path: None, wall: Arc::new(system_wall), counts: Mutex::new(HashMap::new()), pending: Mutex::new(HashMap::new()) }
    }

    /// The ledger kept at `path`, with what it held read back. A file that is missing or not ours
    /// starts the day empty: the provider's own refusal is the backstop.
    pub fn at(path: PathBuf) -> Budget {
        Budget::at_with(path, Arc::new(system_wall))
    }

    pub fn at_with(path: PathBuf, wall: Wall) -> Budget {
        let counts = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<State>(&t).ok()).map(|s| s.counts).unwrap_or_default();
        Budget { path: Some(path), wall, counts: Mutex::new(counts), pending: Mutex::new(HashMap::new()) }
    }

    /// What `key` has been sent today.
    pub fn today(&self, key: &str, end: DayEnd) -> Today {
        let now = (self.wall)();
        let counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        match counts.get(key).filter(|c| c.day == end.day_of(now) || now < c.held_until) {
            Some(c) => {
                let resets_in = if now < c.held_until && c.day != end.day_of(now) { Duration::from_secs((c.held_until - now) as u64) } else { end.until_end(now) };
                Today { tokens: c.tokens, requests: c.requests, itself_tokens: c.itself_tokens, itself_requests: c.itself_requests, resets_in }
            }
            None => Today { resets_in: end.until_end(now), ..Today::default() },
        }
    }

    /// The count for `key` as of now, a new day's if its day has ended and the account has not said
    /// it still counts the old one.
    fn current<'a>(&self, counts: &'a mut HashMap<String, Count>, key: &str, end: DayEnd) -> Option<&'a mut Count> {
        let now = (self.wall)();
        let day = end.day_of(now);
        counts.retain(|_, c| c.day >= day - 1);
        if counts.len() >= MAX_ENTRIES && !counts.contains_key(key) {
            return None;
        }
        let c = counts.entry(key.to_string()).or_default();
        if c.day != day && now >= c.held_until {
            *c = Count { day, ..Count::default() };
        }
        Some(c)
    }

    /// Call `rid`, of about `tokens`, is being sent to `key`.
    pub fn reserve(&self, rid: u64, key: &str, end: DayEnd, tokens: u64, itself: bool) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = self.current(&mut counts, key, end) else { return };
        c.requests += 1;
        c.tokens += tokens;
        if itself {
            c.itself_requests += 1;
            c.itself_tokens += tokens;
        }
        let day = end.day_of((self.wall)());
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        // A call that never settled (the process was cut) leaves nothing behind for long.
        pending.retain(|_, p| p.day >= day - 1);
        pending.insert(rid, Pending { key: key.to_string(), day, tokens, itself });
        self.save(&counts);
    }

    /// Call `rid` is over: its estimate is replaced by `real` tokens (0 for a call that was refused
    /// and so used none), or kept when the reply gave no count (`None`). A call that was never
    /// reserved changes nothing.
    pub fn settle(&self, rid: u64, real: Option<u64>) {
        let Some(p) = self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&rid) else { return };
        let Some(real) = real else { return };
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = counts.get_mut(&p.key).filter(|c| c.day == p.day) else { return };
        c.tokens = c.tokens.saturating_sub(p.tokens) + real;
        if p.itself {
            c.itself_tokens = c.itself_tokens.saturating_sub(p.tokens) + real;
        }
        self.save(&counts);
    }

    /// The account said what it counts for `key` today (a refusal's "Used"): that is the count, in
    /// place of this ledger's own; `rid` is the refused call. `hold` is how long it said until it frees up, when it named it:
    /// the count is then kept past midnight until then, for an account whose day is not ours.
    pub fn learn(&self, rid: u64, key: &str, end: DayEnd, used: DayUsed, hold: Option<Duration>) {
        if used.tokens.is_none() && used.requests.is_none() {
            return;
        }
        // The account's count has no part of call `rid` in it (it was refused): its estimate is not
        // taken off again when it settles.
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&rid);
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = self.current(&mut counts, key, end) else { return };
        if let Some(t) = used.tokens {
            c.tokens = t;
            c.itself_tokens = c.itself_tokens.min(t);
        }
        if let Some(r) = used.requests {
            c.requests = r;
            c.itself_requests = c.itself_requests.min(r);
        }
        if let Some(h) = hold {
            c.held_until = c.held_until.max((self.wall)() + h.as_secs() as i64);
        }
        self.save(&counts);
    }

    /// Counts only, written whole and renamed into place, readable by the person alone.
    fn save(&self, counts: &HashMap<String, Count>) {
        let Some(path) = &self.path else { return };
        let write = || -> std::io::Result<()> {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let text = serde_json::to_vec(&State { counts: counts.clone() }).map_err(std::io::Error::other)?;
            let tmp = path.with_extension("tmp");
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp)?;
            f.write_all(&text)?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, "the gateway's day ledger was not written");
        }
    }
}

/// A count as the person reads it: 4,193, or 150K from ten thousand up.
pub fn short(n: u64) -> String {
    if n >= 10_000 {
        return format!("{}K", (n + 500) / 1000);
    }
    if n >= 1000 { format!("{},{:03}", n / 1000, n % 1000) } else { n.to_string() }
}

/// A wait as the person reads it: "5 h 12 min", "12 min", "under a minute".
pub fn wait_words(d: Duration) -> String {
    let mins = d.as_secs().div_ceil(60);
    match (mins / 60, mins % 60) {
        (0, 0) => "under a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// What an account's refusal says it counts today: its own "Used", as numbers only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DayUsed {
    pub tokens: Option<u64>,
    pub requests: Option<u64>,
}

/// The day's counts a refusal states, read the way [`crate::fit::counted_by`] reads a 413: Groq's
/// "... on tokens per day (TPD): Limit 200000, Used 196699, Requested 4193" (and "requests per day
/// (RPD)"), OpenAI's the same sentence. A per-minute limit is not a day and is not read. Numbers
/// only: nothing else of the body is kept.
pub fn day_used(body: &str) -> DayUsed {
    let used_after = |markers: &[&str]| -> Option<u64> {
        markers.iter().find_map(|m| {
            let rest = &body[body.find(m)? + m.len()..];
            // The sentence that names the limit ends at the next full stop and space.
            let sentence = rest.split(". ").next().unwrap_or(rest);
            let after = &sentence[sentence.find("Used ")? + "Used ".len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == ',').filter(|c| *c != ',').collect();
            digits.parse().ok()
        })
    };
    DayUsed { tokens: used_after(&["(TPD)", "tokens per day"]), requests: used_after(&["(RPD)", "requests per day"]) }
}
