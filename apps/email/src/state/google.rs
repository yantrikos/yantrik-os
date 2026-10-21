//! The Google sign-in, as the window sees it.

use yantrik_ipc_contracts::email::OAuthStatus;

/// What the setup screen does with one answer from `email.oauth_status`.
///
/// The polling loop asks a question every second and most answers are "not yet". This is the
/// decision about which of them ends the wait, made once and in a place with no Slint in it —
/// because the failure mode it exists to stop is a window that sits on "Waiting for Google…"
/// forever because the answer that would have ended it was not one of the shapes the loop knew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleOutcome {
    /// Still going. Ask again.
    Waiting,
    /// Signed in, and the account is in the service's file. Carries the address it signed in as
    /// — which came from Google, not from anything typed into this window.
    SignedIn(String),
    /// Over, and not signed in. The sentence goes on the screen *and* into `describe.notice`,
    /// because a sign-in that quietly stopped is the same silence this app spent September
    /// removing from everything else.
    Stopped(String),
}

/// Turn one status answer into that decision.
///
/// The `Err` arm is everything between this window and an answer: the service went away, the
/// socket closed, the method is missing on an older binary, or the service has forgotten a flow it
/// once had. It is a stop rather than a retry, because the alternative is a window polling a
/// service that cannot answer until its own deadline and then saying nothing useful about why.
///
/// The wrapper is worded for both — a refusal *is* an answer — so the sentence is true whichever
/// of the two happened, and the service's own words come after it either way.
pub fn google_outcome(answer: Result<OAuthStatus, String>) -> GoogleOutcome {
    match answer {
        Ok(OAuthStatus::Waiting) => GoogleOutcome::Waiting,
        Ok(OAuthStatus::Done { account }) => GoogleOutcome::SignedIn(account.email),
        Ok(OAuthStatus::Failed { reason }) => GoogleOutcome::Stopped(reason),
        Err(e) => GoogleOutcome::Stopped(format!(
            "The mail service could not say how the Google sign-in went: {e}"
        )),
    }
}

/// What the screen says when the app could not open a browser for the consent page.
///
/// Not a dead end and not a log line: the address is put on the screen beside this, because on a
/// machine with no `xdg-open` handler that is the only way through — and a person who can read
/// the URL can finish the sign-in from a phone.
pub fn browser_failed_note(reason: &str) -> String {
    format!(
        "This machine could not open a browser for the Google sign-in ({reason}). The address is \
         below \u{2014} open it anywhere signed in to the right Google account, and this window \
         will notice when it comes back."
    )
}
