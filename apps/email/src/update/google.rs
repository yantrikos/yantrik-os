//! The Google sign-in, on this side of the wire.
//!
//! There is a person in the middle of it: they go to a browser and take as long as they take.
//! Everything slow is in a job. `oauth_begin` is a socket bind and answers at once, but it is
//! still a service call, and opening a browser is a process spawn with a bounded wait on it. The
//! mail service is then asked once a second where the sign-in has got to, from a timer, until it
//! stops saying "waiting"; meanwhile the window says so, and offers Cancel.

use std::time::Duration;

use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;
use yantrik_ipc_contracts::email::OAuthBeginResult;

use super::{clear_notice, say};
use crate::backend::calls;
use crate::command::{Command, Look, Work};
use crate::model::{part, Clear, Flow, Model};
use crate::state::{self, GoogleOutcome};
use crate::Email;

/// The poll that asks where a sign-in has got to.
pub const POLL: TimerId = TimerId("email.google");
pub const POLL_EVERY: Duration = Duration::from_secs(1);

/// How much longer than the service's own deadline this window waits, so the service's sentence
/// about giving up is the one that is shown rather than this app's.
const GRACE_POLLS: u64 = 20;

/// Start a sign-in: ask the service, open a browser, and leave a poll running.
pub fn sign_in(m: &mut Model, fx: &mut Effects<Email>) -> Result<(), Refused> {
    let google = m.state.google_sign_in();
    let refusal = if m.setup.busy || m.google.flow.is_some() {
        Some("A sign-in is already in progress.".to_string())
    } else if !google.available {
        // The reason is already on the screen, from `email.accounts`; a caller of the action
        // gets the same sentence a person is reading.
        Some(google.note)
    } else {
        None
    };
    if let Some(e) = refusal {
        if fx.origin() != Origin::Local {
            return Err(Refused::new("not_started", e));
        }
        // Said again here so a refusal from a click is never silent.
        m.setup.ok = false;
        m.setup.status = e.clone();
        fx.changed(part::SETUP);
        say(m, fx, e);
        return Ok(());
    }
    m.setup.busy = true;
    m.google.waiting = true;
    m.google.url.clear();
    m.setup.ok = false;
    m.setup.status = "Opening the Google sign-in\u{2026}".into();
    fx.changed(part::SETUP);
    let mail = m.mail.clone();
    fx.spawn_keyed_or(
        JobKey { feature: "google", scope: Scope::App },
        Concurrency::RejectIfRunning,
        Command::Unfinished(Work::Google),
        move |_| {
            let begun = calls::oauth_begin(mail.as_ref());
            // The browser is opened here rather than after the hop back, so the window is never
            // held waiting on a process spawn.
            let opened = begun.as_ref().ok().map(|b| mail.open_in_browser(&b.auth_url));
            Command::GoogleBegun { begun, opened }
        },
    );
    Ok(())
}

pub fn begun(m: &mut Model, begun: Result<OAuthBeginResult, String>, opened: Option<Result<(), String>>, fx: &mut Effects<Email>) {
    m.setup.busy = false;
    fx.changed(part::SETUP);
    // Cancelled (or the form opened afresh) before the start landed: the flow it opened is ended
    // at the service, so no listener is left open and nothing polls.
    if !m.google.waiting {
        if let Ok(begun) = begun {
            let mail = m.mail.clone();
            fx.spawn(move |_| Command::GoogleCancelled(calls::oauth_cancel(mail.as_ref(), &begun.flow_id)));
        }
        return;
    }
    let begun = match begun {
        Ok(begun) => begun,
        Err(e) => {
            m.google.waiting = false;
            let text = format!("The Google sign-in could not be started: {e}");
            m.setup.status = text.clone();
            return say(m, fx, text);
        }
    };
    m.google.flow = Some(Flow { id: begun.flow_id.clone(), polls_left: begun.expires_in_secs + GRACE_POLLS });
    match opened {
        Some(Ok(())) => {
            m.google.url.clear();
            m.setup.status = "Waiting for Google\u{2026}".into();
            clear_notice(m, fx);
        }
        Some(Err(why)) => {
            // Not a dead end: the address goes on the screen so the sign-in can be finished from
            // anywhere with a browser.
            let text = state::browser_failed_note(&why);
            m.google.url = begun.auth_url.clone();
            m.setup.status = text.clone();
            say(m, fx, text);
        }
        None => {}
    }
    fx.every(POLL, POLL_EVERY, Command::GooglePoll);
}

/// One second gone: ask where the sign-in is, or stop waiting once the deadline has passed.
pub fn poll(m: &mut Model, fx: &mut Effects<Email>) {
    let Some(flow) = m.google.flow.as_mut() else {
        return fx.stop_timer(POLL);
    };
    flow.polls_left = flow.polls_left.saturating_sub(1);
    let id = flow.id.clone();
    if flow.polls_left == 0 {
        let reason = "The Google sign-in was not finished in time, so this window stopped waiting for it. Nothing was saved.";
        return finish(m, &id, GoogleOutcome::Stopped(reason.to_string()), fx);
    }
    let mail = m.mail.clone();
    // One question at a time: a status call slower than a second is not asked again on top of
    // itself.
    fx.spawn_keyed(JobKey { feature: "google-status", scope: Scope::App }, Concurrency::RejectIfRunning, move |_| {
        let outcome = state::google_outcome(calls::oauth_status(mail.as_ref(), &id));
        Command::GoogleStatus { flow: id, outcome }
    });
}

pub fn status(m: &mut Model, flow: &str, outcome: GoogleOutcome, fx: &mut Effects<Email>) {
    if outcome != GoogleOutcome::Waiting {
        finish(m, flow, outcome, fx);
    }
}

/// Put a finished sign-in on the screen.
fn finish(m: &mut Model, flow: &str, outcome: GoogleOutcome, fx: &mut Effects<Email>) {
    // A flow the window is no longer waiting on: cancelled, or abandoned and restarted. Applying
    // it would put an answer to an old question over the top of a new one.
    if m.google.flow.as_ref().map(|f| f.id.as_str()) != Some(flow) {
        return;
    }
    m.google.flow = None;
    fx.stop_timer(POLL);
    m.google.waiting = false;
    m.google.url.clear();
    fx.changed(part::SETUP);
    match outcome {
        // Should not reach here — the poll keeps going on Waiting — and if it does, it is said
        // rather than left as a window stuck on a spinner.
        GoogleOutcome::Waiting => {
            let text = "The Google sign-in is still waiting; this window stopped following it.";
            m.setup.ok = false;
            m.setup.status = text.into();
            say(m, fx, text);
        }
        GoogleOutcome::SignedIn(email) => {
            m.setup.ok = true;
            m.setup.status = format!("Signed in as {email}");
            m.setup.clear = Clear::Password;
            m.setup.open = false;
            fx.changed(part::SETUP_FIELDS);
            clear_notice(m, fx);
            // Through the sync, which runs on a job.
            fx.then(Command::Look(Look::Sync));
        }
        // Said twice, like every other failure in this app: on the form the person is looking
        // at, and in `describe.notice` for whatever is driving the window.
        GoogleOutcome::Stopped(reason) => {
            m.setup.ok = false;
            m.setup.status = reason.clone();
            say(m, fx, reason);
        }
    }
}

/// Give up on one, and tell the mail service to close the socket it is waiting on.
pub fn cancel(m: &mut Model, fx: &mut Effects<Email>) {
    let flow = m.google.flow.take();
    fx.stop_timer(POLL);
    m.google.waiting = false;
    m.google.url.clear();
    m.setup.ok = false;
    fx.changed(part::SETUP);
    match flow {
        Some(flow) => {
            m.setup.status = "The Google sign-in was cancelled.".into();
            clear_notice(m, fx);
            // In a job: the service stops a listener thread, which is fast, but this is still a
            // socket call from a button.
            let mail = m.mail.clone();
            fx.spawn(move |_| Command::GoogleCancelled(calls::oauth_cancel(mail.as_ref(), &flow.id)));
        }
        // Nothing to cancel. The panel is closed anyway rather than left up, because the window
        // saying it is waiting for something it is not waiting for is its own small lie.
        None => m.setup.status.clear(),
    }
}

pub fn cancelled(outcome: Result<(), String>) {
    if let Err(e) = outcome {
        tracing::warn!(error = %e, "the mail service could not be told to cancel");
    }
}
