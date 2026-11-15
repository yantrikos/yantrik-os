//! What the git tools show of a protected file (#1247, review H2).
//!
//! `validate_path` checks the repository a git tool is given, not what git then prints. A tracked
//! `config/credentials.json` with a local edit came out of `git diff -- <file>` line by line, and
//! `file: "."` or a glob pathspec printed every changed file. Two rules close that, both through
//! the desktop's shared name rule ([`yantrik_ipc_contracts::home_paths::is_protected`]):
//!
//! - [`check_pathspec`] refuses a file argument that is, or matches, a protected path, before
//!   any diff runs: the names git itself would diff are listed first.
//! - [`without_protected`] drops from a status or a stat the lines that name a protected path,
//!   and says how many, never which. Git's quoted paths (`"tokens (1).json"`) are spelled out first
//!   ([`super::git_quote`]). The count tells that something is there, as for `find`
//!   (see `safe_command_listing`).

use std::path::Path;

use yantrik_ipc_contracts::home_paths::is_protected;

const REFUSED: &str = "is, or leads into, a place or file the AI never reads";

/// Whether `file`, a pathspec for `git diff` in the repository `repo`, may be diffed: it is a
/// plain path (no `:(magic)` form, no option, not the whole tree), and no changed file it
/// selects is protected. It fails closed: when git cannot list the selection, the answer is no. Contents are never printed here.
pub fn check_pathspec(repo: &str, file: &str) -> Result<(), String> {
    if file.starts_with([':', '-']) || file == "." {
        return Err(format!("git_diff_file takes the path of one file or folder, not '{file}'"));
    }
    if is_protected(&Path::new(repo).join(file)) {
        return Err(format!("{file} {REFUSED}"));
    }
    // What git would diff for it: a folder, a glob or a renamed file selects other names.
    let listed = super::git_guard::command(repo)
        .args(["diff", "--name-only", "-z", "--relative", "--no-renames", "--", file])
        .output();
    // If git cannot say what the pathspec selects, nothing is diffed (review N6).
    let Ok(listed) = listed.map_err(|_| ()).and_then(|o| o.status.success().then_some(o).ok_or(())) else {
        return Err(format!("git could not list what {file} selects, so it is not diffed"));
    };
    let names = String::from_utf8_lossy(&listed.stdout);
    if names.split('\0').filter(|n| !n.is_empty()).any(|n| is_protected(&Path::new(repo).join(n))) {
        return Err(format!("{file} selects a file the AI never reads, so it is not diffed"));
    }
    Ok(())
}

/// `out`, the answer of a git command in `repo` that names files (`status --short`, the files of
/// a commit), without the lines that name a protected path.
pub fn without_protected(repo: &str, out: &str) -> String {
    let (kept, notice) = split_protected(repo, out, &[]);
    format!("{kept}{notice}")
}

/// [`without_protected`] for a stat: `changed` are the paths the stat is about. A name that git
/// cut from the left (`...tail`) is judged by the changed path it ends: its tail is no secret
/// name by itself (review R3-3).
pub fn without_protected_cut(repo: &str, out: &str, changed: &[String]) -> String {
    let (kept, notice) = split_protected(repo, out, changed);
    format!("{kept}{notice}")
}

/// [`without_protected_cut`] as the lines it keeps and, apart, the line that counts what it hid
/// (empty when nothing was hidden), so a caller that cuts the answer short keeps the count.
pub fn split_protected(repo: &str, out: &str, changed: &[String]) -> (String, String) {
    let mut kept = Vec::new();
    let mut hidden = 0;
    for line in out.lines() {
        // A stat line ends in ` | 3 ++-`; a rename reads `old => new` (stat) or `old -> new` (status).
        // Git quotes a path with a space, a `"`, a `\` or a non-ASCII character; match what it spells.
        let plain = super::git_quote::unquoted(line);
        let names = whole_renames(plain.rsplit_once(" | ").map_or(plain.as_str(), |(names, _)| names));
        if crate::safe_command_listing::names_a_protected_entry(&names, repo) || cut_from_a_protected_path(&names, repo, changed) {
            hidden += 1;
        } else {
            kept.push(line);
        }
    }
    let notice = if hidden > 0 {
        format!("\n({hidden} entries are not shown: each is, or is inside, a place or file the AI never reads)")
    } else {
        String::new()
    };
    (kept.join("\n"), notice)
}

/// Whether `names` (the left side of a stat line) starts with git's `...` and what follows ends a
/// changed path that is protected.
fn cut_from_a_protected_path(names: &str, repo: &str, changed: &[String]) -> bool {
    let Some(tail) = names.trim().strip_prefix("...") else { return false };
    changed.iter().any(|path| path.ends_with(tail) && is_protected(&Path::new(repo).join(path)))
}

/// A stat's `dir/{old => new}/f` as the two whole paths, `dir/old/f -> dir/new/f`; any other
/// `old => new` as `old -> new`.
fn whole_renames(names: &str) -> String {
    if let (Some(open), Some(close)) = (names.find('{'), names.rfind('}')) {
        let (before, inner, after) = (&names[..open], names.get(open + 1..close).unwrap_or(""), &names[close + 1..]);
        if let Some((old, new)) = inner.split_once(" => ") {
            return format!("{before}{old}{after} -> {before}{new}{after}");
        }
    }
    names.replace(" => ", " -> ")
}

#[cfg(test)]
#[path = "git_secrets_tests.rs"]
mod tests;
