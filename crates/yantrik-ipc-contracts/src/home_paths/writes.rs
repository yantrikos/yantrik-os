//! The rule for an agent's writes that no list could be: nowhere hidden in the home.
//!
//! The protected list names what runs as the person, and it can never be complete: git runs
//! `core.pager` and `core.fsmonitor` from ~/.gitconfig and from any repository's .git/config,
//! vim runs ~/.vimrc, cargo runs what ~/.cargo/config.toml says, tmux its ~/.tmux.conf, and
//! whatever is in ~/.local/bin or ~/bin runs in place of the command the person typed. They have
//! one thing in common: programs keep their settings and startup in the hidden files and folders
//! of the home, and ~/bin. So an agent writes into none of them - a new file, a saved file, a
//! folder, a rename, a pasted or moved tree - wherever a link would take the path. Reading keeps
//! the protected list alone: a dotfile that is not protected may be read.

use std::path::{Component, Path, PathBuf};

/// Why an agent's write into a hidden place is refused.
pub const HIDDEN_RULE: &str =
    "an agent does not write into hidden folders or dotfiles in the home, where programs read their settings and startup";

/// Whether `path` is somewhere in `home` that programs read their settings or startup from: a
/// component, below the home, that starts with `.`, or `bin` directly in the home. Checked as
/// written and where it resolves (the deepest part that exists, with the rest joined on), so a
/// link `~/x -> ~/.config` does not make `~/x/git/config` an ordinary folder.
pub fn where_programs_look(path: &Path, home: &Path) -> bool {
    hidden_below(path, home)
        || match (resolve(path), home.canonicalize()) {
            (Some(real), Ok(real_home)) => hidden_below(&real, &real_home),
            _ => false,
        }
}

fn hidden_below(path: &Path, home: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(home) else { return false };
    let parts: Vec<String> = rest
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    parts.first().is_some_and(|first| first == "bin") || parts.iter().any(|part| part.starts_with('.'))
}

/// Where `path` really goes: the deepest part of it that exists, links followed, with the rest
/// joined on. `None` when no part of it resolves.
pub fn resolve(path: &Path) -> Option<PathBuf> {
    let mut probe = path.to_path_buf();
    loop {
        if let Ok(real) = probe.canonicalize() {
            let tail = path.strip_prefix(&probe).unwrap_or(Path::new(""));
            return Some(real.join(tail));
        }
        probe = probe.parent()?.to_path_buf();
    }
}
