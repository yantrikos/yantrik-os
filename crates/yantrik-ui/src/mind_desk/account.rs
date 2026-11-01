//! Which account a Mind's harness runs under: the kernel's word, not a name list.
//!
//! The desk's safety line says "Own account" only for a harness the kernel showed to be the mind
//! account (`yantrik-mind`) when it attached, and "Runs as you" only for one it showed to be the
//! person. Anything else is `Unknown`, which the desk says as unavailable and never as either
//! comfortable claim. The host keeps the uid of every attached harness (`Host::attached_uid`), and
//! the mind account is the one `mind_door::is_mind` names, so no mind's name is written down here.

use yantrik_ipc_contracts::mind_desk::Account;

/// The account of a harness that attached as `uid`, given who is the mind account and who this
/// shell runs as.
pub fn of(uid: Option<u32>, is_mind: impl Fn(u32) -> bool, person: u32) -> Account {
    match uid {
        Some(uid) if is_mind(uid) => Account::Own,
        Some(uid) if uid == person => Account::Person,
        _ => Account::Unknown,
    }
}

/// The account of the harness `harness`, as the live host recorded it at attach.
#[cfg(unix)]
pub fn of_harness(harness: &str) -> Account {
    let Some(host) = crate::wire::harness::host() else { return Account::Unknown };
    // SAFETY: getuid cannot fail.
    let person = unsafe { libc::getuid() };
    of(host.attached_uid(harness), yantrik_ipc_transport::mind_door::is_mind, person)
}

#[cfg(not(unix))]
pub fn of_harness(_harness: &str) -> Account {
    Account::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_is_what_the_kernel_said_at_attach() {
        let mind = |uid: u32| uid == 990;
        assert_eq!(of(Some(990), mind, 1000), Account::Own);
        assert_eq!(of(Some(1000), mind, 1000), Account::Person);
        // A uid that is neither is not guessed at, and no uid at all is not an answer.
        assert_eq!(of(Some(1234), mind, 1000), Account::Unknown);
        assert_eq!(of(None, mind, 1000), Account::Unknown);
    }

    /// The mind account outranks the person's uid: a harness the kernel shows to be the mind
    /// account is never read as the person because the two numbers were mixed up.
    #[test]
    fn the_mind_account_is_never_read_as_the_person() {
        assert_eq!(of(Some(990), |_| true, 990), Account::Own);
    }
}
