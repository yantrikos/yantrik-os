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
use std::sync::OnceLock;

/// The header the gateway reads.
pub const HEADER: &str = "x-yantrik-asked";
/// Its value for a call the companion started itself. No other value means anything.
pub const BY_ITSELF: &str = "self";
/// The gateway's refusal code for such a call to a tight account.
pub const KEPT_CODE: &str = "kept_for_asked";

thread_local! {
    static SELF_STARTED: Cell<bool> = const { Cell::new(false) };
    static TIGHT_HERE: Cell<Option<bool>> = const { Cell::new(None) };
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

/// What the shell installs: whether the account the companion's calls go to is tight now.
static TIGHT: OnceLock<fn() -> bool> = OnceLock::new();

/// Install the shell's answer to "is the companion's account tight". Once; a second is ignored.
pub fn on_tight_check(check: fn() -> bool) {
    let _ = TIGHT.set(check);
}

/// Whether the account is tight, for this thread: its own answer when one was set
/// ([`tight_for_this_thread`]), else the shell's, else not tight.
fn tight_now() -> bool {
    TIGHT_HERE.with(Cell::get).unwrap_or_else(|| TIGHT.get().is_some_and(|check| check()))
}

/// Whether what the companion starts by itself would be refused now: its account is tight.
pub fn keeps_for_asked() -> bool {
    tight_now()
}

/// Ask before the companion starts something by itself: `true` means skip it, because a call it
/// started would be refused (the account is tight and is kept for what is asked). Says so, at
/// debug level, and makes no call.
pub fn skip_self_started(what: &str) -> bool {
    let skip = keeps_for_asked();
    if skip {
        tracing::debug!(what, "not started: this account allows few requests and is kept for what you ask");
    }
    skip
}

/// What [`tight_for_this_thread`] puts back.
pub struct TightHere {
    before: Option<bool>,
}

impl Drop for TightHere {
    fn drop(&mut self) {
        TIGHT_HERE.with(|t| t.set(self.before));
    }
}

/// Answer [`skip_self_started`] on this thread alone, until the guard drops: for the tests of what
/// is kept back, which must not depend on (or change) another thread's account.
pub fn tight_for_this_thread(tight: bool) -> TightHere {
    TightHere { before: TIGHT_HERE.with(|t| t.replace(Some(tight))) }
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
