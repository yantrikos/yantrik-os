//! The credential vault as the OS's programs reach it: one trait, one way to keep a secret, one
//! way to wait for the vault to open.
//!
//! The vault itself lives in the companion's database inside the shell. Everything else holds a
//! [`KeyVault`]: the shell's saved AI providers (`provider_vault`), the shell's door for the mail
//! service (`credential_door`), and the mail service's accounts, which reach the shell through that
//! door. They share three rules, written once here:
//!
//! - A secret is *kept* only when the vault gives back the same text ([`keep`]). A write that
//!   reports success and reads back something else is a failure.
//! - A shut vault ([`VaultError::Locked`]) and a vault that did not answer ([`VaultError::Failed`])
//!   are different, because the first is waited out by the person typing a passphrase and the
//!   second is not.
//! - Whatever was refused because the vault was shut is run again when it opens ([`opening`]).
//!
//! Nothing here logs or prints a value, and the types that hold one print `<redacted>`.

pub mod memory;
pub mod opening;

#[cfg(test)]
mod tests;

/// Why the vault did not do what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultError {
    /// The vault has a passphrase and is shut.
    Locked,
    /// The vault refused, did not answer, or gave back something other than what was stored.
    Failed,
}

/// The vault as a caller needs it. Each program supplies its own: the shell's goes through the
/// companion's worker, the mail service's through the shell's door, and a test's is in memory
/// ([`memory::MemoryVault`]).
pub trait KeyVault: Send + Sync {
    fn store(&self, id: &str, value: &str) -> Result<(), VaultError>;
    /// The value kept under `id`, or `None` when nothing is.
    fn value(&self, id: &str) -> Result<Option<String>, VaultError>;
    fn remove(&self, id: &str) -> Result<(), VaultError>;
}

/// Keep `value` under `id` and read it back: Ok only when the vault gives the same text.
pub fn keep(vault: &dyn KeyVault, id: &str, value: &str) -> Result<(), VaultError> {
    vault.store(id, value)?;
    match vault.value(id)? {
        Some(back) if back == value => Ok(()),
        _ => Err(VaultError::Failed),
    }
}
