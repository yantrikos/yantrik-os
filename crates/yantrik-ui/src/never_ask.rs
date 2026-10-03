//! "Approvals off for a test run": a gate mode in which nothing is ever put in front of the person.
//!
//! # Why this exists
//!
//! The Mind gate (`tests/arena/harness_arena.py`) runs scored tasks on a machine where a real
//! person is logged in. A task's `shell.agent_run` needed approval, so it put an approval card in
//! that person's live session; nobody answered and it sat there for about 110 s. A gate must
//! never put anything in front of the person.
//!
//! # What it does
//!
//! While on, every request that would raise an approval card is REFUSED at once, with a reason a
//! mind can read, and no card, notification, phone message or window is made. The one place that
//! makes cards is [`crate::approvals::request`], and it asks [`refusal`] first, so a new caller of
//! it cannot forget.
//!
//! # Why it is safe
//!
//! - **It only removes power.** It turns an "ask" into a "refuse", never into an "allow". An
//!   action the table runs without a card still runs, and the grade and the mode are untouched.
//! - **A mind cannot switch it on or off.** [`may_switch`] lets in only the shell's own account or
//!   root, and never a mind account, an agent token or a process an attached mind started. On
//!   only removes power, but a mind able to re-arm it could silently refuse every other mind's
//!   requests with nothing shown to the person, so both directions take the same door.
//! - **It ends by itself.** It is a deadline, never a bare flag and never written to a file, so
//!   a crashed arena or a restarted shell cannot leave a machine refusing every approval. The
//!   longest it can be set for is [`MAX_MINUTES`], and asking again only moves the deadline
//!   inside that cap from now.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long it lasts when the caller names no time.
pub const DEFAULT_MINUTES: u64 = 30;
/// The most it can be set for in one go. An arena run is minutes; four hours covers a long one.
pub const MAX_MINUTES: u64 = 240;

/// The deadline, and nothing else.
#[derive(Clone, Copy, Debug, Default)]
pub struct NeverAsk {
    until: Option<Instant>,
}

impl NeverAsk {
    /// On for `minutes` (clamped to 1..=[`MAX_MINUTES`]) from `now`. Returns the length used.
    pub fn engage(&mut self, now: Instant, minutes: u64) -> Duration {
        let length = Duration::from_secs(minutes.clamp(1, MAX_MINUTES) * 60);
        self.until = Some(now + length);
        length
    }

    pub fn clear(&mut self) {
        self.until = None;
    }

    /// Derived from the clock on every read, like a bypass: it cannot still be on merely
    /// because no timer happened to fire.
    pub fn active(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| now < until)
    }

    pub fn left(&self, now: Instant) -> Option<Duration> {
        self.until.filter(|until| now < *until).map(|until| until - now)
    }
}

/// What a refused approval says. Read by a mind, so it names the cause and what to do.
pub const REFUSED: &str = "refused: this action needs the person's approval, and approvals are off during a \
     test run, so nothing was shown to them and nothing was run. Do not retry it; \
     choose a step that needs no approval, or say that this one could not be done.";

/// The refusal for one request at `now`, or `None` when asking is allowed.
fn refusal_at(state: &NeverAsk, now: Instant) -> Option<String> {
    state.active(now).then(|| REFUSED.to_string())
}

/// Whether the caller may switch it on or off. Pure, so each way in is a test.
///
/// `caller_uid` is `None` for no socket call at all (a click in the shell itself). Allowed:
/// that, the shell's own account, and root. Never: the mind account, an agent token, or a
/// process an attached mind started (`requester_is_mind`), whatever account it runs as.
pub fn may_switch(
    caller_uid: Option<u32>,
    shell_uid: u32,
    mind_account: bool,
    agent_or_mind_process: bool,
) -> Result<(), String> {
    let no = |why: &str| {
        Err(format!(
            "{why} Only the person's own account or root can switch approvals back on, from the \
             machine's control socket. Nothing was changed; it ends by itself when its time is up."
        ))
    };
    if mind_account || agent_or_mind_process {
        return no("A mind or an agent cannot do that.");
    }
    match caller_uid {
        None => Ok(()),
        Some(uid) if uid == shell_uid || uid == 0 => Ok(()),
        Some(_) => no("That account is neither the person's nor root."),
    }
}

fn state() -> &'static Mutex<NeverAsk> {
    static STATE: OnceLock<Mutex<NeverAsk>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(NeverAsk::default()))
}

fn locked() -> std::sync::MutexGuard<'static, NeverAsk> {
    state().lock().unwrap_or_else(|e| e.into_inner())
}

/// The refusal [`crate::approvals::request`] returns while the mode is on, `None` otherwise.
pub fn refusal() -> Option<String> {
    refusal_at(&locked(), Instant::now())
}

/// Switch it on; returns the minutes it was set for.
pub fn switch_on(minutes: Option<u64>) -> u64 {
    let length = locked().engage(Instant::now(), minutes.unwrap_or(DEFAULT_MINUTES));
    tracing::warn!(minutes = length.as_secs() / 60, "approvals are off for a test run: every approval is refused and no card is shown");
    length.as_secs() / 60
}

/// Switch it off. The caller's right to has been checked by [`may_switch`].
pub fn switch_off() {
    locked().clear();
    tracing::info!("approvals are back on: the test run ended");
}

/// What `describe shell` publishes under `approvals_off_for_test`.
pub fn snapshot() -> serde_json::Value {
    let left = locked().left(Instant::now());
    serde_json::json!({
        "on": left.is_some(),
        "expires_in_secs": left.map(|d| d.as_secs()),
        "means": if left.is_some() { REFUSED } else {
            "Off: approvals work as the mode says. A test run turns this on so that nothing is ever put in front of the person."
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_refuses_while_on_and_says_why() {
        let now = Instant::now();
        let mut s = NeverAsk::default();
        assert!(refusal_at(&s, now).is_none(), "off by default");
        s.engage(now, 5);
        let why = refusal_at(&s, now).expect("on");
        assert!(why.starts_with("refused: this action needs the person's approval"), "{why}");
        assert!(why.contains("approvals are off during a test run"), "{why}");
    }

    #[test]
    fn it_expires_by_itself() {
        let now = Instant::now();
        let mut s = NeverAsk::default();
        s.engage(now, 5);
        assert!(s.active(now + Duration::from_secs(299)));
        assert!(!s.active(now + Duration::from_secs(300)), "the deadline is the end");
        assert!(refusal_at(&s, now + Duration::from_secs(301)).is_none());
        assert_eq!(s.left(now + Duration::from_secs(301)), None);
    }

    #[test]
    fn its_length_is_capped_and_never_zero() {
        let now = Instant::now();
        let mut s = NeverAsk::default();
        assert_eq!(s.engage(now, u64::MAX), Duration::from_secs(MAX_MINUTES * 60));
        assert_eq!(s.engage(now, 0), Duration::from_secs(60), "zero minutes is one, not forever");
    }

    #[test]
    fn a_mind_cannot_switch_it_on_or_off() {
        let me = 1000;
        assert!(may_switch(Some(me), me, true, false).is_err(), "the mind account");
        assert!(may_switch(Some(me), me, false, true).is_err(), "an agent or a mind's process");
        assert!(may_switch(Some(me), me, true, true).is_err());
        assert!(may_switch(Some(4242), me, false, false).is_err(), "some other account");
        assert!(may_switch(Some(me), me, false, false).is_ok(), "the person's own account");
        assert!(may_switch(Some(0), me, false, false).is_ok(), "root");
        assert!(may_switch(None, me, false, false).is_ok(), "the shell itself");
    }

    /// The one rule that keeps the mode from ever loosening anything: it has no path to a
    /// `Run`. Read from the source, so a later edit that lets it answer "allow" fails here.
    #[test]
    fn it_can_only_refuse() {
        let src = include_str!("never_ask.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for word in ["Decision::Run", "approvals::grant", "person_set_mode", "person_add_rule"] {
            assert!(!code.contains(word), "never_ask.rs names `{word}`; it may only refuse");
        }
    }
}
