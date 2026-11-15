//! The names a copy of a file gets (#1247, reviews M4 and N1).
//!
//! A credential copied by an editor, a package manager or a file manager is the same text under
//! another name: `tokens.json.save`, `.tokens.json.swp`, `tokens (copy).json`, `Copy of tokens.json`.
//! [`originals`] gives the names a copy may have been made from, one marker removed. Every rule is
//! an exact ending, start or shape; a word somewhere inside a name is never a marker, so
//! `notes copy.md` and `chapter.2.md` are no one's copy.

/// Endings that add to a name: backups, editors' and package managers' leftovers.
const ENDINGS: &[&str] = &[
    ".bak", ".old", ".orig", ".backup", ".copy", "~", ".save", ".swp", ".swo", ".tmp", ".dpkg-old", ".dpkg-dist", ".rpmsave",
    ".rpmnew", ".prev",
];

/// Vim's swap files: `.NAME.swp`, with the dot of a dotfile kept (`.env.swp`).
const SWAP_ENDINGS: &[&str] = &[".swp", ".swo"];

/// The names `name` (lower case) may be a copy of, each with one marker removed. Empty when
/// `name` carries no marker. Callers peel again, so stacked markers (`a.json.bak (1)`) come off.
pub(super) fn originals(name: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut add = |bare: &str| {
        if !bare.is_empty() {
            found.push(bare.to_string());
        }
    };
    for end in ENDINGS {
        if let Some(bare) = name.strip_suffix(end) {
            add(bare);
            if SWAP_ENDINGS.contains(end) {
                bare.strip_prefix('.').map(&mut add);
            }
        }
    }
    if let Some((bare, tail)) = name.rsplit_once('.') {
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let counted = digits(tail)
            || tail.strip_prefix("bak").is_some_and(digits)
            || tail.strip_prefix('~').and_then(|t| t.strip_suffix('~')).is_some_and(digits);
        if counted {
            add(bare);
        }
    }
    if let Some(inner) = name.strip_prefix('#').and_then(|n| n.strip_suffix('#')) {
        add(inner);
    }
    if let Some(bare) = name.strip_prefix(".#").or_else(|| name.strip_prefix("copy of ")) {
        add(bare);
    }
    // A file manager's marker goes before the extension, or at the end when there is none.
    let stems = std::iter::once((name, "")).chain(name.rfind('.').filter(|&dot| dot > 0).map(|dot| name.split_at(dot)));
    for (stem, extension) in stems {
        for bare in without_name_marker(stem) {
            add(&format!("{bare}{extension}"));
        }
    }
    found
}

/// `stem` without a trailing ` (copy)`, ` (another copy)`, ` (3rd copy)`, ` (2)`, `(2)`, ` - copy`,
/// ` copy` or ` copy 2`.
fn without_name_marker(stem: &str) -> Vec<&str> {
    let mut found = Vec::new();
    if let Some(open) = stem.strip_suffix(')').and_then(|s| s.rfind('(').map(|at| (s, at))) {
        let (s, at) = open;
        let inner = &s[at + 1..];
        let before = &s[..at];
        let number = !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit());
        if number {
            found.push(before.strip_suffix(' ').unwrap_or(before));
        } else if is_a_copy_label(inner) {
            found.extend(before.strip_suffix(' '));
        }
    }
    found.extend(stem.strip_suffix(" - copy"));
    found.extend(stem.strip_suffix(" copy"));
    if let Some((rest, number)) = stem.rsplit_once(' ') {
        if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) {
            found.extend(rest.strip_suffix(" copy"));
        }
    }
    found
}

/// `copy`, `another copy` or `3rd copy`: what GNOME Files and Finder put in the brackets.
fn is_a_copy_label(inner: &str) -> bool {
    let Some(ordinal) = inner.strip_suffix(" copy") else { return inner == "copy" };
    let digits = ordinal.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    ordinal == "another"
        || (!digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && matches!(&ordinal[digits.len()..], "st" | "nd" | "rd" | "th"))
}
