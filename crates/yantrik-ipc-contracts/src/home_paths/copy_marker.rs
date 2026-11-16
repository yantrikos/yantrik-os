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
    ".trashinfo", ".yantrik-moved", "___jb_old___", "___jb_tmp___", ".part", ".partial", ".crdownload", ".filepart", ".download", ".sav", ".bk",
];

/// The name an editor's swap or undo file is the swap of: `NAME.swp` or `.NAME.swp` (the dot of a
/// dotfile kept: `.env.swp` is `.env`). Any name may end `.swp`, `.swo` or `.swn`. Vim runs on
/// through `.swa` … `.swm` and `.svz` … `.saa` as swap files collide, but only for its own
/// `.NAME.s??` or a `%`/`!` spelling of a path, which is `wide`: `credentials.svg` and `cookies.sql`
/// are no one's swap (review M1). Vim keeps undo text in `.NAME.un~`, and Kate its journal in
/// `.NAME.kate-swp`.
fn swap_original(name: &str, wide: bool) -> Option<&str> {
    let (bare, end) = name.rsplit_once('.')?;
    let narrow = matches!(end, "swp" | "swo" | "swn");
    let range = matches!(end.as_bytes(), [b's', b'w', b'a'..=b'p'] | [b's', b'a'..=b'v', b'a'..=b'z']);
    let swap = end == "un~" || end == "kate-swp" || narrow || (wide && range);
    swap.then_some(bare)
}

/// The paths an editor's copy spells into its name, when `name` (a last part) is one: Neovim's swap
/// and undo files (`%home%ann%.kube%config.swp`), Emacs's backups (`!home!ann!.kube!config~`) and
/// auto-saves (`#!home!ann!.kube!config#`), with the marker removed and each `%` or `!` read as a `/` (`%%` and `!!` are the character
/// itself). The caller judges the path, so a copy of a file in a protected place is protected
/// (review R5-H1).
pub(super) fn encoded_path(name: &str) -> Option<std::path::PathBuf> {
    // Emacs's auto-save file wraps the same spelling in `#…#` (review R6-H1). Judged before the name
    // is lowered: a title filter asks about every run of a long title, and almost none start so.
    let name = name.strip_prefix('#').and_then(|n| n.strip_suffix('#')).unwrap_or(name);
    let sep = name.chars().next().filter(|c| matches!(c, '%' | '!'))?;
    let name = name.to_lowercase();
    let body = originals(&name).into_iter().find(|o| o.starts_with(sep)).unwrap_or(name);
    let mut path = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c == sep && chars.peek() == Some(&sep) => {
                chars.next();
                path.push(sep);
            }
            c if c == sep => path.push('/'),
            c => path.push(c),
        }
    }
    Some(path.into())
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
    // Vim's whole range is for a dotfile's own swap and for a path spelled into a name.
    if let Some(bare) = swap_original(name, name.starts_with(['.', '%', '!'])) {
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
    // Neovim's swap and undo files with `%`, Emacs's backups with `!` (review R3-1). The file is
    // the last part, so it is kept with its swap ending removed; a URL-encoded name
    // (`id_rsa%20old`) starts with its file, so the first part is kept when it looks like a file's
    // name (starts with `.`, or has a `_` or `.`), and `cookies%20recipe.pdf` is not a cookie jar
    // (review R5-L1). The caller judges the whole name as well (review H1).
    if let Some((first, last)) = name.split_once('%').zip(name.rsplit_once('%')).map(|((first, _), (_, last))| (first, last)) {
        if first.starts_with('.') || first.contains(['_', '.']) {
            add(first);
        }
        add(last);
    }
    if let Some(rest) = name.strip_prefix('!') {
        add(rest.rsplit('!').next().unwrap_or(rest));
    }
    // Windows leaves `NAME:Zone.Identifier` beside a download; LibreOffice `.~lock.NAME#` (review
    // R3-5). A stream is the text after the last colon, with no space in it: `notes: a title.md`
    // has none (review L1), and `deploy-10:30:00.pem` keeps its own ending through the caller.
    if let Some((bare, stream)) = name.rsplit_once(':') {
        if !stream.contains(char::is_whitespace) {
            add(bare);
            add(stream);
        }
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

/// What a person or a backup script adds to a folder's name by hand: a separator and a backup word with
/// optional digits (`-old`, `_bak2`, `.backup`) or a date or number (`.2026-11-15`, `-2`). The name
/// before the first separator that is followed by one is the folder it copies; `.kube-tools` and
/// `.kubectl` are copies of nothing. Used for a folder part only, and only for a hidden one (`.kube`, not `Screenshots-old`) (review N-M3).
pub(super) fn backup_original(name: &str) -> Option<&str> {
    if !name.starts_with('.') {
        return None;
    }
    name.char_indices().filter(|&(at, c)| at > 0 && matches!(c, '-' | '_' | '.')).find_map(|(at, _)| is_backup_tail(&name[at + 1..]).then_some(&name[..at]))
}

/// Whether `tail`, what follows a separator in a name, is a backup word with optional digits, a date or a number.
pub(super) fn is_backup_tail(tail: &str) -> bool {
    const WORDS: &[&str] = &["old", "bak", "backup", "orig", "prev", "save", "copy", "yantrik-moved"];
    WORDS.contains(&tail.trim_end_matches(|c: char| c.is_ascii_digit()))
        || (tail.bytes().any(|b| b.is_ascii_digit()) && tail.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_')))
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

/// `copy`, `copy 1`, `another copy`, `3rd copy` or a conflict label (whose it is before it, and a
/// date and time after): what GNOME Files, Thunar, Finder, Dropbox and Nextcloud put in the
/// brackets. A conflict label is taken as it is: the name is judged by the original it leaves,
/// so a decoy such as `notes (conflicted copy 1).md` is still no one's secret (review M1).
fn is_a_copy_label(inner: &str) -> bool {
    if inner.contains("conflicted copy") || inner.contains("case clash") {
        return true;
    }
    if inner.strip_prefix("copy ").is_some_and(is_number) {
        return true;
    }
    let Some(ordinal) = inner.strip_suffix(" copy") else { return inner == "copy" };
    let digits = ordinal.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    ordinal == "another"
        || (!digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && matches!(&ordinal[digits.len()..], "st" | "nd" | "rd" | "th"))
}
