//! Where in the person's home an agent may look and write: one rule for every side that asks.
//!
//! The shell's Files screen, its `files_stat`, the Text Editor, the Image Viewer and the file
//! tools each decided this on their own, from their own copy of a protected list, and the copies
//! drifted: the editor had no rule at all, so a mind could `open ~/.ssh/id_ed25519` and read it
//! back through `describe`, or `save_as ~/.bashrc` and run code as the person at their next
//! login. The list and the walk that applies it live here, in the leaf crate all of them already
//! depend on, so a place protected for one is protected for all.
//!
//! Two things make the rule hold against links. Where a path goes is decided by the deepest part
//! of it that resolves, so a link to /etc cannot be used to ask, a directory at a time, what /etc
//! holds. And the part below that, which does not exist yet, is checked too, joined to where the
//! rest resolved: a link `~/x/c -> ~/.config` makes `~/x/c/labwc/autostart` a write into
//! `~/.config/labwc`, a protected place split across the link.
//!
//! Writes have one more rule, because no list of protected places can be complete: an agent
//! writes nowhere hidden in the home (writes.rs).

use std::path::{Component, Path, PathBuf};

mod stat;
mod tree;
mod verdict;
mod writes;

#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, unix))]
mod write_tests;

pub use stat::stat;
pub use tree::may_land;
pub use verdict::{may_create, may_read_file, may_write_file};
pub use writes::{resolve, where_programs_look, HIDDEN_RULE};

/// Places in the home no agent reads or writes: keys, the shell's own configuration and memory,
/// and every file a shell or the session runs as the person without asking - the login and
/// startup scripts, autostart entries, user service units, the environment files, desktop
/// entries (which run their `Exec=` when clicked), and `mimeapps.list`, which decides which of
/// them opens a file. Written relative to the home, one or more whole path components each, in
/// lower case. The same places under `$XDG_CONFIG_HOME` and `$XDG_DATA_HOME`, when those are set
/// elsewhere, are protected too; see [`is_protected`].
///
/// The file tools in `yantrik-companion-core` add places outside the home to this (their
/// BLOCKED_SEGMENTS); this is the part every side shares.
pub const PROTECTED: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".config/labwc",
    ".config/yantrik",
    "memory.db",
    ".bashrc",
    ".profile",
    ".bash_history",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".pam_environment",
    ".config/autostart",
    ".config/environment.d",
    ".config/systemd",
    ".local/share/applications",
    ".config/mimeapps.list",
    // Credentials. The list above was built against code that runs as the person; it left every
    // token and password file in the home readable to any agent's `open`, and a safe editor
    // `read` would have handed them to phone turns unasked (security review of #617, 4 October).
    ".aws",
    ".azure",
    ".config/gcloud",
    ".config/gh",
    ".docker/config.json",
    ".kube",
    ".git-credentials",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".vault-token",
    ".terraform.d/credentials.tfrc.json",
    ".password-store",
    ".local/share/keyrings",
    // The agent harnesses' own state, which carries their API keys and session credentials.
    ".claude",
    ".claude.json",
    ".hermes",
    ".openclaw",
    ".pi",
];

/// What `.config/...` in [`PROTECTED`] names, found under `$XDG_CONFIG_HOME` instead.
const CONFIG_PLACES: &[&str] = &["autostart", "systemd", "environment.d", "labwc", "yantrik", "mimeapps.list"];

/// Whether `path` passes through a protected place, compared a whole component at a time and
/// without regard to case (a case-folding filesystem makes `.SSH` the same folder): `.ssh` is
/// protected, `.ssh-notes` is not, and `.config/labwc` only as those two in a row.
///
/// The session reads desktop entries from `$XDG_DATA_HOME/applications` and its configuration
/// from `$XDG_CONFIG_HOME`, and a session that moves either elsewhere would otherwise leave the
/// real place open; a const list cannot name a path from the environment, so it is asked here.
pub fn is_protected(path: &Path) -> bool {
    let from_env = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    is_protected_with(path, from_env("XDG_DATA_HOME").as_deref(), from_env("XDG_CONFIG_HOME").as_deref())
}

/// [`is_protected`] with the XDG data and config homes given, or none.
pub fn is_protected_with(path: &Path, xdg_data_home: Option<&Path>, xdg_config_home: Option<&Path>) -> bool {
    let parts = lowered(path);
    let listed = PROTECTED.iter().any(|place| {
        let want = lowered(Path::new(place));
        !want.is_empty() && parts.windows(want.len()).any(|w| w == want.as_slice())
    });
    let under = |base: &Path, place: &str| {
        // As written and as resolved, since the path may be either.
        let at = base.join(place);
        starts_with_folded(path, &at) || at.canonicalize().is_ok_and(|real| starts_with_folded(path, &real))
    };
    listed
        || xdg_data_home.is_some_and(|xdg| under(xdg, "applications"))
        || xdg_config_home.is_some_and(|xdg| CONFIG_PLACES.iter().any(|place| under(xdg, place)))
}

fn lowered(path: &Path) -> Vec<String> {
    path.iter().map(|part| part.to_string_lossy().to_lowercase()).collect()
}

fn starts_with_folded(path: &Path, base: &Path) -> bool {
    let (path, base) = (lowered(path), lowered(base));
    path.len() >= base.len() && path[..base.len()] == base[..]
}

/// `asked` as an absolute path: `~` and `~/...` are the home. Relative paths, other users'
/// `~name`, any `..` and a NUL are not paths this answers for. Rebuilt from its components, so a
/// trailing slash or a doubled one does not change what is asked about.
pub fn expand(asked: &str, home: &Path) -> Option<PathBuf> {
    if asked.contains('\0') {
        return None;
    }
    let path = if asked == "~" {
        home.to_path_buf()
    } else if let Some(rest) = asked.strip_prefix("~/") {
        if rest.starts_with('/') {
            return None;
        }
        home.join(rest)
    } else {
        PathBuf::from(asked)
    };
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    Some(path.components().collect())
}
