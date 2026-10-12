//! Getting the window up before the mail is.
//!
//! The first look at the mail service ran on the UI thread before the window existed: start the
//! service (up to two seconds), then list the folders and the inbox, each an IMAP round trip of
//! up to ten. On the test VM the window came up eight seconds after `open_app email` was
//! accepted, and for those eight seconds nothing on screen said Email was coming — the window in
//! front stayed in front, and it looked as if the wrong app had opened.
//!
//! So the window is shown first, saying it is starting, and the look runs on a worker in two
//! halves: which account (local and quick once the service is up), then that account's mailbox.
//! The screen says "Connecting to <account>…" between them, with the folder's counts not known
//! yet rather than the 0 of 0 the header holds before anything has been read (#131).

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use slint::ComponentHandle;

use crate::state::MailState;
use crate::{EmailApp, Loaded};

/// What the first look has found so far, in the order the screen is told it.
pub enum Stage {
    /// The account is known and its mailbox has not answered.
    Connecting(MailState),
    /// The look is over, whichever way it went. Told exactly once.
    Done(Loaded),
}

/// The first look, start to finish, on the thread that calls it (a worker): `ask` which account,
/// then, when there is one, `open` its mailbox, telling `tell` each [`Stage`] as it is reached.
///
/// A look that panics still ends in `Done`, as the mail service unreachable: without that the
/// window would say "Starting…" for as long as it stayed open, with Refresh held off behind it.
pub fn run_first_look(
    ask: impl FnOnce() -> MailState,
    open: impl FnOnce(MailState) -> Loaded,
    tell: impl Fn(Stage),
) {
    let looked = catch_unwind(AssertUnwindSafe(|| {
        let state = ask();
        if state.has_account() == Some(true) {
            tell(Stage::Connecting(state.clone()));
        }
        open(state)
    }));
    tell(Stage::Done(looked.unwrap_or_else(|_| Loaded::unreachable(PANICKED))));
}

/// The reason given when the first look failed by panicking rather than with an answer.
pub const PANICKED: &str = "the first look at the mail service failed inside Email (see its log)";

/// Log how long the window took to draw for the first time, once.
///
/// From `started`, which `main` takes as its first act, so the number is process start to a
/// window on screen — the thing a person waits through.
pub fn log_first_frame(app: &EmailApp, started: Instant) {
    let logged = Cell::new(false);
    let notifier = app.window().set_rendering_notifier(move |state, _| {
        if matches!(state, slint::RenderingState::AfterRendering) && !logged.replace(true) {
            first_frame(started);
        }
    });
    // The software renderer has no notifier. The event loop's first turn is the window shown.
    if notifier.is_err() {
        slint::Timer::single_shot(std::time::Duration::ZERO, move || first_frame(started));
    }
}

fn first_frame(started: Instant) {
    tracing::info!(ms = started.elapsed().as_millis() as u64, "email: first frame");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Counted, FolderCounts};
    use std::cell::RefCell;
    use yantrik_ipc_contracts::email::{EmailAccountSummary, EmailFolder, GoogleSignIn};

    fn ready() -> MailState {
        MailState::Ready {
            account: EmailAccountSummary {
                id: "a1".into(),
                email: "you@example.com".into(),
                display_name: String::new(),
                provider: String::new(),
                imap_server: "imap.example.com".into(),
                imap_port: 993,
                smtp_server: "smtp.example.com".into(),
                smtp_port: 587,
                uses_oauth: false,
            },
            config_path: "/etc/mail.toml".into(),
            secrets_are_plaintext: false,
            google: GoogleSignIn::default(),
        }
    }

    /// The stages one first look tells, in order.
    fn stages(
        ask: impl FnOnce() -> MailState,
        open: impl FnOnce(MailState) -> Loaded,
    ) -> Vec<Stage> {
        let told = RefCell::new(Vec::new());
        run_first_look(ask, open, |stage| told.borrow_mut().push(stage));
        told.into_inner()
    }

    /// Starting, then Connecting with the counts not known, then Ready with the server's counts.
    #[test]
    fn an_account_is_connecting_and_then_ready_with_real_counts() {
        let told = stages(ready, |state| Loaded {
            state,
            folders: vec![EmailFolder::counted("INBOX".to_string(), 3, 35)],
            messages: Vec::new(),
            mailbox_error: None,
        });
        assert_eq!(told.len(), 2);
        let Stage::Connecting(state) = &told[0] else { panic!("connecting first") };
        assert_eq!(state.account_name(), "you@example.com");
        assert_eq!(Counted::connecting().known(), None, "no 0 of 0 while connecting");
        let Stage::Done(loaded) = &told[1] else { panic!("done last") };
        assert!(matches!(loaded.state, MailState::Ready { .. }));
        assert_eq!(
            Counted::of(&loaded.folders, "INBOX", 0, 0).known(),
            Some(FolderCounts { unread: 3, total: 35 })
        );
    }

    /// No account: nothing to connect to, so the look goes straight to its answer.
    #[test]
    fn no_account_is_not_connecting() {
        let none = || MailState::NoAccount {
            config_path: String::new(),
            secrets_are_plaintext: false,
            google: GoogleSignIn::default(),
        };
        let told = stages(none, |state| Loaded::of(state));
        assert_eq!(told.len(), 1);
        assert!(matches!(&told[0], Stage::Done(l) if l.state.has_account() == Some(false)));
    }

    /// A look that panics, before or after the account is known, ends Unreachable rather than
    /// leaving the window starting for ever.
    #[test]
    fn a_look_that_panics_ends_unreachable() {
        let told = stages(|| panic!("the service answered nonsense"), |state| Loaded::of(state));
        assert_eq!(told.len(), 1);
        let Stage::Done(loaded) = &told[0] else { panic!("done") };
        assert_eq!(loaded.state, MailState::Unreachable { reason: PANICKED.into() });

        let told = stages(ready, |_| panic!("the mailbox answered nonsense"));
        assert!(matches!(told[0], Stage::Connecting(_)));
        assert!(matches!(&told[1], Stage::Done(l) if l.state.service_word() == "unreachable"));
    }

    /// Before the service answers the app is starting: not unreachable, and not "no account".
    #[test]
    fn starting_is_neither_a_failure_nor_no_account() {
        let starting = MailState::Starting;
        assert_eq!(starting.service_word(), "starting");
        assert_eq!(starting.has_account(), None);
        assert!(starting.notice().is_empty(), "nothing has gone wrong yet");
    }
}
