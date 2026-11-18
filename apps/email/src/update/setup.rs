//! Account setup: the form, Test Connection, and saving the account.
//!
//! Neither button is on the control surface. The password is the most sensitive thing this app
//! handles, and an action that accepted one would put it in a transcript a mind can read. The
//! password travels from the window to the mail service inside one command and one job; it is
//! never kept in the model, never said, and it leaves the window the moment it is not needed.

use yantrik_app::effects::Effects;
use yantrik_ipc_contracts::email::{AccountSettings, EmailAccountSummary, TestAccountResult};

use crate::backend::calls;
use crate::command::{Command, Look, SetupForm, Work};
use crate::model::{part, Clear, Model};
use crate::{state, Email};

/// The form opened fresh, for another account.
pub fn add_account(m: &mut Model, fx: &mut Effects<Email>) {
    m.setup.open = true;
    m.setup.status.clear();
    m.setup.ok = false;
    m.setup.clear = Clear::Fresh;
    // A form opened fresh is not waiting for anything. Left over from a previous visit, the
    // waiting panel would hide the fields and offer a Cancel for a flow that ended.
    m.google.waiting = false;
    m.google.url.clear();
    fx.changed(part::SETUP);
    fx.changed(part::SETUP_FIELDS);
}

/// The form closed without saving.
pub fn cancel_setup(m: &mut Model, fx: &mut Effects<Email>) {
    // A setup form closed while a Google sign-in is in flight leaves a listener open on this
    // machine and a consent page in a browser that leads nowhere. Closed properly.
    if m.google.flow.is_some() {
        super::google::cancel(m, fx);
    }
    m.setup.open = false;
    // Not kept anywhere, on purpose: a password left in a property is a password in the process
    // for as long as the window is open.
    m.setup.clear = Clear::Password;
    m.setup.status.clear();
    m.setup.ok = false;
    fx.changed(part::SETUP);
    fx.changed(part::SETUP_FIELDS);
}

/// The settings the form describes, or the sentence about what is missing. No refusal here
/// contains the password.
fn settings(form: &SetupForm) -> Result<AccountSettings, String> {
    state::account_settings_from_form(
        &form.email,
        &form.password,
        &form.display_name,
        &form.provider,
        &form.imap_server,
        &form.imap_port,
        &form.smtp_server,
        &form.smtp_port,
    )
}

/// Ready the form for a sign-in, or say on it why it cannot be tried.
fn begin(m: &mut Model, form: &SetupForm, fx: &mut Effects<Email>) -> Option<AccountSettings> {
    if m.setup.busy {
        return None;
    }
    fx.changed(part::SETUP);
    let settings = match settings(form) {
        Ok(settings) => settings,
        Err(e) => {
            m.setup.ok = false;
            m.setup.status = e;
            return None;
        }
    };
    m.setup.busy = true;
    m.setup.testing = true;
    m.setup.status = "Signing in\u{2026}".into();
    m.setup.ok = false;
    Some(settings)
}

/// Both halves of the account tried, on a job: signing in to a mail server takes seconds, and a
/// form that freezes the window it is on is its own kind of lie about what is happening.
pub fn test_connection(m: &mut Model, form: &SetupForm, fx: &mut Effects<Email>) {
    let Some(settings) = begin(m, form, fx) else { return };
    let mail = m.mail.clone();
    fx.spawn_or(Command::Unfinished(Work::Setup), move |_| {
        Command::Tested(calls::test_account(mail.as_ref(), &settings))
    });
}

pub fn tested(m: &mut Model, outcome: Result<TestAccountResult, String>, fx: &mut Effects<Email>) {
    m.setup.busy = false;
    m.setup.testing = false;
    fx.changed(part::SETUP);
    match outcome {
        Ok(result) => {
            m.setup.ok = result.imap_ok && result.smtp_ok;
            m.setup.status = state::test_summary(result.imap_ok, &result.imap, result.smtp_ok, &result.smtp);
        }
        Err(e) => {
            m.setup.ok = false;
            m.setup.status = format!("The mail service could not try it: {e}");
        }
    }
}

/// Save the account. The service signs in before it writes anything, so an account that lands
/// in the config file is one that worked at least once.
pub fn save_account(m: &mut Model, form: &SetupForm, fx: &mut Effects<Email>) {
    let Some(settings) = begin(m, form, fx) else { return };
    let mail = m.mail.clone();
    fx.spawn_or(Command::Unfinished(Work::Setup), move |_| {
        Command::Saved(calls::save_account(mail.as_ref(), &settings))
    });
}

pub fn saved(m: &mut Model, outcome: Result<EmailAccountSummary, String>, fx: &mut Effects<Email>) {
    m.setup.busy = false;
    m.setup.testing = false;
    fx.changed(part::SETUP);
    match outcome {
        Ok(saved) => {
            m.setup.ok = true;
            m.setup.status = format!("Signed in as {}", saved.email);
            // The password leaves the window the moment it is not needed.
            m.setup.clear = Clear::Password;
            m.setup.open = false;
            fx.changed(part::SETUP_FIELDS);
            // Through the sync, which runs on a job.
            fx.then(Command::Look(Look::Sync));
        }
        Err(e) => {
            // The form stays up, holding what was typed, with the mail server's own answer
            // above it.
            m.setup.ok = false;
            m.setup.status = e;
        }
    }
}

