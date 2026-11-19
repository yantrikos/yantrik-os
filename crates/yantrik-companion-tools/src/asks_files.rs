//! The card for a file the companion is about to overwrite or edit (#1311): `write_file` onto a
//! path that exists, and `edit_file`. Both ask `delete_for_good` ("cannot be undone": what was
//! there is gone) immediately before the write, after the path is validated and the
//! replacement computed, so no way into either tool reaches the write around the question.
//!
//! The card shows the path (and where it really goes, when that is somewhere else), how much
//! changes, and the first and last lines of what replaces it, each line cut to [`LINE_CHARS`]
//! with the cut marked. It says so: the middle of the text is not on the card, and the card
//! does not claim it is. A path the card cannot show whole is not asked about, and the file is
//! left alone ([`crate::asks::Question::fits`]).
//!
//! # The write is of the file that was shown (#1313, M3)
//!
//! The card waits up to two minutes. What is written is what was read when the card went up:
//! at the moment of the write the path is validated again and must resolve to the same place,
//! the opened file must be the same device and inode, and its content must still be what the card
//! was made from. Anything that changed in the wait: nothing is written, and the sentence says so.
//! A new file is created with `create_new`, so a file that appeared since is never truncated
//! unasked: it falls back to the asked overwrite.

use std::io::{Read, Seek, Write};
use std::path::PathBuf;

use crate::asks::{act, Card, Question};

/// The longest first or last line the card shows before it marks a cut. Four of these, the path
/// and the sentence about the change fit the 640 characters a question's card carries
/// ([`crate::asks::WHOLE_TOTAL_CHARS`]).
const LINE_CHARS: usize = 40;

/// Which file this is: its device and inode. Where the system has neither, nothing tells two
/// files apart this way and the path and the content are what is compared.
#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn identity(_: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

/// How a path as written is checked again at the moment of the write.
pub type Validate<'a> = &'a dyn Fn(&str) -> Result<String, String>;

/// One line of text for the card, cut with the cut marked: what was cut is said, not hidden.
fn line(text: &str) -> String {
    let count = text.chars().count();
    if count <= LINE_CHARS {
        return text.to_string();
    }
    format!("{}… ({count} characters in all)", text.chars().take(LINE_CHARS).collect::<String>())
}

fn size(text: &str) -> String {
    let lines = text.lines().count();
    format!("{lines} line{}, {} characters", if lines == 1 { "" } else { "s" }, text.chars().count())
}

/// The first and last line of `text` as the card shows them.
fn ends(card: Card, key_first: &str, key_last: &str, text: &str) -> Card {
    let first = text.lines().next().unwrap_or("");
    let last = text.lines().last().unwrap_or("");
    card.text(key_first, &line(first)).text(key_last, &line(last))
}

/// What the file was when the card was made: where it really is, which file it is, what it held.
struct Snapshot {
    resolved: PathBuf,
    dev: u64,
    ino: u64,
    bytes: Vec<u8>,
}

fn snapshot(expanded: &str) -> Result<Snapshot, String> {
    let nothing = |why: String| format!("Could not read {expanded} to show what changes, so nothing was written: {why}");
    let resolved = std::fs::canonicalize(expanded).map_err(|e| nothing(e.to_string()))?;
    let meta = std::fs::metadata(&resolved).map_err(|e| nothing(e.to_string()))?;
    if !meta.is_file() {
        return Err(nothing("it is not a regular file".into()));
    }
    let bytes = std::fs::read(&resolved).map_err(|e| nothing(e.to_string()))?;
    let (dev, ino) = identity(&meta);
    Ok(Snapshot { resolved, dev, ino, bytes })
}

/// The card's path rows: the path as written, and where it really goes when that is elsewhere.
fn path_rows(card: Card, path: &str, expanded: &str, snap: &Snapshot) -> Card {
    let card = card.text("path", path);
    let resolved = snap.resolved.to_string_lossy();
    if resolved == expanded {
        card
    } else {
        card.text("resolves_to", &resolved)
    }
}

fn changed(why: &str) -> String {
    format!("The file changed while the question was open ({why}), so nothing was written. Nothing was changed.")
}

/// The write itself, of the file that was shown or of nothing.
fn rewrite(validate: Validate, path: &str, expanded: &str, snap: &Snapshot, new: &[u8]) -> Result<(), String> {
    match validate(path) {
        Ok(again) if again == expanded => {}
        Ok(_) => return Err(changed("the path now leads somewhere else")),
        Err(e) => return Err(changed(&format!("the path no longer passes the check: {e}"))),
    }
    if std::fs::canonicalize(expanded).ok().as_deref() != Some(snap.resolved.as_path()) {
        return Err(changed("the path now resolves to another place"));
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(expanded)
        .map_err(|e| changed(&format!("it could not be opened: {e}")))?;
    let meta = file.metadata().map_err(|e| changed(&e.to_string()))?;
    if !meta.is_file() || identity(&meta) != (snap.dev, snap.ino) {
        return Err(changed("it is not the same file"));
    }
    let mut now = Vec::new();
    file.read_to_end(&mut now).map_err(|e| changed(&e.to_string()))?;
    if now != snap.bytes {
        return Err(changed("its content is not what was shown"));
    }
    let write = |file: &mut std::fs::File| -> std::io::Result<()> {
        file.set_len(0)?;
        file.rewind()?;
        file.write_all(new)
    };
    write(&mut file).map_err(|e| format!("Error writing file: {e}"))
}

/// `write_file` onto a path that exists, without `append`: ask, then replace its content.
pub fn overwrite(path: &str, expanded: &str, content: &str) -> Result<(), String> {
    overwrite_with(&crate::validate_write_path, path, expanded, content)
}

/// [`overwrite`], with the check made at the moment of the write given.
pub fn overwrite_with(validate: Validate, path: &str, expanded: &str, content: &str) -> Result<(), String> {
    let snap = snapshot(expanded)?;
    let was = size(&String::from_utf8_lossy(&snap.bytes));
    let change = format!("replaces the whole file (was {was}; becomes {}); the first and last lines are shown", size(content));
    let card = path_rows(Card::new("write_file"), path, expanded, &snap).plain("change", change);
    let card = ends(card, "first_line", "last_line", content);
    act(Question::DeleteForGood, &card, || rewrite(validate, path, expanded, &snap, content.as_bytes()))
}

/// `write_file` without `append`: a new file is made without a question, and only if it is still
/// new when it is made; a file that is there is asked about ([`overwrite`]).
pub fn write_new_or_overwrite(path: &str, expanded: &str, content: &str) -> Result<(), String> {
    write_new_or_overwrite_with(&crate::validate_write_path, path, expanded, content)
}

/// [`write_new_or_overwrite`], with the check made at the moment of the write given.
pub fn write_new_or_overwrite_with(validate: Validate, path: &str, expanded: &str, content: &str) -> Result<(), String> {
    match std::fs::OpenOptions::new().write(true).create_new(true).open(expanded) {
        Ok(mut file) => file.write_all(content.as_bytes()).map_err(|e| format!("Error writing file: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => overwrite_with(validate, path, expanded, content),
        Err(e) => Err(format!("Error writing file: {e}")),
    }
}

/// `edit_file`: ask, then write the edited content back.
pub fn edit(path: &str, expanded: &str, replaced: usize, old: &str, new: &str, before: &str, after: &str) -> Result<(), String> {
    edit_with(&crate::validate_write_path, path, expanded, replaced, old, new, before, after)
}

/// [`edit`], with the check made at the moment of the write given.
#[allow(clippy::too_many_arguments)]
pub fn edit_with(validate: Validate, path: &str, expanded: &str, replaced: usize, old: &str, new: &str, before: &str, after: &str) -> Result<(), String> {
    let snap = snapshot(expanded)?;
    if snap.bytes != before.as_bytes() {
        return Err(changed("it was not what the edit was made from"));
    }
    let change = format!(
        "replaces {replaced} place(s) (the file was {}; becomes {}); only the first and last lines of what goes and what comes are shown",
        size(before),
        size(after)
    );
    let card = path_rows(Card::new("edit_file"), path, expanded, &snap).plain("change", change);
    let card = ends(ends(card, "removes_first", "removes_last", old), "first_line", "last_line", new);
    act(Question::DeleteForGood, &card, || rewrite(validate, path, expanded, &snap, after.as_bytes()))
}
