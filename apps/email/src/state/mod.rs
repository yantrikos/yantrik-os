//! The half of the Email app that has no window in it.
//!
//! Three things live here, and they are here because each one was wrong in a way that could not
//! be seen from either the screen or the service on its own.
//!
//! **Which of three states the app is in.** A dead service and an unconfigured machine used to
//! be the same picture: `list_folders` returned `None` for both, and `None` meant
//! `set_has_account(false)`, which draws the onboarding form. An audit looked at that form and
//! concluded the machine had no mail account. It may never have been about an account at all —
//! `email-service` is registered `autostart: false` and nothing started it, so every call failed
//! at connect on every machine, every time. [`MailState`] is the distinction, made once.
//!
//! **Which message a caller means.** `open_message which=1` asked for `args["which"].as_str()`,
//! got `""` from a number, and answered `nothing in this folder matches ""` — a sentence with a
//! quoted nothing in it. [`resolve_which`] takes a row number or text.
//!
//! **What happens to a half-written message.** Closing the composer threw it away and Save Draft
//! logged a line. [`Draft`] is a file.
//!
//! Nothing in this file imports Slint or opens a socket, so `tests/email-core` can exercise all
//! of it on a machine with no mailbox.

mod counts;
mod draft;
mod google;
mod mail_state;
mod original;
mod setup;
mod triage;
mod which;

pub use counts::*;
pub use draft::*;
pub use google::*;
pub use mail_state::*;
pub use original::*;
pub use setup::*;
pub use triage::*;
pub use which::*;
