//! Where a program keeps its files on this OS: the XDG base directories, with their fallbacks,
//! implemented once (design/app-framework-2026-10-21.md, §6). Every path rule above this
//! (`yantrik_app::paths`) is built from these four.

use std::path::PathBuf;

use crate::place::home;

fn base(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var).filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home().join(fallback),
    }
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
pub fn config_home() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME`, else `~/.local/share`.
pub fn data_home() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub fn state_home() -> PathBuf {
    base("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CACHE_HOME`, else `~/.cache`.
pub fn cache_home() -> PathBuf {
    base("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_RUNTIME_DIR`, when the session set one. Sockets and other transient coordination live
/// under it; nothing durable does.
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallbacks_hang_off_home() {
        // The variables are process-wide; this reads whatever the machine has and only checks
        // the shape, so it holds under any environment and beside any other test.
        for dir in [config_home(), data_home(), state_home(), cache_home()] {
            assert!(dir.is_absolute() || dir.starts_with(home()), "{}", dir.display());
        }
    }
}
