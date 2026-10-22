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
pub use tree::{may_land, may_read_tree};
pub use verdict::{may_create, may_read_file, may_write_file};
pub use writes::{resolve, where_programs_look, HIDDEN_RULE};

/// Places in the home no agent reads or writes: keys, the shell's own configuration and memory,
/// and every file a shell or the session runs as the person without asking - the login and
/// startup scripts, autostart entries, user service units, the environment files, desktop
/// entries (which run their `Exec=` when clicked), and `mimeapps.list`, which decides which of
/// them opens a file. Written relative to the home, one or more whole path components each, in
/// lower case. The same places under `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME` and `$XDG_STATE_HOME`,
/// when those are set elsewhere, are protected too; see [`is_protected`].
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
    // The person's screenshots and screen recordings (design/capture-2026-10-15.md). Each is a
    // picture of their desktop, which a mind looks at only as it reads the screen: asked every
    // time, and tainted as a screen read. Readable here, `files_open` of one in Mind View and a
    // safe `read_mind_view` was every desktop shot ever taken with no card at all (security
    // review of #828, finding 1). The shell's `open_capture` is a mind's one way to one.
    "pictures/screenshots",
    "videos/recordings",
    // Yantrik's own data and state, as whole trees: every app's store, exports and drafts (the
    // snippets library, Email's draft and its unsent mail, every Notes draft), the minds' and
    // agents' sessions, the gateway's records. Each app's data is reached through that app's own
    // door, which applies its grades and Private mode; read as a file, the snippets store was
    // every snippet's code as a `safe` editor `open` (security review of #940, High 1).
    ".local/share/yantrik",
    ".local/state/yantrik",
    // The Mind's folders from before `yantrik-update migrate-minds`: its database, the egress key
    // and the console and memory tokens, which `shared_memory.rs` still reads from there, and
    // the environment file its service starts from (security review of #942, finding 4).
    ".local/share/yantrik-mind",
    ".config/yantrik-mind",
    ".config/yantrik-mind.env",
];

/// What `.config/...` in [`PROTECTED`] names, found under `$XDG_CONFIG_HOME` instead.
const CONFIG_PLACES: &[&str] = &[
    "autostart",
    "systemd",
    "environment.d",
    "labwc",
    "yantrik",
    "mimeapps.list",
    "yantrik-mind",
    "yantrik-mind.env",
];

/// What `.local/share/...` in [`PROTECTED`] names, found under `$XDG_DATA_HOME` instead.
const DATA_PLACES: &[&str] = &["applications", "yantrik", "yantrik-mind", "keyrings"];

/// Whether `path` passes through a protected place, compared a whole component at a time and
/// without regard to case (a case-folding filesystem makes `.SSH` the same folder): `.ssh` is
/// protected, `.ssh-notes` is not, and `.config/labwc` only as those two in a row.
///
/// The session reads desktop entries from `$XDG_DATA_HOME/applications` and its configuration
/// from `$XDG_CONFIG_HOME`, and Yantrik keeps its own data and state in `$XDG_DATA_HOME/yantrik`
/// and `$XDG_STATE_HOME/yantrik`; a session that moves any of them elsewhere would otherwise
/// leave the real place open, and a const list cannot name a path from the environment, so it is
/// asked here.
///
/// A protected place reached through a link is protected where it really is, too: a dotfile
/// manager that makes `~/.local` a link to `~/dotfiles/local` leaves the snippets store at
/// `~/dotfiles/local/share/yantrik` as well, and the list, matched as written, would know only
/// its alias (security review of #942, finding 3).
pub fn is_protected(path: &Path) -> bool {
    is_protected_in(path, None)
}

/// [`is_protected`] for a path asked about in `home`, which the person's `$HOME` is otherwise.
pub(crate) fn is_protected_in(path: &Path, home: Option<&Path>) -> bool {
    let from_env = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let env_home = from_env("HOME");
    is_protected_with(
        path,
        &Places {
            home: home.or(env_home.as_deref()),
            data_home: from_env("XDG_DATA_HOME").as_deref(),
            config_home: from_env("XDG_CONFIG_HOME").as_deref(),
            state_home: from_env("XDG_STATE_HOME").as_deref(),
        },
    )
}

/// Where the session's home and XDG base directories are, when they are set: absolute paths, or
/// none.
#[derive(Debug, Clone, Copy, Default)]
pub struct Places<'a> {
    pub home: Option<&'a Path>,
    pub data_home: Option<&'a Path>,
    pub config_home: Option<&'a Path>,
    pub state_home: Option<&'a Path>,
}

/// [`is_protected`] with the home and the XDG base directories given.
pub fn is_protected_with(path: &Path, places: &Places<'_>) -> bool {
    let parts = lowered(path);
    let listed = PROTECTED.iter().any(|place| {
        let want = lowered(Path::new(place));
        !want.is_empty() && parts.windows(want.len()).any(|w| w == want.as_slice())
    });
    let under = |base: &Path, place: &str| {
        // As written and as resolved, since the path may be either.
        let at = base.join(place);
        starts_with_folded(path, &at) || resolved(&at).is_some_and(|real| starts_with_folded(path, &real))
    };
    // Matched as written above; here, where each place in the home really is. Only a place whose
    // first part is there can have moved, which keeps this to a few lookups in a common home.
    let moved = |home: &Path| {
        PROTECTED.iter().any(|place| {
            let first = place.split('/').next().unwrap_or(place);
            home.join(first).symlink_metadata().is_ok()
                && resolved(&home.join(place)).is_some_and(|real| starts_with_folded(path, &real))
        })
    };
    listed
        || places.home.is_some_and(moved)
        || places.data_home.is_some_and(|base| DATA_PLACES.iter().any(|place| under(base, place)))
        || places.state_home.is_some_and(|base| under(base, "yantrik"))
        || places.config_home.is_some_and(|base| CONFIG_PLACES.iter().any(|place| under(base, place)))
}

/// Where `at` really is: its deepest part that exists, resolved through every link, with the
/// rest joined on. A protected folder not made yet is still protected where it would be made.
fn resolved(at: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut base = at;
    loop {
        if let Ok(real) = base.canonicalize() {
            return Some(rest.iter().rev().fold(real, |p, part| p.join(part)));
        }
        rest.push(base.file_name()?);
        base = base.parent()?;
    }
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
