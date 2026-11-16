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

mod copy_marker;
mod kept_places;
mod secrets;
mod stat;
mod titles;
mod tree;
mod verdict;
pub mod spawner;
pub mod workshops;
mod writes;

#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, unix))]
mod protected_tests;
#[cfg(all(test, unix))]
mod regression_tests;
#[cfg(all(test, unix))]
mod copy_form_tests;
#[cfg(all(test, unix))]
mod secret_tests;
#[cfg(all(test, unix))]
mod cores_tests;
#[cfg(all(test, unix))]
mod write_tests;

pub use secrets::{is_a_copy_name, is_secret_name, pattern_asks_for_a_secret, secret_cores, NAME_MAX, SECRET_NAMES, SECRET_PREFIXES, SECRET_SUFFIXES};
pub use stat::stat;
pub use titles::title_names_a_secret;
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
    // Where gpg keeps its private keys; a copy of the folder is as secret as the original.
    "private-keys-v1.d",
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
    // What an update renames the person's old Hermes folder to when it moves it to the Mind's account:
    // the same config, with its gateway key, under another name (gate 12).
    ".hermes.yantrik-moved",
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
    // Browser profiles, which hold the person's saved logins, cookies and history, and the mail
    // programs' accounts (#1247). `run_command` alone kept the browsers; every other side that
    // reads files could reach them.
    ".mozilla",
    ".config/chromium",
    ".config/google-chrome",
    ".config/bravesoftware",
    ".var/app/com.google.chrome",
    ".var/app/org.chromium.chromium",
    ".var/app/com.brave.browser",
    "snap/chromium",
    "snap/firefox",
    ".thunderbird",
    ".config/evolution",
    // Editors keep a whole copy of every saved version under a random name, and their hot-exit
    // text, which no name rule can see (review H2): VS Code, VSCodium and Code - OSS.
    ".config/code/user/history",
    ".config/code/backups",
    ".config/vscodium/user/history",
    ".config/vscodium/backups",
    ".config/code - oss/user/history",
    ".config/code - oss/backups",
    ".var/app/com.visualstudio.code/config/code/user/history",
    // What a shell, a REPL and Vim remember holds pasted tokens, commands that name secret files
    // and yanked text (review M5).
    ".zsh_history",
    ".local/share/fish/fish_history",
    ".python_history",
    ".node_repl_history",
    ".psql_history",
    ".mysql_history",
    ".viminfo",
    ".local/state/nvim/shada",
    // Files that record the names of files: the trash's, the recent-files list and Emacs's list of
    // auto-save files (review L2).
    ".local/share/recently-used.xbel",
    ".emacs.d/auto-save-list",
];

/// The places [`PROTECTED`] keeps under `prefix` (`.config/`, `.local/share/`), without it: what
/// is found under `$XDG_CONFIG_HOME` or `$XDG_DATA_HOME` instead when those are moved. Read from
/// the one list, so a place added there is protected in both spots (`gh` and `gcloud` were
/// missing from a hand copy; security review of #942, finding 4).
fn under_prefix(prefix: &'static str) -> impl Iterator<Item = &'static str> {
    PROTECTED.iter().filter_map(move |place| place.strip_prefix(prefix))
}

/// Whether `path` passes through a protected place, or is a file that holds a credential by its
/// name wherever it is kept (`secrets.rs`), compared a whole component at a time and
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

/// Whether `name` (a last part) is an editor's copy that spells a path in a protected place into
/// it: Neovim's `%home%ann%.kube%config.swp`, Emacs's `!home!ann!.kube!config~` and
/// `#!home!ann!.kube!config#` (review R6-L3). For a window title, which names the file only.
pub fn names_a_protected_copy(name: &str) -> bool {
    copy_marker::encoded_path(name).is_some_and(|original| is_listed_place(&original))
}

/// Whether `path` is in a listed place or has a secret's name, judged on its text alone: no
/// filesystem, no links, no XDG lookups. For a window title, which only names a file and may be
/// judged thousands of times (review R7-H1). What it misses is protected for every reader and shown
/// in a title (review R8-L2):
///
/// 1. a link to a secret (`notes.txt -> tokens.json`): the name is judged, not what it points at;
/// 2. a place moved behind a link (`~/.kube` pointing to `~/dotfiles/kube`);
/// 3. a place the XDG variables relocate (`XDG_CONFIG_HOME=~/cfg` makes `~/cfg/labwc/autostart`);
/// 4. a workshop reached by another route than [`workshops::BASE`] as written (a link to it, a
///    second mount of it, or a root under `XDG_DATA_HOME`): the prefix is matched as text, and the
///    inode test that finds the others reads the filesystem.
///
/// Every reader of contents asks [`is_protected`].
pub fn is_listed_place(path: &Path) -> bool {
    in_a_listed_place(path) || secrets::names_a_secret_by_name(path)
}

fn in_a_listed_place(path: &Path) -> bool {
    // A Mind's workshops, by their fixed prefix as text.
    if path.starts_with(workshops::BASE) {
        return true;
    }
    // The list in lower case, made once: a title is judged by hundreds of words.
    static LOWERED: std::sync::OnceLock<Vec<Vec<String>>> = std::sync::OnceLock::new();
    let places = LOWERED.get_or_init(|| PROTECTED.iter().map(|place| lowered(Path::new(place))).filter(|want| !want.is_empty()).collect());
    let parts = lowered(path);
    places.iter().any(|want| parts.windows(want.len()).any(|w| w == want.as_slice()))
        || kept_places::in_a_harness_home(&parts)
        || kept_places::in_an_object_store(&parts)
        || parts.iter().any(|part| kept_places::is_a_kept_copy(part))
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
    // A Mind's workshops are held by one check, the prefix and the inode (workshops.rs).
    in_a_listed_place(path)
        || secrets::names_a_secret(path)
        // An editor's copy that spells the path of a file into its name (review R5-H1).
        || path.file_name().and_then(|n| copy_marker::encoded_path(&n.to_string_lossy())).is_some_and(|original| is_protected_with(&original, places))
        || places.home.is_some_and(|home| moved(home) || workshops::holds(path, home))
        || places.data_home.is_some_and(|base| workshops::Root::at(&base.join("yantrik/workshops")).holds(path))
        || places.data_home.is_some_and(|base| under_prefix(".local/share/").any(|place| under(base, place)))
        || places.state_home.is_some_and(|base| under(base, "yantrik"))
        || places.config_home.is_some_and(|base| under_prefix(".config/").any(|place| under(base, place)))
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
