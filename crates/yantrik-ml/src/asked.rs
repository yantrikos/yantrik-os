//! Who started a call: a person (or an agent), or the companion by itself.
//!
//! A free account whose allowance is tight (a few requests a minute) is kept for what somebody
//! asked: a message, a button, a proof, a recipe the person started. What the companion starts on
//! its own (learning from a finished conversation, its reflection, the narrative refresh, an urge
//! from the think timer, the startup brief) is not sent there.
//!
//! The default is **asked**. Only a path that is the companion's own wears the mark:
//! [`self_started`] at its top, undone when the guard drops. The mark is per thread, so a thread
//! that carries on a self-started path takes it with [`carry`] and [`adopt`].
//!
//! The mark reaches the gateway as a header ([`HEADER`]), and only when the client is the
//! gateway's own ([`stamp`]): a provider never sees it. The gateway refuses a marked call from the
//! built-in companion to a tight account with [`KEPT_CODE`]; a client that gets that answer is told
//! so ([`KeptForAsked`]), and it is not an outage. Before starting such a path the companion asks
//! [`skip_self_started`], so the refusal is the rare race, not the rule.

use std::cell::Cell;
use std::sync::{Mutex, OnceLock};

/// The header the gateway reads.
pub const HEADER: &str = "x-yantrik-asked";
/// Its value for a call the companion started itself. No other value means anything.
pub const BY_ITSELF: &str = "self";
/// The gateway's refusal code for such a call to a tight account.
pub const KEPT_CODE: &str = "kept_for_asked";

thread_local! {
    static SELF_STARTED: Cell<bool> = const { Cell::new(false) };
    static TIGHT_HERE: Cell<Option<Wait>> = const { Cell::new(None) };
}

/// Whether this thread is running something the companion started by itself.
pub fn started_by_itself() -> bool {
    SELF_STARTED.with(Cell::get)
}

/// The mark, set for as long as this lives; the earlier value comes back on drop.
#[must_use = "the mark ends when the guard is dropped"]
pub struct SelfStarted {
    before: bool,
}

impl Drop for SelfStarted {
    fn drop(&mut self) {
        SELF_STARTED.with(|m| m.set(self.before));
    }
}

/// Mark this thread's calls as the companion's own until the guard drops.
pub fn self_started() -> SelfStarted {
    adopt(true)
}

/// The mark as this thread has it, to hand to a thread it starts ([`adopt`]).
pub fn carry() -> bool {
    started_by_itself()
}

/// Set this thread's mark to what another thread [`carry`]ed, until the guard drops.
pub fn adopt(carried: bool) -> SelfStarted {
    SelfStarted { before: SELF_STARTED.with(|m| m.replace(carried)) }
}

/// Why self-started work should wait now, as the shell answers it from the gateway's "now".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// Nothing: start it.
    No,
    /// The person has a turn open: paused while they are being answered.
    TurnOpen,
    /// The account its calls go to is tight and kept for what is asked.
    Tight,
    /// The companion's key has no room this minute.
    NoRoom,
}

/// What the shell installs: whether self-started work should wait now.
static CHECK: OnceLock<fn() -> Wait> = OnceLock::new();
/// Work skipped because a turn was open, by name, run once when the turn closes ([`turn_closed`]).
static LATER: Mutex<Vec<(String, Box<dyn FnOnce() + Send>)>> = Mutex::new(Vec::new());

/// Install the shell's answer to "should self-started work wait now". Once; a second is ignored.
pub fn on_wait_check(check: fn() -> Wait) {
    let _ = CHECK.set(check);
}

/// The answer for this thread: its own when one was set ([`tight_for_this_thread`],
/// [`wait_for_this_thread`]), else the shell's, else no wait.
pub fn wait_now() -> Wait {
    TIGHT_HERE.with(Cell::get).unwrap_or_else(|| CHECK.get().map_or(Wait::No, |check| check()))
}

/// Whether what the companion starts by itself should wait now: a turn is open, or its key has
/// no room, or its account is tight.
pub fn keeps_for_asked() -> bool {
    wait_now() != Wait::No
}

/// Ask before the companion starts something by itself: `true` means skip it. Says so, at debug
/// level, and makes no call.
pub fn skip_self_started(what: &str) -> bool {
    skip_self_started_then(what, || {})
}

/// [`skip_self_started`], and when the work was skipped because a turn is open, `later` runs once
/// when the turn closes (one wake-up on close, not a poll). A second skip of the same `what`
/// replaces the first.
pub fn skip_self_started_then(what: &str, later: impl FnOnce() + Send + 'static) -> bool {
    let wait = wait_now();
    match wait {
        Wait::No => return false,
        Wait::TurnOpen => {
            tracing::debug!(what, "paused while you're being answered");
            let mut l = LATER.lock().unwrap_or_else(|e| e.into_inner());
            l.retain(|(w, _)| w != what);
            l.push((what.to_string(), Box::new(later)));
        }
        Wait::Tight | Wait::NoRoom => tracing::debug!(what, "not started: this account allows few requests and is kept for what you ask"),
    }
    true
}

/// The names of the work paused for an open turn, for Mind View.
pub fn paused() -> Vec<String> {
    LATER.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(w, _)| w.clone()).collect()
}

/// The last open turn closed: run what was paused for it, each once.
pub fn turn_closed() {
    let paused = std::mem::take(&mut *LATER.lock().unwrap_or_else(|e| e.into_inner()));
    for (_, later) in paused {
        later();
    }
}

/// What [`tight_for_this_thread`] puts back.
pub struct TightHere {
    before: Option<Wait>,
}

impl Drop for TightHere {
    fn drop(&mut self) {
        TIGHT_HERE.with(|t| t.set(self.before));
    }
}

/// Answer [`skip_self_started`] on this thread alone, until the guard drops: for the tests of what
/// is kept back, which must not depend on (or change) another thread's account.
pub fn tight_for_this_thread(tight: bool) -> TightHere {
    wait_for_this_thread(if tight { Wait::Tight } else { Wait::No })
}

/// [`tight_for_this_thread`] for any [`Wait`].
pub fn wait_for_this_thread(wait: Wait) -> TightHere {
    TightHere { before: TIGHT_HERE.with(|t| t.replace(Some(wait))) }
}

/// The mark as a header for a client: only the gateway's own client sends one, and only while the
/// mark is set.
pub fn header_for(gateway: bool) -> Option<(&'static str, &'static str)> {
    (gateway && started_by_itself()).then_some((HEADER, BY_ITSELF))
}

/// `req` with the mark on it when [`header_for`] says so. The one place a request is marked.
#[cfg(feature = "api-llm")]
pub fn stamp(
    req: ureq::RequestBuilder<ureq::typestate::WithBody>,
    gateway: bool,
) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
    match header_for(gateway) {
        Some((name, value)) => req.header(name, value),
        None => req,
    }
}

/// The gateway kept the call back: a call the companion started by itself, to an account that
/// allows few requests. Nothing was sent to the provider, and the model was reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptForAsked {
    /// The gateway's own sentence.
    pub said: String,
}

impl std::fmt::Display for KeptForAsked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.said)
    }
}

impl std::error::Error for KeptForAsked {}

/// The refusal in a gateway's answer, if this is it: a 429 whose error code is [`KEPT_CODE`].
pub fn kept(status: u16, body: &str) -> Option<KeptForAsked> {
    if status != 429 {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    if json["error"]["code"].as_str() != Some(KEPT_CODE) {
        return None;
    }
    let said = crate::refusal::provider_message(body).unwrap_or_else(|| "This account is kept for what you ask.".to_string());
    Some(KeptForAsked { said })
}

/// The sentence for an error that is [`KeptForAsked`], else `None`: what a caller that tells the
/// person what a failure was reads, beside the provider's own refusals.
pub fn kept_said(e: &(dyn std::error::Error + 'static)) -> Option<String> {
    e.downcast_ref::<KeptForAsked>().map(|k| k.said.clone())
}

#[cfg(test)]
mod tests;
#[cfg(all(test, feature = "api-llm"))]
mod header_tests;
