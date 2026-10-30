//! The shell's credential door: how a service that has no vault of its own keeps a secret in the
//! shell's.
//!
//! The credential vault lives in the companion's database inside the shell. A service is another
//! process and must not link that database, so the shell publishes a small door on the service bus
//! under [`SERVICE_ID`]. Today its one caller is the mail service, which keeps each account's
//! password and OAuth tokens here instead of in `email.json`.
//!
//! What the door guarantees, and where each guarantee is kept:
//!
//! - **Only the mail service gets in.** The shell reads the caller's program from the kernel
//!   (`SO_PEERCRED`, `/proc/<pid>/exe`) and answers `email-service` and nothing else. The socket is
//!   mode 0600 in a 0700 directory, so another user cannot reach it either.
//! - **Only the shell is told a secret.** The mail service reads the kernel's account of whoever
//!   answers before it sends one byte (`SyncRpcClient::expecting_peer`), so a process that bound
//!   the socket first never sees a password.
//! - **Only mail secrets.** An id must start with [`MAIL_PREFIX`]. The door cannot be used to read
//!   a saved AI provider's key or a free tier's.
//! - **A shut vault is an answer, not a failure**: [`ERR_LOCKED`].
//! - **Nothing is logged or printed**: a value is never in a log line, an error or a `Debug`.

/// The door's id on the service bus.
pub const SERVICE_ID: &str = "credentials";

/// The names of the door's JSON-RPC methods.
pub mod method {
    /// `{ id, value }` keeps a value, and answers `{}` once the vault has it.
    pub const STORE: &str = "credentials.store";
    /// `{ id }` answers `{ value }`, with `null` when nothing is kept under the id.
    pub const VALUE: &str = "credentials.value";
    /// `{ id }` forgets a value, and answers `{}` whether or not there was one.
    pub const REMOVE: &str = "credentials.remove";
}

/// The vault has a passphrase and is shut. The caller waits for it to open; it does not retry in a
/// loop. (A server-defined code in JSON-RPC's reserved range, beside the mail service's -32001.)
pub const ERR_LOCKED: i32 = -32010;
/// The vault did not answer, refused, or the shell is not ready for it yet.
pub const ERR_UNAVAILABLE: i32 = -32011;
/// The door does not answer this caller, or this id. Not a vault state: asking again changes
/// nothing.
pub const ERR_REFUSED: i32 = -32012;

/// What every id the door answers starts with.
pub const MAIL_PREFIX: &str = "mail:";

/// The vault id of one mail account's secrets.
pub fn mail_id(account_id: &str) -> String {
    format!("{MAIL_PREFIX}{account_id}")
}

/// Whether the door may keep, read or forget `id`: a mail id with something after the prefix, made
/// of printable characters.
pub fn is_mail_id(id: &str) -> bool {
    id.strip_prefix(MAIL_PREFIX).is_some_and(|rest| !rest.is_empty())
        && id.len() <= 256
        && id.chars().all(|c| !c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mail_id_is_the_prefix_and_the_account() {
        assert_eq!(mail_id("a-example-org"), "mail:a-example-org");
        assert!(is_mail_id(&mail_id("a-example-org")));
    }

    #[test]
    fn nothing_but_a_mail_id_passes() {
        for bad in ["", "mail:", "saved:p1", "groq", "cloudflare_account", "Mail:a", " mail:a", "mail:a\nb", "mail:\u{0}"] {
            assert!(!is_mail_id(bad), "{bad:?}");
        }
        assert!(!is_mail_id(&format!("mail:{}", "a".repeat(300))), "an id of a size no account has");
    }
}
