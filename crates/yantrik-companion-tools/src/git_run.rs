//! Running git for the git tools, and what of its answer they hand back (#1247).
//!
//! The answer is cut to [`LIMIT`] characters only after the lines that name a protected path are
//! dropped (review R3-4): cutting first could leave half a secret name as the last line, which no
//! name rule recognises. A stat is asked for at a width that never shortens a name (review R3-3).

use super::git_secrets::split_protected;

/// The most characters of a git answer a tool returns.
const LIMIT: usize = 3000;

/// A stat width and name width that never cut a path from the left (`...tail`): longer than any
/// path the system allows.
const STAT: &str = "--stat=10000,10000";

/// Run a git command in a validated directory; its whole answer, or an error text.
fn run_raw(dir: &str, git_args: &[&str]) -> String {
    // Where git really goes is checked first: a link or a .git file can lead into a workshop.
    if let Err(e) = super::git_guard::vet(dir) {
        return format!("Error: {e}");
    }
    match super::git_guard::command(dir).args(git_args).output() {
        Ok(o) if o.status.success() => {
            let out = String::from_utf8_lossy(&o.stdout);
            if out.trim().is_empty() { "(no output)".to_string() } else { out.into_owned() }
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            format!("{} {}", out.trim(), err.trim())
        }
        Err(e) => format!("Error (git not available?): {e}"),
    }
}

/// `out` cut to [`LIMIT`] characters, with its full length said when it was cut.
fn capped(out: String) -> String {
    if out.len() > LIMIT {
        format!("{}...\n(truncated, {} chars)", &out[..out.floor_char_boundary(LIMIT)], out.len())
    } else {
        out
    }
}

/// Run a git command that prints no file names.
pub fn run_git(dir: &str, git_args: &[&str]) -> String {
    capped(run_raw(dir, git_args))
}

/// [`run_git`] for a command that names files (`status`, `commit`, `stash`): a protected path is
/// left out of what it prints.
pub fn run_git_names(dir: &str, git_args: &[&str]) -> String {
    names_hidden(dir, &run_raw(dir, git_args), &[])
}

/// `shown` without the lines that name a protected path, cut to [`LIMIT`], and then the count of
/// what was hidden: the cut never takes the count.
fn names_hidden(dir: &str, shown: &str, changed: &[String]) -> String {
    let (kept, notice) = split_protected(dir, shown, changed);
    format!("{}{notice}", capped(kept))
}

/// [`run_git_names`] for `git <git_args> --stat` (`diff`, `diff --cached`, `show <commit>`). The
/// names of the changed files are asked for too, so that a name git still shortens is judged by the
/// path it ends.
pub fn run_git_stat(dir: &str, git_args: &[&str]) -> String {
    let with = |extra: &[&'static str]| -> Vec<&str> { git_args.iter().copied().chain(extra.iter().copied()).collect() };
    let shown = run_raw(dir, &with(&[STAT]));
    let listing: &[&str] = if git_args.first() == Some(&"show") { &["--format=", "--name-only", "-z", "--no-renames"] } else { &["--name-only", "-z", "--no-renames"] };
    let names: Vec<String> = run_raw(dir, &with(listing)).split('\0').filter(|n| !n.is_empty()).map(str::to_string).collect();
    names_hidden(dir, &shown, &names)
}
