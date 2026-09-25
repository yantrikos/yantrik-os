//! The document yDoc edits: bounded UTF-8 Markdown, conflict-aware atomic saves, private
//! recovery, and the pure string work behind the outline, find/replace and the format buttons.
//!
//! Nothing here knows about Slint. Every function is callable from a test with no desktop, which
//! is the whole reason the module exists: the fault it was written to close is that Save was
//! unreachable, and an unreachable path is exactly what an in-memory assertion cannot see.
//!
//! **The format is Markdown, and that is the native format, not an export.** The screen already
//! says so — its empty-document hint reads "Markdown works here — # for a heading, - for a
//! list", the outline panel is derived from `#` lines, and the formatting buttons are the ATX
//! and emphasis marks. So the body of this document is Markdown text and the file on disk is a
//! `.md` file with exactly those bytes in it. Nothing is wrapped, nothing is serialised around
//! it: a mind can read, diff and write these documents with `cat`, `grep` and a text editor,
//! which is worth more here than any container we could have invented.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

/// One document is read and written whole, so the bound is on the whole of it.
///
/// The same 1 MiB the text editor uses. A megabyte of Markdown is about 170,000 words; past
/// that this app would be lying about being able to edit it in a single `TextInput`.
pub const MAX_BYTES: usize = 1024 * 1024;

/// The extension a document with no extension is given, so "notes" becomes "notes.md".
pub const EXTENSION: &str = "md";

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// What the file looked like the last time this document and the disk agreed.
///
/// Kept so a save can ask "has anyone else touched this since I read it" without reading the
/// file back on every keystroke. The stamp is the cheap question; the bytes are the answer, and
/// [`Document::save`] only pays for the bytes when the stamp says something moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub bytes: u64,
    pub modified_unix: i64,
    pub modified_nanos: u32,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Self {
        let (modified_unix, modified_nanos) = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| (d.as_secs() as i64, d.subsec_nanos()))
            .unwrap_or((0, 0));
        Self { bytes: meta.len(), modified_unix, modified_nanos }
    }
}

/// The stamp of whatever is at `path` now, or `None` if nothing is.
pub fn stamp_of(path: &Path) -> Option<Stamp> {
    fs::symlink_metadata(path).ok().map(|m| Stamp::of(&m))
}

/// A document: the text being edited, where it lives, and what it looked like on disk.
///
/// `baseline` is the text as the file last held it. `text == baseline` is what "saved" means, so
/// there is no separate dirty flag to fall out of step with the buffer — the same shape the text
/// editor settled on, and for the same reason.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Document {
    pub path: Option<PathBuf>,
    pub text: String,
    pub baseline: String,
    pub stamp: Option<Stamp>,
    /// Set on a document read back out of the recovery file, so the window can say so and so the
    /// draft stays dirty until a person has looked at it.
    pub recovered: bool,
}

/// What a save actually did, read back off the file after writing it.
///
/// The action that reports a save reports THIS, not the request it was given. Contract point 3:
/// the thing that changed, observed. A `save` that answers with the text it was handed has not
/// looked at anything.
#[derive(Clone, Debug)]
pub struct Saved {
    pub document: Document,
    pub stamp: Stamp,
}

impl Document {
    pub fn blank() -> Self {
        Self::default()
    }

    /// True while the buffer differs from the file. A recovered draft is dirty by definition:
    /// nobody has agreed to it yet.
    pub fn dirty(&self) -> bool {
        self.recovered || self.text != self.baseline
    }

    /// What is in this document and in no file, as a clause a refusal can carry, or `None` when
    /// nothing is at stake.
    ///
    /// A refusal that says only "there are unsaved changes" leaves a person working out for
    /// themselves how much they are about to lose and where the rest of it is. This says both,
    /// which is the difference between a warning and a sentence somebody can act on.
    pub fn unsaved(&self) -> Option<String> {
        if !self.dirty() {
            return None;
        }
        // A recovered draft that has not been typed into since has no "edit" to measure — the
        // whole of it is the thing nobody has agreed to.
        let untouched_draft = self.recovered && self.text == self.baseline;
        let count = if untouched_draft {
            char_count(&self.text)
        } else {
            unsaved_chars(&self.text, &self.baseline)
        };
        let home = match &self.path {
            Some(p) => format!("are not in {}", p.display()),
            None => "are in no file at all".to_string(),
        };
        Some(if untouched_draft {
            format!(
                "a recovered draft of \u{201c}{}\u{201d} \u{2014} {count} characters that {home} \
                 and that nobody has agreed to yet",
                self.title()
            )
        } else {
            format!(
                "{count} characters of \u{201c}{}\u{201d} that {home}",
                self.title()
            )
        })
    }

    /// What to call this document on screen.
    ///
    /// The first `# ` heading, because in a Markdown document that IS the title and it is inside
    /// the file rather than beside it. Failing that the file name, failing that "Untitled". The
    /// old screen had a separate title field that was typed into and saved nowhere; a title read
    /// out of the text cannot drift from it.
    pub fn title(&self) -> String {
        if let Some(h) = outline(&self.text).into_iter().find(|h| h.level == 1) {
            if !h.title.is_empty() {
                return h.title;
            }
        }
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let path = fs::canonicalize(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
        let text = read(&path)?;
        Ok(Self {
            stamp: stamp_of(&path),
            path: Some(path),
            baseline: text.clone(),
            text,
            recovered: false,
        })
    }

    /// Write the document to `path`, and report what landed there.
    ///
    /// Refuses rather than guesses, and every refusal names what is in the way:
    ///
    /// * a document with no path at all is the caller's problem to solve — see [`Document::home`],
    ///   which is what `save` with no path must go through;
    /// * a file that changed on disk since it was read is not overwritten, because the draft in
    ///   the window is not a merge of the two;
    /// * a Save As onto a path that already exists is refused, so "save a copy" can never be the
    ///   thing that destroys the original of something else — [`Document::save_over`] is the
    ///   same write with that last refusal answered.
    pub fn save(&self, path: &Path) -> Result<Saved, String> {
        self.write(path, false)
    }

    /// The same write, told to replace whatever is already at `path`.
    ///
    /// This exists because of the one situation where every refusal was correct and there was
    /// still no way to keep the work. Files moves a document with `rename`, so the file under an
    /// open document can be carried off: `save` then refuses because the original is gone, and
    /// `save_as` to where it went refuses because something is already there. Both sentences are
    /// true; between them a person is stuck. `overwrite` is how the second one is answered, and
    /// the refusal it answers names it.
    pub fn save_over(&self, path: &Path) -> Result<Saved, String> {
        self.write(path, true)
    }

    fn write(&self, path: &Path, overwrite: bool) -> Result<Saved, String> {
        validate(&self.text)?;
        if !path.is_absolute() {
            return Err(format!(
                "Use an absolute file path; `{}` is relative to a directory this app does not \
                 have.",
                path.display()
            ));
        }
        let name = path.file_name().ok_or("Choose a file name.")?;
        let parent = fs::canonicalize(path.parent().ok_or("Choose a file name.")?)
            .map_err(|e| format!("Cannot write into that folder: {e}"))?;
        let path = parent.join(name);

        let own = self.path.as_ref() == Some(&path);
        let mut replace = false;
        let mut permissions = None;
        match fs::symlink_metadata(&path) {
            // The document's own file is not where it was opened from any more. That is almost
            // never a deletion — Files moves with `rename` — so the refusal goes looking, and
            // `overwrite` is the caller having read it and said write it here anyway.
            Err(_) if own && !overwrite => return Err(self.stranded(&path)),
            Err(_) => {}
            Ok(_) if !own && !overwrite && !self.is_its_own_moved_file(&path) => {
                return Err(format!(
                    "{} already exists. Choose another name; nothing was overwritten. Save As \
                     with overwrite=true replaces it.",
                    path.display()
                ))
            }
            Ok(meta) => {
                if !meta.is_file() || meta.nlink() > 1 {
                    return Err(format!(
                        "{} is a link or is not a regular file, so nothing was written. Save As \
                         to another path.",
                        path.display()
                    ));
                }
                if meta.permissions().readonly() {
                    return Err(format!(
                        "{} is read-only, so nothing was written. Save As to another path.",
                        path.display()
                    ));
                }
                // The stamp is the cheap check and the bytes are the real one. A touch that
                // changed nothing must not block a save, and a rewrite that happens to land on
                // the same size and second must not slip through — so a moved stamp sends us to
                // the bytes, and the bytes decide. Only for our own file: a caller that said
                // "overwrite" has been told what is there and has answered.
                if own && stamp_of(&path) != self.stamp && read(&path)? != self.baseline {
                    return Err(format!(
                        "{} changed on disk since it was opened. Nothing was written; your draft \
                         is intact. Use Save As to keep both versions.",
                        path.display()
                    ));
                }
                replace = true;
                permissions = Some(meta.permissions());
            }
        }

        atomic_write(&path, self.text.as_bytes(), replace, permissions)?;

        // Read the file back for its size and mtime. This is what the save reports, and it is
        // the only number in the answer that was not supplied by the caller.
        let stamp = stamp_of(&path).ok_or("The file vanished immediately after it was written.")?;
        Ok(Saved {
            document: Self {
                path: Some(path),
                text: self.text.clone(),
                baseline: self.text.clone(),
                stamp: Some(stamp),
                recovered: false,
            },
            stamp,
        })
    }

    /// Why a save cannot write to the file this document came from, when that file is not there.
    ///
    /// Seen on 22 Sep 2026: Files moved an open document into a new folder and yDoc was left with
    /// no way to save it at all. `save` refused because the original was gone; `save_as` to the
    /// new path refused because something was already there; `open` on the new path dropped the
    /// unsaved edit without a word. Every one of those was true and together they lost the work.
    /// A refusal with no next step in it is the same as losing the work, so this one goes and
    /// looks for where the file went and names it.
    fn stranded(&self, path: &Path) -> String {
        match moved_to(path, &self.baseline) {
            Some(now) => format!(
                "{} is not there any more \u{2014} the same file looks to be at {} now. Nothing \
                 was written and your draft is intact: Save As to {} to write your changes into \
                 it.",
                path.display(),
                now.display(),
                now.display()
            ),
            None => format!(
                "{} was moved or deleted, so there is nothing there to write into. Nothing was \
                 written and your draft is intact: Save As with overwrite=true to write it here, \
                 or Save As to wherever it went.",
                path.display()
            ),
        }
    }

    /// Is the file at `path` this document's own, carried there by a move?
    ///
    /// True only when the file this document was opened from is no longer where it was, AND what
    /// is at `path` has the same name and exactly the bytes this document last agreed with. That
    /// is not a coincidence anyone should have to argue with: it is this document, moved. Writing
    /// into it is what a plain Save would have done, so a Save As onto it is not a clobber and is
    /// not refused as one.
    fn is_its_own_moved_file(&self, path: &Path) -> bool {
        let Some(origin) = &self.path else { return false };
        origin.file_name() == path.file_name()
            && fs::symlink_metadata(origin).is_err()
            && read(path).ok().as_deref() == Some(self.baseline.as_str())
    }

    /// Where a Save should write, or why it cannot write anywhere.
    ///
    /// This is the bug, in one function. Save used to read an `in` property nothing ever set,
    /// find it empty, log "No file path set" and return — so the guard could never be false, the
    /// `fs::write` under it never ran once, and a person pressing Save saw and heard nothing at
    /// all. A document with no file is a real situation and the answer to it is a sentence, not
    /// silence; the window turns this sentence into the Save As prompt and an action hands it
    /// back to the caller as a refusal.
    pub fn save_target(&self, given: Option<PathBuf>) -> Result<PathBuf, String> {
        given.or_else(|| self.path.clone()).ok_or_else(|| {
            format!(
                "This document has no file yet, so there is nowhere to save it. Use Save As, or \
                 `save_as` with a path — {} would do.",
                self.home().display()
            )
        })
    }

    /// Where a Save with no path should be offered, so the prompt opens on something sensible
    /// rather than on an empty box.
    pub fn home(&self) -> PathBuf {
        if let Some(p) = &self.path {
            return p.clone();
        }
        let documents = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Documents");
        documents.join(format!("{}.{EXTENSION}", slug(&self.title())))
    }
}

/// A file name out of a document title. ASCII, lower case, no separators of its own.
fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.extend(c.to_lowercase());
        } else {
            gap = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

// ── Following the file ──────────────────────────────────────────────────────
//
// A document open in yDoc is a file somebody else can move while it is open — Files does
// exactly that, with `rename`, which keeps the bytes and the inode and changes only the name.
// The decisions a move needs live in `yantrik-file-follow`, shared with the Text Editor so that
// #86 is fixed with one search and one rename rule rather than two to keep in step. What stays
// here is the one part that is this app's: what counts as reading the same bytes back.

/// Where the file that used to be at `original` probably is now, or `None`.
///
/// The same name and exactly the bytes the document last agreed with, somewhere under the folder
/// it used to live in — bounded, out of hidden folders, and honest (`None`) rather than guessing.
/// The search itself is the shared one; `read` is this app's, so a file this editor would refuse
/// to open is never reported as where the document went.
pub fn moved_to(original: &Path, baseline: &str) -> Option<PathBuf> {
    yantrik_file_follow::moved_to(original, baseline, read)
}

/// How many characters of `text` the file does not have.
///
/// The characters between the longest common prefix and the longest common suffix of the two,
/// which for an ordinary edit is the edit itself rather than the whole document. Characters and
/// not bytes, for the same reason the status bar counts characters.
pub fn unsaved_chars(text: &str, baseline: &str) -> usize {
    let a: Vec<char> = text.chars().collect();
    let b: Vec<char> = baseline.chars().collect();
    let mut head = 0;
    while head < a.len() && head < b.len() && a[head] == b[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < a.len() - head
        && tail < b.len() - head
        && a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
    {
        tail += 1;
    }
    a.len() - head - tail
}

/// What this app will hold, said before anything is written or replaced.
pub fn validate(text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES {
        return Err(format!(
            "A document is limited to 1 MiB; this one is {} bytes.",
            text.len()
        ));
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err("That is not a text document: it contains control bytes. yDoc edits UTF-8 Markdown.".into());
    }
    Ok(())
}

/// Read a file as the text of a document, or say exactly why it is not one.
pub fn read(path: &Path) -> Result<String, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Only a regular file can be opened as a document.".into());
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err(format!(
            "{} is {} bytes, past the 1 MiB editing limit. It was not loaded.",
            path.display(),
            meta.len()
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    // Strict, not lossy. Replacing a bad byte with U+FFFD and saving would write the damage back
    // over the original, which is worse than refusing to open it.
    let text = String::from_utf8(bytes)
        .map_err(|_| "That file is not valid UTF-8, and no lossy conversion was made.".to_string())?;
    validate(&text)?;
    Ok(text)
}

/// Write, then publish. Never truncate the real file and write into it.
///
/// A crash halfway through a plain write leaves a half-document where the whole one was. This
/// writes a private temporary beside the target, syncs it, renames it over (or links it into
/// place when nothing may be clobbered), and syncs the directory. The temporary is removed on
/// every failure path, so a failed save leaves nothing behind for the next person to wonder at.
fn atomic_write(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    permissions: Option<fs::Permissions>,
) -> Result<(), String> {
    let parent = path.parent().ok_or("No folder to write into")?;
    let temp = parent.join(format!(
        ".yantrik-ydoc-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        if let Some(p) = permissions {
            file.set_permissions(p)?;
        }
        file.sync_all()?;
        if replace {
            fs::rename(&temp, path)?;
        } else {
            // Publication that cannot clobber, including against another process creating the
            // same name in the same instant.
            fs::hard_link(&temp, path)?;
            fs::remove_file(&temp)?;
        }
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("Save failed: {e}. Your draft is still open and unchanged."))
}

// ── Recovery ────────────────────────────────────────────────────────────────

/// Where the unsaved draft is kept between sessions. Private to the user, 0700.
pub fn recovery_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("yantrik/document-editor/draft.json")
}

/// The draft the last session left behind, if there was one.
///
/// A document closed with unsaved changes must not disappear in silence. The window asks before
/// it closes; this is what covers the case where nobody got to ask — a kill, a crash, a machine
/// that went away. The recovered draft comes back dirty, so it is reviewed and not assumed.
pub fn recover(path: &Path) -> Result<Option<Document>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("Cannot read the recovery file: {e}"))?
        .take((MAX_BYTES * 4 + 65536) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut doc: Document = serde_json::from_slice(&bytes)
        .map_err(|e| format!("The recovery file could not be read: {e}. It has been left alone."))?;
    validate(&doc.text)?;
    validate(&doc.baseline)?;
    doc.recovered = true;
    Ok(Some(doc))
}

/// Keep the draft, or clear it when there is nothing to keep.
///
/// Written through the same temp-and-rename as a document, because a recovery file half-written
/// by a crash is exactly the file nobody can use.
pub fn checkpoint(path: &Path, doc: &Document) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid recovery path")?;
    if !doc.dirty() {
        // A saved document needs no draft, and leaving the old one would offer a stale draft on
        // the next start as though work had been lost.
        if path.exists() {
            fs::remove_file(path).map_err(|e| format!("Cannot clear the recovery file: {e}"))?;
        }
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(doc).map_err(|e| e.to_string())?;
    atomic_write(path, &bytes, true, None)
}

// ── The outline ─────────────────────────────────────────────────────────────

/// One ATX heading, and where it starts in the text.
///
/// `offset` is a byte offset into the document, which is what the editing surface's
/// `set-selection-offsets` takes — so clicking a heading in the outline moves the real caret
/// rather than scrolling to an approximation of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub title: String,
    pub level: i32,
    pub offset: usize,
    pub line: usize,
}

/// The headings of a Markdown document, in document order.
///
/// Fenced code blocks are skipped: a `# comment` inside ``` is a comment in some language, not a
/// section of the document, and an outline that lists it is describing the wrong thing.
pub fn outline(text: &str) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut offset = 0usize;
    let mut fenced = false;
    for (line_number, line) in text.split('\n').enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            let rest = &trimmed[hashes..];
            if (1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with(' ')) {
                headings.push(Heading {
                    title: rest.trim().trim_end_matches('#').trim_end().to_string(),
                    level: hashes as i32,
                    offset,
                    line: line_number + 1,
                });
            }
        }
        offset += line.len() + 1;
    }
    headings
}

// ── Find and replace ────────────────────────────────────────────────────────

/// Every match of `query`, case-insensitively, as byte ranges into `text`.
///
/// Through `regex` rather than by lowercasing a copy: case folding changes byte lengths for some
/// characters, so offsets taken from a lowercased string do not point where they claim to in the
/// original, and a replace built on them cuts a document in the wrong place.
pub fn matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return vec![];
    }
    regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
        .map(|r| r.find_iter(text).map(|m| (m.start(), m.end())).collect())
        .unwrap_or_default()
}

/// Replace the given ranges, checking the resulting size before allocating for it.
pub fn replace(text: &str, ranges: &[(usize, usize)], replacement: &str) -> Result<String, String> {
    let removed: usize = ranges.iter().map(|(a, z)| z - a).sum();
    let added = replacement
        .len()
        .checked_mul(ranges.len())
        .ok_or("That replacement is too large.")?;
    let size = text
        .len()
        .checked_sub(removed)
        .and_then(|n| n.checked_add(added))
        .ok_or("That replacement is too large.")?;
    if size > MAX_BYTES {
        return Err("That replacement would push the document past the 1 MiB limit.".into());
    }
    let mut out = String::with_capacity(size);
    let mut end = 0;
    for &(a, z) in ranges {
        out.push_str(&text[end..a]);
        out.push_str(replacement);
        end = z;
    }
    out.push_str(&text[end..]);
    validate(&out)?;
    Ok(out)
}

/// Replace every match, and say how many there were. Zero is not a failure; it is a count.
pub fn replace_all(text: &str, query: &str, with: &str) -> Result<(String, usize), String> {
    if query.is_empty() {
        return Err("Say what to find.".into());
    }
    let ranges = matches(text, query);
    let count = ranges.len();
    Ok((replace(text, &ranges, with)?, count))
}

// ── The format buttons ──────────────────────────────────────────────────────

/// What a formatting button does to the text, once the document is Markdown.
///
/// Every one of these is an insertion of Markdown syntax around the selection or at the start of
/// the lines it touches, which is why they can be real: the format the buttons describe is the
/// format the file is in. Underline and highlight are missing on purpose — Markdown has neither,
/// so those two buttons have been taken off the screen rather than made to write a syntax that
/// no reader of a `.md` file would render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Heading(u8),
    Bullet,
    Checklist,
    Quote,
    CodeBlock,
    Divider,
}

/// The text after a formatting button, and where the selection should now be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Snap an offset onto a character boundary, inside the text.
///
/// The offsets come from the editing surface as byte positions, and a caret sitting in the
/// middle of a multi-byte character would panic a slice. Walking back to the boundary is the
/// only behaviour that is both safe and unsurprising.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn wrap_markers(what: Format) -> Option<&'static str> {
    match what {
        Format::Bold => Some("**"),
        Format::Italic => Some("*"),
        Format::Strikethrough => Some("~~"),
        Format::InlineCode => Some("`"),
        _ => None,
    }
}

fn line_prefix(what: Format) -> Option<String> {
    match what {
        Format::Heading(n) => Some(format!("{} ", "#".repeat(n.clamp(1, 6) as usize))),
        Format::Bullet => Some("- ".into()),
        Format::Checklist => Some("- [ ] ".into()),
        Format::Quote => Some("> ".into()),
        _ => None,
    }
}

/// Apply a formatting button to `text` over the selection `start..end`.
///
/// Each of these toggles: pressing Bold on text that is already bold takes the markers off, and
/// pressing H2 on a line that is already an H2 makes it a paragraph again. A button that can
/// only ever add is a button you cannot undo without hunting for the characters it inserted.
pub fn format(text: &str, start: usize, end: usize, what: Format) -> Result<Edit, String> {
    let (start, end) = {
        let (a, z) = if start <= end { (start, end) } else { (end, start) };
        (boundary(text, a), boundary(text, z))
    };

    if let Some(marker) = wrap_markers(what) {
        return Ok(wrap(text, start, end, marker));
    }

    if let Some(prefix) = line_prefix(what) {
        return prefixed(text, start, end, &prefix, what);
    }

    match what {
        Format::CodeBlock => {
            let (from, to) = line_span(text, start, end);
            let body = &text[from..to];
            let replacement = format!("```\n{body}\n```");
            let out = splice(text, from, to, &replacement)?;
            // Inside the fence, where the next keystroke belongs.
            Ok(Edit { start: from + 4, end: from + 4 + body.len(), text: out })
        }
        Format::Divider => {
            let (from, _) = line_span(text, start, end);
            let out = splice(text, from, from, "---\n")?;
            Ok(Edit { start: from + 4, end: from + 4, text: out })
        }
        _ => Err("That formatting is not one this document knows.".into()),
    }
}

/// A link, which is the one format that carries something of its own.
///
/// With a URL it writes `[selection](url)`. Without one it writes the placeholder and selects
/// it, so the next thing typed is the address — the ribbon button passes no URL, because there
/// is nowhere on this screen to have typed one first.
pub fn format_link(text: &str, start: usize, end: usize, url: &str) -> Result<Edit, String> {
    let (start, end) = {
        let (a, z) = if start <= end { (start, end) } else { (end, start) };
        (boundary(text, a), boundary(text, z))
    };
    let label = &text[start..end];
    let address = if url.trim().is_empty() { "url" } else { url.trim() };
    let replacement = format!("[{label}]({address})");
    let out = splice(text, start, end, &replacement)?;
    let address_at = start + 1 + label.len() + 2;
    Ok(if url.trim().is_empty() {
        Edit { start: address_at, end: address_at + address.len(), text: out }
    } else {
        let after = start + replacement.len();
        Edit { start: after, end: after, text: out }
    })
}

fn splice(text: &str, from: usize, to: usize, replacement: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..from]);
    out.push_str(replacement);
    out.push_str(&text[to..]);
    validate(&out)?;
    Ok(out)
}

/// The whole of every line the selection touches.
fn line_span(text: &str, start: usize, end: usize) -> (usize, usize) {
    let from = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let to = text[end..].find('\n').map(|i| end + i).unwrap_or(text.len());
    (from, to)
}

fn wrap(text: &str, start: usize, end: usize, marker: &str) -> Edit {
    let n = marker.len();
    // Already wrapped, with the markers outside the selection: take them off.
    if start >= n
        && end + n <= text.len()
        && &text[start - n..start] == marker
        && &text[end..end + n] == marker
    {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start - n]);
        out.push_str(&text[start..end]);
        out.push_str(&text[end + n..]);
        return Edit { start: start - n, end: end - n, text: out };
    }
    // Already wrapped, with the markers inside the selection: take them off too, so a
    // double-click that grabs `**bold**` behaves the same as a drag that grabs `bold`.
    if end - start >= 2 * n && text[start..end].starts_with(marker) && text[start..end].ends_with(marker) {
        let inner = &text[start + n..end - n];
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start]);
        out.push_str(inner);
        out.push_str(&text[end..]);
        return Edit { start, end: start + inner.len(), text: out };
    }
    let mut out = String::with_capacity(text.len() + 2 * n);
    out.push_str(&text[..start]);
    out.push_str(marker);
    out.push_str(&text[start..end]);
    out.push_str(marker);
    out.push_str(&text[end..]);
    Edit { start: start + n, end: end + n, text: out }
}

/// Put a line prefix on every line the selection touches, or take it off every one that has it.
fn prefixed(text: &str, start: usize, end: usize, prefix: &str, what: Format) -> Result<Edit, String> {
    let (from, to) = line_span(text, start, end);
    let lines: Vec<&str> = text[from..to].split('\n').collect();

    // Off, only when every line already carries exactly this. A mixed selection gets the prefix
    // applied throughout, which is what a person pressing "bullet list" over mixed lines means.
    let all_have = lines.iter().all(|l| line_has(l, prefix, what));
    let rewritten: Vec<String> = lines
        .iter()
        .map(|l| {
            let bare = strip(l, what);
            if all_have {
                bare.to_string()
            } else {
                format!("{prefix}{bare}")
            }
        })
        .collect();
    let replacement = rewritten.join("\n");
    let out = splice(text, from, to, &replacement)?;
    Ok(Edit { start: from, end: from + replacement.len(), text: out })
}

/// Does this line already carry this exact mark?
///
/// A checklist item starts `- ` too, so a plain bullet does not count itself as one and — more
/// usefully — pressing Bullet on a checklist item converts it rather than clearing it.
fn line_has(line: &str, prefix: &str, what: Format) -> bool {
    match what {
        Format::Checklist => line.starts_with("- [ ] ") || line.starts_with("- [x] "),
        Format::Bullet => line.starts_with("- ") && !line.starts_with("- ["),
        _ => line.starts_with(prefix),
    }
}

/// The line without whichever mark of this family it carries, so switching H2 to H3 does not
/// leave `## ### `, and a bullet does not become `- - `.
fn strip(line: &str, what: Format) -> &str {
    match what {
        Format::Heading(_) => {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) {
                line[hashes..].trim_start_matches(' ')
            } else {
                line
            }
        }
        Format::Bullet | Format::Checklist => line
            .strip_prefix("- [ ] ")
            .or_else(|| line.strip_prefix("- [x] "))
            .or_else(|| line.strip_prefix("- "))
            .unwrap_or(line),
        Format::Quote => line.strip_prefix("> ").unwrap_or(line),
        _ => line,
    }
}

// ── Counts and export ───────────────────────────────────────────────────────

pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Characters, not bytes. The old status bar reported `text.len()`, which is the byte length and
/// overstates every document with an accent in it.
pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// The document as a standalone HTML page.
///
/// The renderer is `pulldown-cmark`, which is already in this tree — the Notes app parses
/// Markdown with it — so this is a reuse and not a second Markdown implementation to keep in
/// step with the first. The page it emits is deliberately plain: a charset, a title and the
/// rendered body, because the point of the export is the content, and a stylesheet invented here
/// would be this app's opinion travelling with someone else's document.
pub fn to_html(markdown: &str, title: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut body = String::new();
    html::push_html(&mut body, Parser::new_ext(markdown, options));
    let title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<!doctype html>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n{body}")
}
