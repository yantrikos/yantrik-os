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

/// Whether `parts` (the path's parts, lower case) are inside a harness's home or a kept copy of one,
/// or are one. A folder part takes any suffix (`.pi-old/x`); the last part, which may be a file, takes
/// a copy marker (`.hermes.bak`) or, with no extension, any suffix too (`.hermes-2026`), so a listing
/// and a window title do not name the folder, while `.pi.png` and `.pi-day.png` stay files (review
/// R11-L1).
pub(super) fn in_a_harness_home(parts: &[String]) -> bool {
    let Some((last, folders)) = parts.split_last() else { return false };
    let has_extension = last.rfind('.').is_some_and(|dot| dot > 0);
    folders.iter().any(|part| is_a_home_by_any_suffix(part))
        || (!has_extension && is_a_home_by_any_suffix(last))
        || HARNESS_HOMES.contains(&last.as_str())
}

/// Whether `parts` are inside a copy of a protected place that a person or a file manager made by
/// marking a folder's or a file's name: `.kube.bak`, `.kube (copy)`, `.hermes copy`, `.ssh~`,
/// `.config/labwc (copy)`, `.docker.bak/config.json`. One rule for every listed place: a run of parts
/// in which each is the place's part, or a copy of it (one marker removed), and one at least is a
/// copy (review R11-M2). `places` is [`super::PROTECTED`], split and in lower case. A part with a
/// `%`, `!`, `:` or `#` in it is an editor's encoded copy or a title's word, judged elsewhere.
pub(super) fn in_a_marked_copy(parts: &[String], places: &[Vec<String>]) -> bool {
    // A marker only removes text, so a copy holds the name it copies; and every marker has a dot, a
    // space, a bracket or another mark in it, so a part of letters and digits is no copy. Most parts
    // are neither, and a title is judged by thousands of them.
    let may_copy = |part: &str| {
        !part.bytes().all(|b| b.is_ascii_alphanumeric()) && !part.contains(['%', '!', ':', '#']) && (HARNESS_HOMES.iter().any(|home| part.contains(home)) || places.iter().flatten().any(|name| part.contains(name.as_str())))
    };
    let copies: Vec<Vec<String>> = parts.iter().map(|part| if may_copy(part) { super::copy_marker::originals(part) } else { Vec::new() }).collect();
    if copies.iter().all(Vec::is_empty) {
        return false;
    }
    copies.iter().any(|originals| originals.iter().any(|original| HARNESS_HOMES.contains(&original.as_str())))
        || places.iter().any(|want| {
            parts.windows(want.len()).zip(copies.windows(want.len())).any(|(window, marked)| {
                marked.iter().any(|originals| !originals.is_empty())
                    && window.iter().zip(marked).zip(want).all(|((part, originals), name)| part == name || originals.iter().any(|original| original == name))
            })
        })
}

/// Whether `parts` are inside a repository's object store, or are a file that lists what is in it: a
/// part that names a git directory (`.git`, or a bare repository's `name.git`) with `objects` right
/// after it (or after `modules/<name>/`, a submodule's), its `index` (which names every staged file:
/// `git mv notes.md tokens.json` puts `tokens.json` in it), or, for a git directory kept apart from
/// its work tree (`--separate-git-dir`), an `objects` folder that holds a loose object
/// (`ab/<38 or more hex>`), a pack (`pack/pack-<id>.pack|idx|rev|bitmap`, or a `tmp_pack_*` one) or an
/// `incoming-*` folder. A bundle is the whole pack in one file (review R11-L2).
pub(super) fn in_an_object_store(parts: &[String]) -> bool {
    let hex = |s: &str, least: usize| s.len() >= least && s.bytes().all(|b| b.is_ascii_hexdigit());
    let is_git = |part: &String| part == ".git" || part.ends_with(".git");
    let after = |git: usize, at: usize| parts.get(git + at).map(String::as_str);
    if parts.last().is_some_and(|last| last.ends_with(".bundle")) {
        return true;
    }
    if parts.iter().enumerate().filter(|(_, part)| is_git(part)).any(|(git, _)| {
        matches!(after(git, 1), Some("objects" | "index")) || (after(git, 1) == Some("modules") && parts[git + 2..].iter().any(|part| part == "objects"))
    }) {
        return true;
    }
    parts.iter().enumerate().filter(|(_, part)| *part == "objects").any(|(at, _)| match (parts.get(at + 1).map(String::as_str), parts.get(at + 2).map(String::as_str)) {
        (Some(dir), Some(id)) => {
            (hex(dir, 2) && dir.len() == 2 && hex(id, 38))
                || (dir == "pack" && (id.starts_with("tmp_pack_") || (id.starts_with("pack-") && [".pack", ".idx", ".rev", ".bitmap"].iter().any(|end| id.ends_with(end)))))
                || dir.starts_with("incoming-")
        }
        _ => false,
    })
}

/// What the installers and the updater add to a name when they keep a copy of a file they are about to
/// change (`config.yaml.before-yantrik`, `.model-before-yantrik.json`, `.hermes.yantrik-moved`): the
/// copy holds what the original did, wherever it is kept.
pub(super) fn is_a_kept_copy(part: &str) -> bool {
    // As an ending, or just before an extension: `before-yantrik-notes.md` and `why-yantrik-moved.txt`
    // are the person's own words, and are not copies (review R11-L5).
    [".yantrik-moved", "before-yantrik"].iter().any(|mark| part.ends_with(mark) || part.contains(&format!("{mark}.")))
}
