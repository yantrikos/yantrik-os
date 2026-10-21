//! Which of three states the app is in: the service could not be reached, it answered with
//! no account, or it holds one.

use yantrik_ipc_contracts::email::{AccountsResult, EmailAccountSummary, GoogleSignIn};

/// What the app has been told about the mail service, and about the account behind it.
#[derive(Debug, Clone, PartialEq)]
pub enum MailState {
    /// The service could not be started or could not be answered. Carries the reason, which is
    /// the thing that used to be thrown away.
    Unreachable { reason: String },
    /// The service answered, and holds no account.
    NoAccount {
        config_path: String,
        secrets_are_plaintext: bool,
        google: GoogleSignIn,
    },
    /// The service answered, and holds at least one.
    Ready {
        account: EmailAccountSummary,
        config_path: String,
        secrets_are_plaintext: bool,
        google: GoogleSignIn,
    },
}

impl MailState {
    /// What `describe` reports under `service`.
    pub fn service_word(&self) -> &'static str {
        match self {
            MailState::Unreachable { .. } => "unreachable",
            _ => "up",
        }
    }

    /// Whether an account is configured — or `None`, which is the honest answer when the service
    /// could not be reached.
    ///
    /// `false` would be the original lie in a new place: with nothing answering, this app has not
    /// been told whether an account exists, and saying it has none is a claim it cannot support.
    pub fn has_account(&self) -> Option<bool> {
        match self {
            MailState::Unreachable { .. } => None,
            MailState::NoAccount { .. } => Some(false),
            MailState::Ready { .. } => Some(true),
        }
    }

    /// The failure a person should see on screen and a mind should read in `describe.notice`.
    /// Empty when there is nothing wrong.
    pub fn notice(&self) -> String {
        match self {
            MailState::Unreachable { reason } => {
                format!("The mail service could not be reached: {reason}")
            }
            _ => String::new(),
        }
    }

    /// The one line a caller surveying every window pays for.
    pub fn summary(&self) -> String {
        match self {
            MailState::Unreachable { reason } => {
                format!("Email — the mail service could not be reached: {reason}")
            }
            MailState::NoAccount { config_path, .. } => {
                format!("Email — the mail service is running and no account is configured in {config_path}")
            }
            MailState::Ready { account, .. } => format!("Email — {}", account.email),
        }
    }

    pub fn config_path(&self) -> &str {
        match self {
            MailState::Unreachable { .. } => "",
            MailState::NoAccount { config_path, .. } => config_path,
            MailState::Ready { config_path, .. } => config_path,
        }
    }

    pub fn secrets_are_plaintext(&self) -> bool {
        match self {
            MailState::Unreachable { .. } => false,
            MailState::NoAccount { secrets_are_plaintext, .. } => *secrets_are_plaintext,
            MailState::Ready { secrets_are_plaintext, .. } => *secrets_are_plaintext,
        }
    }

    /// Whether a Google sign-in can be started, in the mail service's own words.
    ///
    /// Unreachable is *not available*, and the note says which of the two reasons it is. The
    /// distinction matters because the two look identical on the screen otherwise: a machine with
    /// no Google OAuth client and a machine whose mail service is not running both draw no
    /// button, and only one of them is fixed by setting a client id.
    pub fn google_sign_in(&self) -> GoogleSignIn {
        match self {
            MailState::Unreachable { .. } => GoogleSignIn {
                available: false,
                note: "The mail service is not running, so this app cannot tell whether Google \
                       sign-in is available on this build."
                    .to_string(),
            },
            MailState::NoAccount { google, .. } => google.clone(),
            MailState::Ready { google, .. } => google.clone(),
        }
    }

    pub fn account_id(&self) -> String {
        match self {
            MailState::Ready { account, .. } => account.id.clone(),
            _ => String::new(),
        }
    }

    pub fn account_name(&self) -> String {
        match self {
            MailState::Ready { account, .. } => account.email.clone(),
            _ => String::new(),
        }
    }
}

/// Turn the service's answer into the state the app is in.
///
/// The `Err` arm is everything between this process and an answer: the shell refusing to start
/// the service, the service not coming up inside its budget, a socket that is not there, a
/// method an older binary does not have. All of them are "unreachable, and here is why" — none
/// of them is "no account".
pub fn decide(answer: Result<AccountsResult, String>) -> MailState {
    match answer {
        Err(reason) => MailState::Unreachable { reason },
        Ok(result) => {
            let google = result.google_sign_in;
            match result.accounts.into_iter().next() {
                None => MailState::NoAccount {
                    config_path: result.config_path,
                    secrets_are_plaintext: result.secrets_are_plaintext,
                    google,
                },
                Some(account) => MailState::Ready {
                    account,
                    config_path: result.config_path,
                    secrets_are_plaintext: result.secrets_are_plaintext,
                    google,
                },
            }
        }
    }
}
