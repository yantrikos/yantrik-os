//! What logind will do on this machine: today, whether it will hibernate.
//!
//! Suspend, restart and power off are offered everywhere logind runs, but Hibernate needs swap
//! big enough to hold memory and a resume device, and most images (the QEMU VMs among them) have
//! neither. logind knows, and says so through `CanHibernate`; the power menu draws Hibernate only
//! when it answers "yes". Asking uses the same 2 s bus connection as the power-profile code, so a
//! wedged logind is an answer of "no", never a hang.

/// What logind answers when the machine can hibernate. "challenge" means it would first ask for
/// authentication, which the shell has no agent to answer, so it is not an offer.
const YES: &str = "yes";

/// Whether an answer from `CanHibernate` (or `CanSuspend`, …) means the action can be offered.
pub fn offered(answer: &str) -> bool {
    answer == YES
}

/// Whether logind will hibernate this machine now. Blocks on the system bus for up to 2 s: call
/// it from a worker, never from the thread that draws.
pub fn can_hibernate() -> bool {
    let Ok(conn) = crate::power_profile::system_bus() else { return false };
    conn.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        "CanHibernate",
        &(),
    )
    .ok()
    .and_then(|msg| msg.body().deserialize::<String>().ok())
    .is_some_and(|answer| offered(&answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "challenge" is logind asking for a password the shell cannot give, "na" is no swap or no
    /// resume device. Neither may put a Hibernate row on screen that then does nothing.
    #[test]
    fn only_a_plain_yes_offers_hibernate() {
        assert!(offered("yes"));
        for no in ["no", "na", "challenge", "", "YES"] {
            assert!(!offered(no), "{no:?} is not an offer");
        }
    }
}
