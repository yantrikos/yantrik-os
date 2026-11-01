//! The companion's side of mail.
//!
//! There is one mail stack (#1022): `email-service` owns the accounts, and signs in with each
//! account's secret from the credential vault for the length of one call. This module holds no
//! secret and speaks no IMAP or SMTP.
//!
//! - [`port`]: the mind's way to the service, as a trait the tools and the tests share.
//! - [`db`]: the companion's cache of what the service last listed (addresses and subjects, never a
//!   secret), which the mind's message numbers and the VIP-mail playbook rest on.
//! - [`legacy`]: moves the mail accounts an older companion kept in `config.yaml`, password and
//!   all, into the service and the vault, and archives the file it changed.

pub mod approval;
pub mod db;
mod flight;
pub mod legacy;
pub mod port;

#[cfg(test)]
pub mod fake;

#[cfg(test)]
mod port_tests;
