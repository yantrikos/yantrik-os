//! A vault in memory, for tests of everything that keeps a secret in the real one.
//!
//! It can be shut, can refuse to write, can fail to answer, and can hand back text other than
//! what was stored: the four ways the real one disappoints a caller. Never used outside tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::{KeyVault, VaultError};

#[derive(Default)]
pub struct MemoryVault {
    held: Mutex<HashMap<String, String>>,
    shut: AtomicBool,
    refuse_writes: AtomicBool,
    silent: AtomicBool,
    garble: AtomicBool,
    writes: AtomicUsize,
}

impl MemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shut the vault, or open it.
    pub fn shut(&self, on: bool) {
        self.shut.store(on, Ordering::SeqCst);
    }

    /// Refuse every write, as a vault with a failing disk does.
    pub fn refuse_writes(&self, on: bool) {
        self.refuse_writes.store(on, Ordering::SeqCst);
    }

    /// Not answer reads, as a worker still starting does not.
    pub fn go_silent(&self, on: bool) {
        self.silent.store(on, Ordering::SeqCst);
    }

    /// Keep something other than what was stored.
    pub fn garble(&self, on: bool) {
        self.garble.store(on, Ordering::SeqCst);
    }

    /// Whether anything is kept under `id`, whatever state the vault is in.
    pub fn has(&self, id: &str) -> bool {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).contains_key(id)
    }

    /// What is kept under `id`, whatever state the vault is in.
    pub fn peek(&self, id: &str) -> Option<String> {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    /// How many writes have been accepted.
    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    fn check_open(&self) -> Result<(), VaultError> {
        if self.shut.load(Ordering::SeqCst) {
            return Err(VaultError::Locked);
        }
        Ok(())
    }
}

impl KeyVault for MemoryVault {
    fn store(&self, id: &str, value: &str) -> Result<(), VaultError> {
        self.check_open()?;
        if self.refuse_writes.load(Ordering::SeqCst) {
            return Err(VaultError::Failed);
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        let kept = if self.garble.load(Ordering::SeqCst) { format!("{value}-changed") } else { value.to_string() };
        self.held.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), kept);
        Ok(())
    }

    fn value(&self, id: &str) -> Result<Option<String>, VaultError> {
        self.check_open()?;
        if self.silent.load(Ordering::SeqCst) {
            return Err(VaultError::Failed);
        }
        Ok(self.peek(id))
    }

    fn remove(&self, id: &str) -> Result<(), VaultError> {
        self.check_open()?;
        self.held.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        Ok(())
    }
}
