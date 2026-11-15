//! The names a copy of a file gets (#1247, reviews M4 and N1).
//!
//! A credential copied by an editor, a package manager or a file manager is the same text under
//! another name: `tokens.json.save`, `.tokens.json.swp`, `tokens (copy).json`, `Copy of tokens.json`.
//! [`originals`] gives the names a copy may have been made from, one marker removed. Every rule is
//! an exact ending, start or shape; a word somewhere inside a name is never a marker, so
//! `notes copy.md` and `chapter.2.md` are no one's copy.

/// Endings that add to a name: backups, editors' and package managers' leftovers.
const ENDINGS: &[&str] = &[
    ".bak", ".old", ".orig", ".backup", ".copy", "~", ".save", ".tmp", ".dpkg-old", ".dpkg-dist", ".rpmsave",
    ".rpmnew", ".prev",
    // Editors' safe-write copies, and a browser's or a copy tool's unfinished transfer (review R3-1, R3-2).
    "___jb_old___", "___jb_tmp___", ".part", ".partial", ".crdownload", ".filepart", ".download", ".sav", ".bk",
];

/// The name an editor's swap or undo file is the swap of: `NAME.swp` or `.NAME.swp` (the dot of a
/// dotfile kept: `.env.swp` is `.env`). Vim runs through `.swp`, `.swo` … `.swa` as swap files
/// collide; it keeps undo text in `.NAME.un~`, and Kate its journal in `.NAME.kate-swp`.
fn swap_original(name: &str) -> Option<&str> {
    let (bare, end) = name.rsplit_once('.')?;
    let swap = end == "un~" || end == "kate-swp" || end.strip_prefix("sw").is_some_and(|l| matches!(l.as_bytes(), [b'a'..=b'p']));
    swap.then_some(bare)
}

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
        }
    }
    if let Some(bare) = swap_original(name) {
        add(bare);
        bare.strip_prefix('.').map(&mut add);
    }
    if let Some((bare, tail)) = name.rsplit_once('.') {
        let counted = is_number(tail)
            || tail.strip_prefix("bak").is_some_and(is_number)
            || tail.strip_prefix("backup").is_some_and(is_number)
            || tail.strip_prefix('~').and_then(|t| t.strip_suffix('~')).is_some_and(is_number);
        if counted {
            add(bare);
        }
    }
    // `tokens.json~1`: an Emacs numbered backup.
    if let Some((bare, number)) = name.rsplit_once('~') {
        if is_number(number) {
            add(bare);
        }
    }
    // Where an editor keeps its copies away from the file, it spells the whole path into the name:
    // Neovim's swap and undo files with `%`, Emacs's backups with `!` (review R3-1).
    if let Some((_, last)) = name.rsplit_once('%') {
        add(last);
    }
    if let Some(rest) = name.strip_prefix('!') {
        add(rest.rsplit('!').next().unwrap_or(rest));
    }
    // Windows leaves `NAME:Zone.Identifier` beside a download; LibreOffice `.~lock.NAME#` (review R3-5).
    if let Some((bare, _stream)) = name.split_once(':') {
        add(bare);
    }
    if let Some(inner) = name.strip_prefix(".~lock.").and_then(|n| n.strip_suffix('#')) {
        add(inner);
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
    if let Some(bare) = without_sync_conflict(name) {
        add(&bare);
    }
    found
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `name` without Syncthing's `.sync-conflict-<date>-<time>-<device>` (8 digits, 6 digits, 7 letters
/// or digits), which goes before the extension or at the end.
fn without_sync_conflict(name: &str) -> Option<String> {
    const MARK: &str = ".sync-conflict-";
    let at = name.find(MARK)?;
    let rest = &name[at + MARK.len()..];
    let mut parts = rest.splitn(3, '-');
    let (date, time, device_and_extension) = (parts.next()?, parts.next()?, parts.next()?);
    let device_len = device_and_extension.find('.').unwrap_or(device_and_extension.len());
    let device = &device_and_extension[..device_len];
    let shaped = date.len() == 8 && is_number(date) && time.len() == 6 && is_number(time) && device.len() == 7 && device.bytes().all(|b| b.is_ascii_alphanumeric());
    shaped.then(|| format!("{}{}", &name[..at], &device_and_extension[device_len..]))
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
        if is_number(number) {
            found.extend(rest.strip_suffix(" copy"));
            // Finder's "Keep Both": `tokens 2.json` (review R3-2). It is judged by what it
            // is a copy of, so `chapter 2.md` stays readable.
            found.push(rest);
        }
    }
    found
}

/// `copy`, `another copy`, `3rd copy` or `conflicted copy` (with whose it is before, and a date
/// after): what GNOME Files, Finder, Dropbox and Nextcloud put in the brackets.
fn is_a_copy_label(inner: &str) -> bool {
    const CONFLICT: &str = "conflicted copy";
    if let Some(at) = inner.find(CONFLICT) {
        let after = &inner[at + CONFLICT.len()..];
        return after.is_empty() || after.strip_prefix(' ').is_some_and(|date| !date.is_empty() && date.bytes().all(|b| b.is_ascii_digit() || b == b'-'));
    }
    let Some(ordinal) = inner.strip_suffix(" copy") else { return inner == "copy" };
    let digits = ordinal.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    ordinal == "another"
        || (!digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && matches!(&ordinal[digits.len()..], "st" | "nd" | "rd" | "th"))
}
