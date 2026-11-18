//! Everything this app does that leaves its process: the mail service, and the browser.
//!
//! The app talks to `email-service` over JSON-RPC and starts it first: the service is registered
//! `autostart: false`, nothing ever started it, and every call this app made failed at connect on
//! every machine. The app read that failure as "no account configured" and drew the onboarding
//! form, which is why an audit of this machine concluded it had no mail account. It may never
//! have been about an account at all. `design/email-2026-09-20.md` has the whole of it.
//!
//! Three things follow from that, and they are the shape of this module:
//!
//! * every service call goes through [`yantrik_ipc_transport::service::client`], which asks the
//!   shell to start the service and clears the transport's circuit breaker;
//! * every wrapper returns `Result<T, String>`, because an `Option` is where the reason was lost;
//! * the app distinguishes three states — the service could not be reached, the service is up and
//!   no account is configured, an account exists — and says which, on screen and in `describe`.
//!
//! `frameworks/yantrik-services` has no mail client, so the call stays the one it always was and
//! only moves: it runs in a job now, never on the UI thread. It sits behind [`MailService`] so
//! the tests drive the app against a scripted service ([`fake`]) instead of a socket, and no test
//! reaches a real mail server or opens a real browser.

use std::sync::Arc;
use std::time::Duration;

use yantrik_app::Paths;

mod browser;
pub mod calls;
pub mod look;

#[cfg(test)]
pub mod fake;

/// How long a call that crosses the internet may take.
///
/// `SyncRpcClient`'s default is two seconds, which is right for a local service answering out of
/// memory and wrong for every method here: an IMAP round trip leaves the machine. Under the
/// default, a mailbox that was working perfectly reported itself unreachable.
///
/// Ten seconds is a compromise and worth naming as one. Every call is made from a job, so a
/// slow mailbox keeps the window drawing while it answers.
pub const MAIL_BUDGET: Duration = Duration::from_secs(10);

/// What this app asks of the world outside it. Blocking: from a job only.
pub trait MailService: Send + Sync {
    /// One call to the mail service. The error is the service's own sentence, or the
    /// transport's when there was no answer.
    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String>;

    /// Put a page in front of the person, in their browser.
    fn open_in_browser(&self, url: &str) -> Result<(), String>;
}

/// The live service, over the socket bus, and the desktop's `xdg-open`. (A test's app talks to
/// [`fake`] instead.)
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, allow(dead_code))]
pub struct Live;

impl MailService for Live {
    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        // A client for the mail service, started first if it was not running.
        yantrik_ipc_transport::service::client("email")?
            .with_timeout(MAIL_BUDGET)
            .call(method, params)
            .map_err(|e| e.message)
    }

    fn open_in_browser(&self, url: &str) -> Result<(), String> {
        browser::open(url)
    }
}

/// The service this app talks to: the live one, or under a test the fake that test installed for
/// these paths (an unreachable one when it installed none).
#[cfg(not(test))]
pub fn for_app(_paths: &Paths) -> Arc<dyn MailService> {
    Arc::new(Live)
}

#[cfg(test)]
pub fn for_app(paths: &Paths) -> Arc<dyn MailService> {
    fake::for_app(paths)
}
