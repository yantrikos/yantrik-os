//! Trying an account's settings against its mail servers, before anything is stored.
//!
//! Nothing here stores anything or sends mail. The password it is given is passed to
//! [`connect::name_failure`] so that a server quoting the line it was sent cannot put it on a
//! screen, and it is in this process only for the length of the call: the credential vault
//! (`store.rs`) is where it is kept afterwards.

use yantrik_ipc_contracts::email::{AccountSettings, ServiceError, TestAccountResult};

use crate::accounts::Account;
use crate::{connect, imap_session, smtp_transport};

pub(crate) fn parse_settings(params: &serde_json::Value) -> Result<AccountSettings, ServiceError> {
    serde_json::from_value(params.clone()).map_err(|e| ServiceError {
        code: -32602,
        // `e` is serde's account of which field is missing or mistyped. It never contains a
        // value, only a field name and a type, so there is no password in it.
        message: format!("these are not account settings: {e}"),
    })
}

/// Sign in to both halves of an account and say what each one did.
///
/// Nothing is stored and nothing is sent. The password is passed to
/// [`connect::name_failure`] so that a server quoting the line it was sent cannot put it on a
/// screen.
pub(crate) fn try_account(settings: &AccountSettings) -> TestAccountResult {
    let imap_where =
        connect::Attempt::new("IMAP", &settings.imap_server, settings.imap_port);
    let (imap_ok, imap) = match imap_try(settings) {
        Ok(()) => (true, connect::name_success(&imap_where)),
        Err(raw) => (false, connect::name_failure(&imap_where, &raw, &settings.password)),
    };

    let smtp_where =
        connect::Attempt::new("SMTP", &settings.smtp_server, settings.smtp_port);
    let (smtp_ok, smtp) = match smtp_try(settings) {
        Ok(()) => (true, connect::name_success(&smtp_where)),
        Err(raw) => (false, connect::name_failure(&smtp_where, &raw, &settings.password)),
    };

    TestAccountResult { imap_ok, imap, smtp_ok, smtp }
}

/// One IMAP sign-in with the supplied settings, and straight back out.
pub(crate) fn imap_try(settings: &AccountSettings) -> Result<(), String> {
    let account = Account {
        email: settings.email.clone(),
        password: settings.password.clone(),
        imap_server: settings.imap_server.clone(),
        imap_port: settings.imap_port,
        smtp_server: settings.smtp_server.clone(),
        smtp_port: settings.smtp_port,
        ..Account::default()
    };
    let mut session = imap_session(&account)?;
    let _ = session.logout();
    Ok(())
}

/// One SMTP sign-in with the supplied settings. `test_connection` in lettre opens the
/// connection, which is where authentication happens, then sends NOOP and quits.
pub(crate) fn smtp_try(settings: &AccountSettings) -> Result<(), String> {
    let mailer = smtp_transport(
        &settings.email,
        &settings.password,
        &settings.smtp_server,
        settings.smtp_port,
        false,
    )?;
    match lettre::SmtpTransport::test_connection(&mailer) {
        Ok(true) => Ok(()),
        Ok(false) => Err("the server accepted the connection and then dropped it".to_string()),
        Err(e) => Err(e.to_string()),
    }
}
