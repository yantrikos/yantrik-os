//! Lock screen — PIN authentication + idle lock management.
//!
//! Validates user input against a PIN stored in ~/.yantrik/lock_pin.
//! Creates a default PIN "0000" on first use.
//! Idle lock triggers after configurable timeout (default 5 minutes).

use std::path::PathBuf;

/// Default idle lock timeout in seconds (5 minutes).
pub const DEFAULT_IDLE_LOCK_SECS: u64 = 300;

/// Path to the PIN file.
pub fn pin_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".yantrik/lock_pin")
}

/// Ensure the PIN file exists. Creates with default "0000" if missing.
pub fn ensure_pin_file() {
    let path = pin_path();
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, "0000");
        tracing::info!("Created default lock PIN (0000) at {}", path.display());
    }
}

/// Check if the given input matches the stored PIN.
/// Returns true if authentication succeeds.
pub fn check_pin(input: &str) -> bool {
    let path = pin_path();
    match std::fs::read_to_string(&path) {
        Ok(stored) => stored.trim() == input.trim(),
        Err(_) => {
            // If we can't read the PIN file, allow unlock (fail-open for dev)
            tracing::warn!("Cannot read PIN file — allowing unlock");
            true
        }
    }
}
