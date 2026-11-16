//! Places that no list of names can hold (#1247, gate 12).
//!
//! [`super::PROTECTED`] names a place once. Two kinds of place are known by their shape instead:
//!
//! - **A harness's home under any suffix.** An update that moves a harness to the Mind's account keeps
//!   the person's old copy, renamed `<home>.yantrik-moved`, whole, with its API keys in it
//!   (`deploy/yantrik-os/harness-isolation.sh`); a person keeps `.hermes.bak` or `.pi-old` by hand. The
//!   copy is as secret as the original, whatever the suffix is.
//! - **A repository's object store.** `.git/objects` (and a bare repository's, and a submodule's under
//!   `.git/modules`) holds every committed file, zlib-packed: reading one is reading a blob the git tools
//!   refuse by name. The git tools stay the way in.

/// The harnesses' homes under the person's home (`harnesses/*` and the vendors' own), lower case.
const HARNESS_HOMES: &[&str] = &[".hermes", ".openclaw", ".pi", ".claude", ".claude.json"];

/// What a kept copy's name may add to a harness's home: a separator or a number, so `.hermes.bak`,
/// `.pi-old`, `.claude_2` and `.hermes~` are copies and `.pip` and `.pidgin` are not.
fn is_a_suffix(rest: &str) -> bool {
    rest.chars().next().is_some_and(|c| matches!(c, '.' | '-' | '_' | '~') || c.is_ascii_digit())
}

/// Whether `part` (lower case) is a harness's home, or one with any suffix: a folder `.pi-old` is
/// where its children are, and every one of them is refused.
fn is_a_home_by_any_suffix(part: &str) -> bool {
    HARNESS_HOMES.iter().any(|home| part.strip_prefix(home).is_some_and(|rest| rest.is_empty() || is_a_suffix(rest)))
}

/// Whether `part` (lower case) is a harness's home, or a copy of one by a copy marker (`.hermes.bak`,
/// `.hermes.yantrik-moved`). The last part of a path may be a file, and `.pi-day.png` is no copy of
/// anything, so it needs a marker; the parts before it are folders and take any suffix.
fn is_a_home_or_a_marked_copy(part: &str) -> bool {
    HARNESS_HOMES.contains(&part) || super::copy_marker::originals(part).iter().any(|original| HARNESS_HOMES.contains(&original.as_str()))
}

/// Whether `parts` (the path's parts, lower case) are inside a harness's home or a kept copy of one,
/// or are one (by a marker: see [`is_a_home_or_a_marked_copy`]).
pub(super) fn in_a_harness_home(parts: &[String]) -> bool {
    let Some((last, folders)) = parts.split_last() else { return false };
    folders.iter().any(|part| is_a_home_by_any_suffix(part)) || is_a_home_or_a_marked_copy(last)
}

/// Whether `parts` are inside a repository's object store: a part that names a git directory
/// (`.git`, or a bare repository's `name.git`) with a part `objects` after it, or, for a git
/// directory kept apart from its work tree (`--separate-git-dir`), an `objects` folder that holds a
/// loose object (`ab/<38 or more hex>`) or a pack (`pack/pack-<id>.pack|idx|rev|bitmap`).
pub(super) fn in_an_object_store(parts: &[String]) -> bool {
    let hex = |s: &str, least: usize| s.len() >= least && s.bytes().all(|b| b.is_ascii_hexdigit());
    if parts.iter().position(|part| part == ".git" || part.ends_with(".git")).is_some_and(|git| parts[git + 1..].iter().any(|part| part == "objects")) {
        return true;
    }
    parts.iter().enumerate().filter(|(_, part)| *part == "objects").any(|(at, _)| match (parts.get(at + 1).map(String::as_str), parts.get(at + 2).map(String::as_str)) {
        (Some(dir), Some(id)) => (hex(dir, 2) && dir.len() == 2 && hex(id, 38)) || (dir == "pack" && id.starts_with("pack-") && [".pack", ".idx", ".rev", ".bitmap"].iter().any(|end| id.ends_with(end))),
        _ => false,
    })
}

/// What the installers and the updater add to a name when they keep a copy of a file they are about to
/// change (`config.yaml.before-yantrik`, `.model-before-yantrik.json`, `.hermes.yantrik-moved`): the
/// copy holds what the original did, wherever it is kept.
pub(super) fn is_a_kept_copy(part: &str) -> bool {
    ["before-yantrik", "yantrik-moved"].iter().any(|mark| part.contains(mark))
}
