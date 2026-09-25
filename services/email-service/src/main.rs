//! Email service — IMAP fetch, SMTP send, folder management via JSON-RPC.
//!
//! Reads account configuration from environment or config file.
//! Supports Gmail, Outlook, Yahoo, iCloud, and custom IMAP/SMTP servers.
//!
//! Methods:
//!   email.accounts        { }                                      → AccountsResult
//!   email.test_account    AccountSettings                          → TestAccountResult
//!   email.save_account    AccountSettings                          → EmailAccountSummary
//!   email.oauth_begin     { provider }                             → OAuthBeginResult
//!   email.oauth_status    { flow_id }                              → OAuthStatus
//!   email.oauth_cancel    { flow_id }                              → { cancelled }
//!   email.list_folders    { account_id }                           → Vec<EmailFolder>
//!   email.list_messages   { account_id, folder, page?, per_page? } → Vec<EmailSummary>
//!   email.get_message     { account_id, folder?, message_id }      → EmailDetail
//!   email.send_message    { account_id, to, subject, body, ... }   → ()
//!   email.mark_read       { account_id, folder?, message_id, read } → ()
//!   email.mark_starred    { account_id, folder?, message_id, starred }       → ()
//!   email.move_message    { account_id, folder?, message_id, target_folder } → ()
//!   email.delete_message  { account_id, folder?, message_id }                → ()
//!   email.search          { account_id, folder?, query }                     → Vec<EmailSummary>
//!
//! Every method that acts on listed messages takes an optional `folder`: IMAP UIDs are
//! per-mailbox, so a message listed from Spam can only be fetched from Spam. Omitted means
//! INBOX, which is all older callers ever asked about (#275). Star, move, delete and search
//! hardcoded INBOX for longer, and a delete of a Spam row could destroy a different INBOX
//! message; they take the folder too now, and the two that cannot be undone log it whenever a
//! caller still leans on the default (#288).
//!
//! `email.accounts` is the one that had to exist. Everything else here needs a mail server, so
//! the only question the app could ask was one whose failure meant three different things at
//! once — no account, a bad password, or an IMAP host that is down — and the app read all three
//! as "no account configured". `accounts` answers from the config file alone, without a socket
//! to anywhere, so "nothing is configured" is a different answer from "the mailbox would not
//! open", and both are different from this service not running at all.

mod accounts;
mod connect;
mod envelope;
mod folders;
mod google;
mod oauth;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accounts::Account;
use yantrik_ipc_contracts::email::*;
use yantrik_service_sdk::prelude::*;

/// How long any single network step may take: the TCP connect, and then each read and write on
/// the socket afterwards.
///
/// There was no bound at all before this, which meant a mail server that accepted a connection
/// and then said nothing held the request until the operating system gave up minutes later. The
/// app calls this service from a window; a person watching one is owed an answer.
const NET_TIMEOUT: Duration = Duration::from_secs(8);

/// Nothing is configured, as distinct from something being wrong. The app tells these apart by
/// asking `email.accounts`; this code is for the callers that do not.
const NO_ACCOUNT: i32 = -32001;

fn main() {
    ServiceBuilder::new("email")
        .handler(EmailHandler::new())
        .run();
}

// ── Account configuration ────────────────────────────────────────────

struct EmailHandler {
    config_path: std::path::PathBuf,
    /// Held only to serialise writes. Reads go to the file: it is a few hundred bytes, it can be
    /// edited by hand while this is running, and a cached copy is how a service comes to insist
    /// an account exists that somebody deleted an hour ago.
    ///
    /// An `Arc` because the Google sign-in's background thread writes the account it just
    /// verified, and it outlives the request that started it.
    writing: Arc<Mutex<()>>,
    /// The Google sign-ins this service is in the middle of.
    flows: oauth::Flows,
}

impl EmailHandler {
    fn new() -> Self {
        Self {
            config_path: accounts::config_path(),
            writing: Arc::new(Mutex::new(())),
            flows: oauth::Flows::default(),
        }
    }

    fn all_accounts(&self) -> Result<Vec<Account>, ServiceError> {
        accounts::load(&self.config_path).map_err(|message| ServiceError { code: -32000, message })
    }

    /// The account a mail request is about, ready to sign in with.
    ///
    /// Two things, not one, and the second is why this is not just a lookup: an OAuth account's
    /// access token lasts an hour, so every mail method has to be able to find one that expired
    /// while nobody was looking and renew it before the socket is opened. That happens here, once,
    /// rather than in each of the nine IMAP functions below.
    ///
    /// The refusal says which of the two things is true: there is no account at all, or the one
    /// that was asked for is not among those there.
    fn get_account(&self, account_id: &str) -> Result<Account, ServiceError> {
        let all = self.all_accounts()?;
        if all.is_empty() {
            return Err(ServiceError {
                code: NO_ACCOUNT,
                message: format!(
                    "no email account is configured; add one in the Email app, or write {}",
                    self.config_path.display()
                ),
            });
        }
        let account =
            accounts::pick(&all, Some(account_id)).cloned().ok_or_else(|| ServiceError {
                code: -32000,
                message: format!(
                    "no account called `{account_id}`; this machine has: {}",
                    all.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")
                ),
            })?;
        self.with_fresh_token(account)
    }

    /// Renew an OAuth account's access token if it is spent, and write the new one down.
    ///
    /// A password account passes straight through. For an OAuth one this is the difference
    /// between a mailbox that works tomorrow morning and one that stopped an hour after it was
    /// set up: Google's access tokens last 3600 seconds and nothing else in this service would
    /// ever ask for another.
    ///
    /// The new token is persisted because the alternative is refreshing on every single call —
    /// nine IMAP methods, each opening its own session — which would turn one sign-in into a
    /// token request per click and get the client rate-limited.
    fn with_fresh_token(&self, account: Account) -> Result<Account, ServiceError> {
        if !account.use_oauth {
            return Ok(account);
        }
        if !google::needs_refresh(account.oauth_expires_at, oauth::now()) {
            return Ok(account);
        }

        let Some(refresh_token) = account.oauth_refresh_token.clone().filter(|t| !t.is_empty())
        else {
            return Err(ServiceError {
                code: -32000,
                message: format!(
                    "Google sign-in expired \u{2014} sign in again. {} signs in with Google and \
                     this machine has nothing to renew its access with.",
                    account.email
                ),
            });
        };

        let client = google::client().map_err(|why| ServiceError {
            code: -32000,
            message: format!(
                "{} signs in with Google and this build cannot renew its access: {why}",
                account.email
            ),
        })?;

        let tokens = oauth::refresh(&client, &refresh_token).map_err(|message| ServiceError {
            code: -32000,
            // Already named and already redacted by `oauth::refresh`. A revoked refresh token
            // arrives here as "Google sign-in expired — sign in again", which is the whole point:
            // it is not an authentication failure to be retried, it is a grant that is gone.
            message,
        })?;

        let mut refreshed = account.clone();
        refreshed.oauth_token = Some(tokens.access.clone());
        refreshed.oauth_refresh_token = Some(tokens.refresh.clone());
        refreshed.oauth_expires_at = Some(tokens.expires_at);

        // Written down, and a failure to write is reported rather than swallowed: a service that
        // kept renewing because it could not remember the answer would look like it was working
        // while making a token request per mail click.
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.all_accounts()?;
        if accounts::store_refreshed(
            &mut all,
            &account.id,
            &tokens.access,
            &tokens.refresh,
            tokens.expires_at,
        ) {
            accounts::save(&self.config_path, &all)
                .map_err(|message| ServiceError { code: -32000, message })?;
            tracing::info!(account = %account.id, "Google access token renewed");
        } else {
            tracing::warn!(
                account = %account.id,
                "the account was removed while its Google token was being renewed"
            );
        }
        Ok(refreshed)
    }

    /// What `email.accounts` says about Google sign-in, and what `oauth_begin` needs.
    fn google_client(&self) -> Result<google::GoogleClient, String> {
        google::client()
    }

    /// Start a Google sign-in.
    ///
    /// The closure handed to the flow is where this service's own rules live: sign in over IMAP
    /// with the token *before* anything is written, then upsert and read back from disk, exactly
    /// as `save_account` does. Nothing gets into the accounts file on the strength of a token
    /// Google issued — a token is not a mailbox that opened.
    fn begin_google(&self) -> Result<OAuthBeginResult, ServiceError> {
        let client = self.google_client().map_err(|message| ServiceError {
            code: -32000,
            message,
        })?;

        let config_path = self.config_path.clone();
        let writing = self.writing.clone();
        let finish: oauth::Finish = Arc::new(move |email: &str, tokens: &google::Tokens| {
            let (imap_server, imap_port, smtp_server, smtp_port) = accounts::GOOGLE_SERVERS;
            let candidate = Account {
                id: accounts::id_for(email),
                email: email.to_string(),
                provider: "gmail".to_string(),
                imap_server: imap_server.to_string(),
                imap_port,
                smtp_server: smtp_server.to_string(),
                smtp_port,
                use_oauth: true,
                oauth_token: Some(tokens.access.clone()),
                oauth_refresh_token: Some(tokens.refresh.clone()),
                oauth_expires_at: Some(tokens.expires_at),
                ..Account::default()
            };

            // The mailbox, before the file. Gmail refuses XOAUTH2 with a token whose scope is
            // wrong, and "Google said yes" and "the mailbox opened" are two different facts.
            let attempt = connect::Attempt::new("IMAP", imap_server, imap_port);
            match imap_session(&candidate) {
                Ok(mut session) => {
                    let _ = session.logout();
                }
                Err(raw) => {
                    return Err(connect::name_oauth_failure(
                        &attempt,
                        &raw,
                        &candidate.secrets(),
                    ))
                }
            }

            let _writing = writing.lock().unwrap_or_else(|e| e.into_inner());
            let mut all = accounts::load(&config_path)?;
            let id = accounts::upsert_google(
                &mut all,
                email,
                &tokens.access,
                &tokens.refresh,
                tokens.expires_at,
            );
            accounts::save(&config_path, &all)?;

            // Read back from disk, so the answer is the account as it is now stored.
            let stored = accounts::load(&config_path)?;
            let saved = stored.iter().find(|a| a.id == id).ok_or_else(|| {
                format!("the account was written to {} and is not in it", config_path.display())
            })?;
            tracing::info!(account = %saved.id, "account saved from a Google sign-in");
            Ok(saved.summary())
        });

        self.flows
            .begin(client, finish)
            .map_err(|message| ServiceError { code: -32000, message })
    }
}

impl ServiceHandler for EmailHandler {
    fn service_id(&self) -> &str {
        "email"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        let account_id = params["account_id"]
            .as_str()
            .unwrap_or("default");

        match method {
            // Answers from the config file, never from a mail server. The whole point is that it
            // can answer on a machine with no network at all.
            method::ACCOUNTS => {
                let all = self.all_accounts()?;
                Ok(serde_json::to_value(AccountsResult {
                    accounts: accounts::summaries(&all),
                    config_path: self.config_path.display().to_string(),
                    // Said out loud rather than assumed. See the note at the head of
                    // `accounts.rs`: the password is in that file in clear text, and the app
                    // puts this on the screen beside the field it was typed into.
                    secrets_are_plaintext: true,
                    // Answered here because this is the call the app already makes on every
                    // load, and because the screen has to know before it draws the button. A
                    // "Sign in with Google" that cannot work is the dead control this whole
                    // flow replaces; when there is no client id the screen says so in words
                    // and points at the App Password path, which does work.
                    google_sign_in: google::availability(&self.google_client()),
                })
                .unwrap())
            }

            // Answers as soon as the loopback socket is bound, which is microseconds. Everything
            // slow — the person at the browser, the token exchange, the IMAP sign-in — happens on
            // a thread, and `oauth_status` is how the app finds out.
            method::OAUTH_BEGIN => {
                // There is one provider and it is named, rather than assumed, so a caller asking
                // for something else is told no instead of being signed in to Google.
                let provider = params["provider"].as_str().unwrap_or("google").trim().to_lowercase();
                if provider != "google" && provider != "gmail" {
                    return Err(ServiceError {
                        code: -32602,
                        message: format!(
                            "there is no `{provider}` sign-in here; this service can start a \
                             Google one. Other providers work from the account form with an app \
                             password."
                        ),
                    });
                }
                Ok(serde_json::to_value(self.begin_google()?).unwrap())
            }

            method::OAUTH_STATUS => {
                let flow_id = require_str(&params, "flow_id")?;
                let status = self.flows.status(flow_id).ok_or_else(|| ServiceError {
                    code: -32602,
                    // Not "waiting". A flow this service has never heard of, or one it has
                    // already forgotten, is a different thing from one that has not finished,
                    // and an app told "waiting" for it would poll until its own deadline.
                    message: "that sign-in is not one this service is waiting for; it may have \
                              been cancelled, or finished long enough ago to be forgotten. Start \
                              it again."
                        .to_string(),
                })?;
                Ok(serde_json::to_value(status).unwrap())
            }

            method::OAUTH_CANCEL => {
                let flow_id = require_str(&params, "flow_id")?;
                let cancelled = self.flows.cancel(flow_id);
                Ok(serde_json::json!({ "cancelled": cancelled }))
            }

            // Try the settings and store nothing. Both halves are reported: an account that can
            // read mail and cannot send it is a real state, and one word for both hides it.
            method::TEST_ACCOUNT => {
                let settings = parse_settings(&params)?;
                accounts::refuse_bad_settings(&settings)
                    .map_err(|message| ServiceError { code: -32602, message })?;
                Ok(serde_json::to_value(try_account(&settings)).unwrap())
            }

            // Verified before it is written. An account that cannot sign in is not an account,
            // and storing it would move the app to "configured" while every later call failed —
            // which is the fault this whole file was opened for, one layer up.
            method::SAVE_ACCOUNT => {
                let settings = parse_settings(&params)?;
                accounts::refuse_bad_settings(&settings)
                    .map_err(|message| ServiceError { code: -32602, message })?;

                let attempt = connect::Attempt::new(
                    "IMAP",
                    &settings.imap_server,
                    settings.imap_port,
                );
                if let Err(raw) = imap_try(&settings) {
                    return Err(ServiceError {
                        code: -32000,
                        message: connect::name_failure(&attempt, &raw, &settings.password),
                    });
                }

                let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
                let mut all = self.all_accounts()?;
                let id = accounts::upsert(&mut all, &settings);
                accounts::save(&self.config_path, &all)
                    .map_err(|message| ServiceError { code: -32000, message })?;

                // Read back from disk, so the answer is the account as it is now stored rather
                // than the one this process just built in memory.
                let stored = accounts::load(&self.config_path)
                    .map_err(|message| ServiceError { code: -32000, message })?;
                let saved = stored.iter().find(|a| a.id == id).ok_or_else(|| ServiceError {
                    code: -32000,
                    message: format!("the account was written to {} and is not in it",
                                     self.config_path.display()),
                })?;
                tracing::info!(account = %saved.id, "account saved");
                Ok(serde_json::to_value(saved.summary()).unwrap())
            }

            "email.list_folders" => {
                let account = self.get_account(account_id)?;
                let folders = imap_list_folders(&account)?;
                Ok(serde_json::to_value(folders).unwrap())
            }
            "email.list_messages" => {
                let account = self.get_account(account_id)?;
                let folder = params["folder"].as_str().unwrap_or("INBOX");
                let page = params["page"].as_u64().unwrap_or(1) as u32;
                let per_page = params["per_page"].as_u64().unwrap_or(20) as u32;
                let messages = imap_list_messages(&account, folder, page, per_page)?;
                Ok(serde_json::to_value(messages).unwrap())
            }
            "email.get_message" => {
                let account = self.get_account(account_id)?;
                let folder = folder_or_inbox(&params);
                let message_id = require_str(&params, "message_id")?;
                let detail = imap_get_message(&account, folder, message_id)?;
                Ok(serde_json::to_value(detail).unwrap())
            }
            "email.send_message" => {
                let account = self.get_account(account_id)?;
                let compose: ComposeRequest =
                    serde_json::from_value(params.clone()).map_err(|e| ServiceError {
                        code: -32602,
                        message: format!("Invalid compose params: {e}"),
                    })?;
                smtp_send(&account, &compose)?;
                Ok(serde_json::json!(null))
            }
            "email.mark_read" => {
                let account = self.get_account(account_id)?;
                let folder = folder_or_inbox(&params);
                let message_id = require_str(&params, "message_id")?;
                let read = params["read"].as_bool().unwrap_or(true);
                imap_mark_read(&account, folder, message_id, read)?;
                Ok(serde_json::json!(null))
            }
            "email.mark_starred" => {
                let account = self.get_account(account_id)?;
                let folder = folder_or_inbox(&params);
                let message_id = require_str(&params, "message_id")?;
                let starred = params["starred"].as_bool().unwrap_or(true);
                imap_mark_starred(&account, folder, message_id, starred)?;
                Ok(serde_json::json!(null))
            }
            "email.move_message" => {
                let account = self.get_account(account_id)?;
                let folder = folder_or_inbox_logged(&params, method);
                let message_id = require_str(&params, "message_id")?;
                let target = require_str(&params, "target_folder")?;
                imap_move_message(&account, folder, message_id, target)?;
                Ok(serde_json::json!(null))
            }
            "email.delete_message" => {
                let account = self.get_account(account_id)?;
                let folder = folder_or_inbox_logged(&params, method);
                let message_id = require_str(&params, "message_id")?;
                imap_delete_message(&account, folder, message_id)?;
                Ok(serde_json::json!(null))
            }
            "email.search" => {
                let account = self.get_account(account_id)?;
                let folder = folder_or_inbox(&params);
                let query = require_str(&params, "query")?;
                let results = imap_search(&account, folder, query)?;
                Ok(serde_json::to_value(results).unwrap())
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
}

fn require_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ServiceError> {
    params[key].as_str().ok_or_else(|| ServiceError {
        code: -32602,
        message: format!("Missing '{key}' parameter"),
    })
}

/// Whether the caller named the folder its message is in. Reads the parameter exactly as
/// [`folder_or_inbox`] does: absent, empty, whitespace or a non-string all count as unsaid.
fn names_a_folder(params: &serde_json::Value) -> bool {
    params["folder"].as_str().map(str::trim).filter(|f| !f.is_empty()).is_some()
}

/// The folder a call that cannot be undone acts on, with the default said out loud.
///
/// `delete_message` and `move_message` keep [`folder_or_inbox`]'s INBOX default only for
/// callers written before the parameter existed. UIDs are per-mailbox, so a caller leaning on
/// the default is either holding a message that really is in INBOX or is about to destroy or
/// displace a different message than the one it named; the log line is what tells the two
/// apart afterwards (#288).
fn folder_or_inbox_logged<'a>(params: &'a serde_json::Value, method: &str) -> &'a str {
    if !names_a_folder(params) {
        tracing::warn!(
            method,
            "no folder given; acting on INBOX. UIDs are per-mailbox — send the folder the \
             message was listed from, or this can act on the wrong message"
        );
    }
    folder_or_inbox(params)
}

// ── Setting an account up ────────────────────────────────────────────

fn parse_settings(params: &serde_json::Value) -> Result<AccountSettings, ServiceError> {
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
fn try_account(settings: &AccountSettings) -> TestAccountResult {
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
fn imap_try(settings: &AccountSettings) -> Result<(), String> {
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
fn smtp_try(settings: &AccountSettings) -> Result<(), String> {
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

// ── IMAP operations ──────────────────────────────────────────────────

/// A TCP connection that gives up rather than hanging, and that keeps giving up afterwards.
///
/// `TcpStream::connect` — which `imap::connect` uses — has no timeout, so a host that swallows
/// SYNs held this service for the operating system's own retry budget. The read and write
/// timeouts matter as much: a server that completes the handshake and then stops talking is the
/// commoner failure, and it is invisible to a connect timeout.
fn tcp_to(host: &str, port: u16) -> Result<std::net::TcpStream, String> {
    use std::net::ToSocketAddrs;
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("failed to lookup address for {host}: {e}"))?;
    let mut last = String::new();
    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, NET_TIMEOUT) {
            Ok(stream) => {
                let _ = stream.set_read_timeout(Some(NET_TIMEOUT));
                let _ = stream.set_write_timeout(Some(NET_TIMEOUT));
                return Ok(stream);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(if last.is_empty() {
        format!("failed to lookup address for {host}: it resolved to nothing")
    } else {
        last
    })
}

/// Connect and sign in, reporting the underlying library's own words.
///
/// The caller decides what to make of them: [`connect::name_failure`] turns them into a sentence
/// for a person, and the mail operations below wrap them in a [`ServiceError`].
fn imap_session(
    account: &Account,
) -> Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS error: {e}"))?;

    let tcp = tcp_to(&account.imap_server, account.imap_port)?;
    let stream = tls
        .connect(&account.imap_server, tcp)
        .map_err(|e| format!("TLS handshake failed: {e}"))?;

    let mut client = imap::Client::new(stream);
    client.read_greeting().map_err(|e| e.to_string())?;

    if account.use_oauth {
        // Not `unwrap_or("")`. An empty bearer token produces a Gmail refusal that reads as bad
        // credentials, which sends a person to check a password this account does not have.
        let token = match account.oauth_token.as_deref().filter(|t| !t.is_empty()) {
            Some(token) => token,
            None => {
                return Err(format!(
                    "Google sign-in expired \u{2014} sign in again. {} signs in with Google and \
                     there is no access token stored for it.",
                    account.email
                ))
            }
        };
        let auth_string = format!("user={}\x01auth=Bearer {}\x01\x01", account.email, token);
        client
            .authenticate("XOAUTH2", &XOAuth2Authenticator(auth_string))
            .map_err(|(e, _)| e.to_string())
    } else {
        client
            .login(&account.email, &account.password)
            .map_err(|(e, _)| e.to_string())
    }
}

fn imap_connect(
    account: &Account,
) -> Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>, ServiceError> {
    let attempt = connect::Attempt::new("IMAP", &account.imap_server, account.imap_port);
    imap_session(account).map_err(|raw| ServiceError {
        code: -32000,
        // Every secret this account holds, not just the password: the XOAUTH2 line carries the
        // access token, and a server is free to quote back the line it was sent.
        message: if account.use_oauth {
            connect::name_oauth_failure(&attempt, &raw, &account.secrets())
        } else {
            connect::name_failure_secrets(&attempt, &raw, &account.secrets())
        },
    })
}

struct XOAuth2Authenticator(String);

impl imap::Authenticator for XOAuth2Authenticator {
    type Response = String;
    fn process(&self, _data: &[u8]) -> Self::Response {
        self.0.clone()
    }
}

fn imap_list_folders(account: &Account) -> Result<Vec<EmailFolder>, ServiceError> {
    let mut session = imap_connect(account)?;

    let folders = session
        .list(None, Some("*"))
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP LIST failed: {e}"),
        })?;

    let mut result = Vec::new();
    for folder in folders.iter() {
        let name = folder.name().to_string();

        // A container of other folders — Gmail's `[Gmail]` — holds no mail and refuses STATUS.
        // Not asked; the list says what it is instead of "0 of 0".
        if folder.attributes().contains(&imap::types::NameAttribute::NoSelect) {
            result.push(folders::container_entry(&name));
            continue;
        }

        // One STATUS per folder, and the answer read from where the `imap` crate puts it: the
        // unsolicited channel, not the `Mailbox` it returns. `status(..).unseen` was read before
        // and was always `None`, which `.unwrap_or(0)` made "unread: 0" for every folder on every
        // machine. `folders.rs` has the whole of it.
        //
        // A folder the server would not count this time — a refusal, a reply that did not come
        // — goes in the list with no counts and the reason beside it. It used to go in with
        // zeros, which is what an empty folder reads as (#131). The server's words are kept, with
        // every secret this account holds taken out of them first, as everywhere else here.
        let status = session
            .status(&name, folders::STATUS_ITEMS)
            .map(|_| ())
            .map_err(|e| without_secrets(&e.to_string(), &account.secrets()));
        result.push(folders::entry(&name, status, session.unsolicited_responses.try_iter()));
    }

    let _ = session.logout();
    Ok(result)
}

fn imap_list_messages(
    account: &Account,
    folder: &str,
    page: u32,
    per_page: u32,
) -> Result<Vec<EmailSummary>, ServiceError> {
    let mut session = imap_connect(account)?;

    let mailbox = session.select(folder).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {folder} failed: {e}"),
    })?;

    // The newest `per_page` messages by sequence number, and exactly that many: the range used
    // to be inclusive at both ends, so a page of twenty came back as twenty-one, and the app's
    // header said "of 21" for a folder the folder list said held 35.
    let Some((start, end)) = folders::page_range(mailbox.exists, page, per_page) else {
        let _ = session.logout();
        return Ok(Vec::new());
    };

    let range = format!("{start}:{end}");
    let messages = session
        .fetch(&range, "(UID FLAGS ENVELOPE BODYSTRUCTURE)")
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP FETCH failed: {e}"),
        })?;

    let mut result = Vec::new();
    for msg in messages.iter() {
        let uid = msg.uid.unwrap_or(0);
        let envelope = match msg.envelope() {
            Some(e) => e,
            None => continue,
        };

        let from = envelope
            .from
            .as_ref()
            .and_then(|addrs| addrs.first())
            .map(|a| {
                let name = a.name.as_ref().map(|n| envelope::text(n));
                let mailbox = a.mailbox.as_ref().map(|m| String::from_utf8_lossy(m).to_string()).unwrap_or_default();
                let host = a.host.as_ref().map(|h| String::from_utf8_lossy(h).to_string()).unwrap_or_default();
                match name {
                    Some(n) if !n.is_empty() => n,
                    _ => format!("{mailbox}@{host}"),
                }
            })
            .unwrap_or_else(|| "Unknown".to_string());

        let subject = envelope
            .subject
            .as_ref()
            .map(|s| envelope::text(s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(no subject)".to_string());

        let date = envelope
            .date
            .as_ref()
            .map(|d| envelope::date(d))
            .unwrap_or_default();

        let flags = msg.flags();
        let is_read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));
        let is_starred = flags.iter().any(|f| matches!(f, imap::types::Flag::Flagged));

        result.push(EmailSummary {
            id: uid.to_string(),
            from,
            to: Vec::new(),
            subject,
            snippet: String::new(),
            date,
            is_read,
            is_starred,
            has_attachments: false,
            folder: folder.to_string(),
            thread_id: None,
        });
    }

    result.reverse(); // newest first
    let _ = session.logout();
    Ok(result)
}

fn imap_get_message(
    account: &Account,
    folder: &str,
    message_id: &str,
) -> Result<EmailDetail, ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    // The message's own folder, not INBOX, and a refused SELECT is an error rather than
    // something to shrug off: UIDs are per-mailbox, so opening a message that was listed from
    // Spam fetched whatever wore this UID in INBOX — the wrong message, or none at all (#275).
    session.select(folder).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {folder} failed: {e}"),
    })?;

    let messages = session
        .uid_fetch(uid.to_string(), "(RFC822 FLAGS)")
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP UID FETCH failed: {e}"),
        })?;

    let msg = messages.first().ok_or_else(|| ServiceError {
        code: -32000,
        message: format!("Message not found: {message_id}"),
    })?;

    // The FLAGS this fetch already asks for. They were read off the wire and dropped, so the app
    // had nothing to show a message's read or flagged state from and hardcoded both.
    let flags = msg.flags();
    let is_read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));
    let is_starred = flags.iter().any(|f| matches!(f, imap::types::Flag::Flagged));

    let body = msg.body().unwrap_or(&[]);
    let parsed = mailparse::parse_mail(body).map_err(|e| ServiceError {
        code: -32000,
        message: format!("Mail parse error: {e}"),
    })?;

    let from = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "From")
        .map(|h| h.get_value())
        .unwrap_or_default();

    let to: Vec<String> = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "To")
        .map(|h| h.get_value().split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let cc: Vec<String> = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "Cc")
        .map(|h| h.get_value().split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let subject = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "Subject")
        .map(|h| h.get_value())
        .unwrap_or_default();

    let date = parsed
        .headers
        .iter()
        .find(|h| h.get_key_ref() == "Date")
        .map(|h| h.get_value())
        .unwrap_or_default();

    // Extract body text/html
    let mut body_text = String::new();
    let mut body_html = String::new();
    let mut attachments = Vec::new();

    extract_parts(&parsed, &mut body_text, &mut body_html, &mut attachments);

    let body_text = plain_body(&body_text, &body_html);

    let _ = session.logout();

    Ok(EmailDetail {
        id: message_id.to_string(),
        from,
        to,
        cc,
        bcc: Vec::new(),
        subject,
        body_html,
        body_text,
        date,
        attachments,
        thread_messages: Vec::new(),
        is_read,
        is_starred,
    })
}

/// The body the reading pane shows: the sender's own plain text when the mail has one, and
/// otherwise the HTML turned into text by the reading-pane rule in `yantrik-email-text`.
///
/// The rule used to be html2text's defaults, which render for a fixed-width terminal: layout
/// tables came back drawn in box characters, every link left a `[1]` footnote, every logo
/// became `[Subreddit Icon]`, and lines were hard-wrapped at column 80 — in a proportional-font
/// pane that wraps on its own, so it showed mail broken in places the sender never chose (#275).
fn plain_body(body_text: &str, body_html: &str) -> String {
    if !body_text.is_empty() {
        return body_text.to_string();
    }
    yantrik_email_text::readable_text(body_html)
}

fn extract_parts(
    mail: &mailparse::ParsedMail,
    body_text: &mut String,
    body_html: &mut String,
    attachments: &mut Vec<EmailAttachment>,
) {
    let content_type = mail.ctype.mimetype.as_str();

    if mail.subparts.is_empty() {
        match content_type {
            "text/plain" => {
                if body_text.is_empty() {
                    *body_text = mail.get_body().unwrap_or_default();
                }
            }
            "text/html" => {
                if body_html.is_empty() {
                    *body_html = mail.get_body().unwrap_or_default();
                }
            }
            _ => {
                // Attachment
                let filename = mail
                    .ctype
                    .params
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| "attachment".to_string());
                let size = mail.get_body_raw().map(|b| b.len() as u64).unwrap_or(0);
                attachments.push(EmailAttachment {
                    filename,
                    mime_type: content_type.to_string(),
                    size_bytes: size,
                });
            }
        }
    } else {
        for part in &mail.subparts {
            extract_parts(part, body_text, body_html, attachments);
        }
    }
}

fn smtp_send(account: &Account, compose: &ComposeRequest) -> Result<(), ServiceError> {
    use lettre::{Message, Transport};

    let mut email_builder = Message::builder()
        .from(account.email.parse().map_err(|e| ServiceError {
            code: -32000,
            message: format!("Invalid from address: {e}"),
        })?)
        .subject(&compose.subject);

    for to in &compose.to {
        email_builder = email_builder.to(to.parse().map_err(|e| ServiceError {
            code: -32000,
            message: format!("Invalid to address '{to}': {e}"),
        })?);
    }
    for cc in &compose.cc {
        email_builder = email_builder.cc(cc.parse().map_err(|e| ServiceError {
            code: -32000,
            message: format!("Invalid cc address '{cc}': {e}"),
        })?);
    }

    let body = if let Some(ref sig) = compose.signature {
        format!("{}\n\n--\n{}", compose.body, sig)
    } else {
        compose.body.clone()
    };

    let email = email_builder
        .body(body)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("Failed to build email: {e}"),
        })?;

    // An OAuth account sends with its access token and no password. This was the missing half:
    // IMAP learned XOAUTH2 and SMTP never did, so an account signed in with Google could read
    // mail and could not send any — and the sentence for that failure was about a password the
    // account does not have.
    let secret = if account.use_oauth {
        account.oauth_token.clone().unwrap_or_default()
    } else {
        account.password.clone()
    };

    let attempt = connect::Attempt::new("SMTP", &account.smtp_server, account.smtp_port);
    let name = |raw: &str| {
        if account.use_oauth {
            connect::name_oauth_failure(&attempt, raw, &account.secrets())
        } else {
            connect::name_failure_secrets(&attempt, raw, &account.secrets())
        }
    };

    let mailer = smtp_transport(
        &account.email,
        &secret,
        &account.smtp_server,
        account.smtp_port,
        account.use_oauth,
    )
    .map_err(|raw| ServiceError { code: -32000, message: name(&raw) })?;

    mailer.send(&email).map_err(|e| ServiceError {
        code: -32000,
        message: name(&e.to_string()),
    })?;

    tracing::info!(to = ?compose.to, subject = %compose.subject, "Email sent");
    Ok(())
}

/// An SMTP transport that goes to the port the account says.
///
/// `SmtpTransport::relay` opens an implicitly-TLS connection on 465 regardless of what is
/// configured, so every account on the default 587 was sent to the wrong port and the
/// configured one was never read at all. 587 is submission with STARTTLS and 465 is submission
/// over TLS; they are different handshakes, and picking by port is what every other mail client
/// does. Anything else is treated as STARTTLS, which is what a hand-entered port on a private
/// server almost always is.
/// `use_oauth` picks the SASL mechanism: XOAUTH2 with the access token as the secret, rather
/// than PLAIN or LOGIN with a password. lettre builds the same `user=…\x01auth=Bearer …` string
/// the IMAP side builds by hand, so the two halves of an account sign in the same way.
///
/// The mechanism is named rather than left to the default, because lettre's default list is
/// PLAIN then LOGIN and neither of those will take an OAuth token: Gmail answers `535` and the
/// account looks like it has a bad password.
fn smtp_transport(
    email: &str,
    secret: &str,
    server: &str,
    port: u16,
    use_oauth: bool,
) -> Result<lettre::SmtpTransport, String> {
    use lettre::transport::smtp::authentication::{Credentials, Mechanism};

    let builder = if port == 465 {
        lettre::SmtpTransport::relay(server)
    } else {
        lettre::SmtpTransport::starttls_relay(server)
    }
    .map_err(|e| e.to_string())?;

    let builder = builder
        .port(port)
        .timeout(Some(NET_TIMEOUT))
        .credentials(Credentials::new(email.to_string(), secret.to_string()));

    Ok(if use_oauth {
        builder.authentication(vec![Mechanism::Xoauth2]).build()
    } else {
        builder.build()
    })
}

/// The SELECT target and UID STORE argument for one flag change, as a pair.
///
/// A STORE runs in whichever mailbox was last SELECTed, and UIDs are per-mailbox, so the two
/// halves of one change belong together: the hardcoded `select("INBOX")` that used to sit
/// beside these commands starred and deleted whichever message wore the same UID in INBOX,
/// whatever folder the person had chosen the message from (#288). Built apart from the
/// session so that the pair can be held to this without a mail server.
fn flag_change(folder: &str, set: bool, flag: &str) -> (String, String) {
    (folder.to_string(), format!("{}FLAGS ({flag})", if set { "+" } else { "-" }))
}

fn imap_mark_read(
    account: &Account,
    folder: &str,
    message_id: &str,
    read: bool,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    // The message's own folder, for get_message's reason: the app marks a message read right
    // after opening it, and flagging INBOX's UID would touch a stranger's mail (#275).
    session.select(folder).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {folder} failed: {e}"),
    })?;

    let flag = "+FLAGS (\\Seen)";
    let unflag = "-FLAGS (\\Seen)";
    session
        .uid_store(uid.to_string(), if read { flag } else { unflag })
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP STORE failed: {e}"),
        })?;

    let _ = session.logout();
    Ok(())
}

fn imap_mark_starred(
    account: &Account,
    folder: &str,
    message_id: &str,
    starred: bool,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let (mailbox, store) = flag_change(folder, starred, "\\Flagged");
    let mut session = imap_connect(account)?;
    // The mailbox the message was listed from, and a refused SELECT is an error rather than
    // something to shrug off: starring ran in INBOX whatever folder was on screen, so it
    // moved the star on whichever message wore this UID there (#288).
    session.select(&mailbox).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {mailbox} failed: {e}"),
    })?;

    session
        .uid_store(uid.to_string(), &store)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP STORE failed: {e}"),
        })?;

    let _ = session.logout();
    Ok(())
}

fn imap_move_message(
    account: &Account,
    folder: &str,
    message_id: &str,
    target_folder: &str,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let mut session = imap_connect(account)?;
    // The mailbox the message is actually in: a move of a row listed from Spam moved
    // whichever message wore this UID in INBOX, and left the person's own mail where it was
    // (#288).
    session.select(folder).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {folder} failed: {e}"),
    })?;

    session
        .uid_mv(uid.to_string(), target_folder)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP MOVE failed: {e}"),
        })?;

    let _ = session.logout();
    Ok(())
}

fn imap_delete_message(
    account: &Account,
    folder: &str,
    message_id: &str,
) -> Result<(), ServiceError> {
    let uid: u32 = message_id.parse().map_err(|_| ServiceError {
        code: -32602,
        message: "Invalid message ID".to_string(),
    })?;

    let (mailbox, store) = flag_change(folder, true, "\\Deleted");
    let mut session = imap_connect(account)?;
    // The mailbox the message was listed from, and a refused SELECT stops the call instead of
    // being shrugged off: delete SELECTed INBOX whatever folder was on screen, and the EXPUNGE
    // below made the mistake permanent — deleting a Spam row could destroy a different INBOX
    // message (#288).
    session.select(&mailbox).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {mailbox} failed: {e}"),
    })?;

    session
        .uid_store(uid.to_string(), &store)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP delete flag failed: {e}"),
        })?;
    session.expunge().map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP EXPUNGE failed: {e}"),
    })?;

    let _ = session.logout();
    Ok(())
}

fn imap_search(
    account: &Account,
    folder: &str,
    query: &str,
) -> Result<Vec<EmailSummary>, ServiceError> {
    let mut session = imap_connect(account)?;
    // The folder the caller is looking at: search SELECTed INBOX whichever folder was on
    // screen, so searching with Spam open answered with mail from a mailbox nobody was
    // looking at (#288).
    session.select(folder).map_err(|e| ServiceError {
        code: -32000,
        message: format!("IMAP SELECT {folder} failed: {e}"),
    })?;

    // IMAP search by subject or from
    let search_query = format!("OR SUBJECT \"{}\" FROM \"{}\"", query, query);
    let uids = session
        .search(&search_query)
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP SEARCH failed: {e}"),
        })?;

    if uids.is_empty() {
        let _ = session.logout();
        return Ok(Vec::new());
    }

    // Fetch the found messages (limit to 50)
    let mut uid_vec: Vec<u32> = uids.into_iter().collect();
    uid_vec.sort_unstable();
    uid_vec.reverse();
    uid_vec.truncate(50);
    let uid_list: Vec<String> = uid_vec.iter().map(|u| u.to_string()).collect();
    let uid_range = uid_list.join(",");

    let messages = session
        .fetch(&uid_range, "(UID FLAGS ENVELOPE)")
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("IMAP FETCH failed: {e}"),
        })?;

    let mut result = Vec::new();
    for msg in messages.iter() {
        let uid = msg.uid.unwrap_or(0);
        let envelope = match msg.envelope() {
            Some(e) => e,
            None => continue,
        };

        let from = envelope
            .from
            .as_ref()
            .and_then(|addrs| addrs.first())
            .map(|a| {
                let name = a.name.as_ref().map(|n| envelope::text(n));
                let mailbox = a.mailbox.as_ref().map(|m| String::from_utf8_lossy(m).to_string()).unwrap_or_default();
                let host = a.host.as_ref().map(|h| String::from_utf8_lossy(h).to_string()).unwrap_or_default();
                match name {
                    Some(n) if !n.is_empty() => n,
                    _ => format!("{mailbox}@{host}"),
                }
            })
            .unwrap_or_else(|| "Unknown".to_string());

        let subject = envelope
            .subject
            .as_ref()
            .map(|s| envelope::text(s))
            .unwrap_or_default();

        let date = envelope
            .date
            .as_ref()
            .map(|d| envelope::date(d))
            .unwrap_or_default();

        let flags = msg.flags();
        let is_read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));
        let is_starred = flags.iter().any(|f| matches!(f, imap::types::Flag::Flagged));

        result.push(EmailSummary {
            id: uid.to_string(),
            from,
            to: Vec::new(),
            subject,
            snippet: String::new(),
            date,
            is_read,
            is_starred,
            has_attachments: false,
            // The folder that was searched, which is the folder the UID belongs to: the app
            // reads this back when a result is opened, starred or deleted.
            folder: folder.to_string(),
            thread_id: None,
        });
    }

    let _ = session.logout();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{extract_parts, flag_change, names_a_folder, plain_body};
    use yantrik_ipc_contracts::email::EmailAttachment;

    /// A mail whose sender supplied both parts, the way multipart/alternative is meant to be
    /// read: the plain text is the sender's own words for this reader, so it wins and no
    /// conversion runs.
    const ALTERNATIVE: &[u8] = b"From: sender@example.com\r\nSubject: Both parts\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"BOUND\"\r\n\r\n--BOUND\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe sender wrote this plain text for people; it is not a conversion of the HTML.\r\n--BOUND\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>The <b>HTML</b> version of the same words.</p></body></html>\r\n--BOUND--\r\n";

    /// A table-laid-out newsletter with no plain part: the shape that reached the reading pane
    /// drawn in box characters with `[1]` footnotes under it (#275).
    const HTML_ONLY: &[u8] = b"From: news@example.com\r\nSubject: Newsletter\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<table border=\"1\" cellpadding=\"4\"><tr><td><a href=\"https://news.example/home\"><img src=\"https://news.example/logo.png\" alt=\"Company Logo\"></a></td></tr><tr><td><a href=\"https://news.example/story\">The story everyone is reading this week</a></td></tr><tr><td><p>It arrived as a table-laid-out newsletter, and this sentence in its cell is far longer than eighty columns so a terminal-width conversion would have to break it somewhere.</p></td></tr><tr><td><a href=\"https://news.example/up\">24 upvotes</a> <a href=\"https://news.example/c\">21 comments</a></td></tr></table>";

    fn parts(raw: &[u8]) -> (String, String, Vec<EmailAttachment>) {
        let parsed = mailparse::parse_mail(raw).unwrap();
        let mut body_text = String::new();
        let mut body_html = String::new();
        let mut attachments = Vec::new();
        extract_parts(&parsed, &mut body_text, &mut body_html, &mut attachments);
        (body_text, body_html, attachments)
    }

    #[test]
    fn multipart_alternative_prefers_the_senders_plain_text() {
        let (body_text, body_html, _) = parts(ALTERNATIVE);
        assert!(body_text.contains("The sender wrote this plain text"), "plain part missing: {body_text:?}");
        assert!(body_html.contains("<b>HTML</b>"), "html part missing: {body_html:?}");
        let shown = plain_body(&body_text, &body_html);
        assert_eq!(shown, body_text, "the sender's own plain text must win over any conversion");
    }

    #[test]
    fn html_only_mail_is_converted_for_the_reading_pane() {
        let (body_text, body_html, _) = parts(HTML_ONLY);
        assert!(body_text.is_empty(), "an HTML-only mail has no plain part");
        assert!(!body_html.is_empty());

        let shown = plain_body(&body_text, &body_html);
        for c in "─│┼┬┐└├┤┴┘".chars() {
            assert!(!shown.contains(c), "table border {c:?} in:\n{shown}");
        }
        // Every bracket the old conversion produced was a footnote reference, a link target or
        // an image alt; nothing in this mail's words has one.
        assert!(!shown.contains('[') && !shown.contains(']'), "bracket in:\n{shown}");
        assert!(!shown.contains("Company Logo"), "image alt in:\n{shown}");
        assert!(!shown.contains("https://"), "footnote URL in:\n{shown}");

        let title = shown.find("The story everyone is reading this week").expect("title missing");
        let body = shown.find("It arrived as a table-laid-out newsletter").expect("body missing");
        assert!(title < body, "cells out of order in:\n{shown}");
        assert!(shown.contains("24 upvotes") && shown.contains("21 comments"), "footer cells missing:\n{shown}");

        let sentence = "this sentence in its cell is far longer than eighty columns so a terminal-width conversion would have to break it somewhere.";
        assert!(shown.lines().any(|l| l.contains(sentence)), "sentence hard-wrapped in:\n{shown}");
    }

    /// The heart of #288: the mailbox a star or a delete runs in is the one the message was
    /// listed from. Every per-message call SELECTed INBOX before it, whatever folder was on
    /// screen, so acting on a Spam row acted on whichever INBOX message wore the same UID.
    #[test]
    fn a_message_listed_from_a_folder_is_acted_on_in_that_folder() {
        assert_eq!(
            flag_change("Spam", true, "\\Flagged"),
            ("Spam".to_string(), "+FLAGS (\\Flagged)".to_string())
        );
        assert_eq!(
            flag_change("[Gmail]/Sent Mail", false, "\\Flagged"),
            ("[Gmail]/Sent Mail".to_string(), "-FLAGS (\\Flagged)".to_string())
        );
        assert_eq!(
            flag_change("Spam", true, "\\Deleted"),
            ("Spam".to_string(), "+FLAGS (\\Deleted)".to_string())
        );
    }

    /// Delete and move keep the INBOX default for callers written before `folder` existed and
    /// log whenever one leans on it; this predicate is what the log hangs off. It has to read
    /// the parameter exactly as `folder_or_inbox` does, or the log fires on calls that did
    /// name a folder and stays quiet on the ones that did not.
    #[test]
    fn a_destructive_call_that_left_the_folder_unsaid_is_recognisable() {
        assert!(!names_a_folder(&serde_json::json!({})));
        assert!(!names_a_folder(&serde_json::json!({"folder": ""})));
        assert!(!names_a_folder(&serde_json::json!({"folder": "   "})));
        assert!(!names_a_folder(&serde_json::json!({"folder": null})));
        assert!(!names_a_folder(&serde_json::json!({"folder": 7})));
        assert!(names_a_folder(&serde_json::json!({"folder": "Spam"})));
        assert!(names_a_folder(&serde_json::json!({"folder": "[Gmail]/Sent Mail"})));
    }
}
