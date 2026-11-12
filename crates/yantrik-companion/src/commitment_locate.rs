//! Which file on disk a commitment's words mean (#1184).
//!
//! A file is chosen only when it is the one: a file whose name carries every distinguishing word
//! of the label as a whole word ("Q4" and "budget" in "Q4 budget spreadsheet"), with an extension
//! of the label's kind, and, for an extension that fits many kinds of thing (pdf, txt, md), the
//! kind's noun too. A label with no distinguishing word ("the contract") names no file. With more
//! than one candidate none is chosen: the answer is how many there were. The newest file is never
//! taken for want of anything better.
//!
//! Only the person's own document folders are searched, not their whole home.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::commitment_files::{FileKind, NamedFile};
use crate::commitment_hands::{Failed, Hands};

/// Folders, by their XDG name and default, where a person keeps what they would send.
const FOLDERS: [(&str, &str); 3] =
    [("XDG_DOCUMENTS_DIR", "Documents"), ("XDG_DESKTOP_DIR", "Desktop"), ("XDG_DOWNLOAD_DIR", "Downloads")];

/// Extensions that name no kind of thing by themselves: a `pdf` can be a deck, a contract or a tax
/// return, so its name must say which.
const SHARED_EXTENSIONS: [&str; 3] = ["pdf", "txt", "md"];

/// Words that say a file is a newer one, not which one.
const FILLER: [&str; 5] = ["new", "latest", "updated", "current", "revised"];

/// What looking for one file came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Located {
    Found(String),
    /// Nothing in the folders could be it.
    Missing,
    /// More than one could be it: none is chosen.
    Ambiguous(usize),
    /// The label says nothing that tells one file from another.
    TooGeneral,
    /// There were more files of that kind than can be told apart reliably.
    TooMany,
    /// Not now.
    Later(String),
}

impl Located {
    /// What to tell the person when no file was chosen.
    pub fn words(&self, label: &str) -> Option<String> {
        match self {
            Located::Found(_) | Located::Later(_) => None,
            Located::Missing => Some(format!("couldn't find the {label}")),
            Located::Ambiguous(n) => Some(format!("found {n} files that could be the {label}")),
            Located::TooGeneral => Some(format!("couldn't tell which file you mean by the {label}")),
            Located::TooMany => Some(format!("there are too many files to tell which one is the {label}")),
        }
    }
}

/// The folders to search for a person at `home`, given the text of their `user-dirs.dirs` (which
/// can move a folder, or rename it): the ones that exist and lie inside `home`, without repeats and
/// without a folder inside another.
pub fn search_roots_in(home: &Path, user_dirs: Option<&str>) -> Vec<PathBuf> {
    let configured = |key: &str| -> Option<PathBuf> {
        let line = user_dirs?.lines().find_map(|l| l.trim().strip_prefix(&format!("{key}=")))?;
        let value = line.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
        Some(PathBuf::from(value))
    };
    let mut roots: Vec<PathBuf> = Vec::new();
    for (key, default) in FOLDERS {
        let dir = configured(key)
            .or_else(|| std::env::var_os(key).map(PathBuf::from))
            .unwrap_or_else(|| home.join(default));
        let dir = dir.canonicalize().unwrap_or(dir);
        if dir.is_dir() && dir.starts_with(home) && dir != home && !roots.iter().any(|r| dir.starts_with(r)) {
            roots.retain(|r| !r.starts_with(&dir));
            roots.push(dir);
        }
    }
    roots
}

/// The person's folders, from their own session.
pub fn search_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from).and_then(|h| h.canonicalize().ok()) else {
        return Vec::new();
    };
    let dirs = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).ok();
    search_roots_in(&home, dirs.as_deref())
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_lowercase).collect()
}

/// The words of `file`'s label that tell it from another of its kind.
fn distinguishing(file: &NamedFile) -> Vec<String> {
    file.distinguishing_words().into_iter().filter(|w| !FILLER.contains(&w.as_str())).collect()
}

/// Whether the file at `path` could be what `file` names.
pub fn could_be(file: &NamedFile, kind: FileKind, path: &str) -> bool {
    let words = distinguishing(file);
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if words.is_empty() || !kind.extensions().contains(&ext.as_str()) {
        return false;
    }
    let name = tokens(p.file_stem().and_then(|s| s.to_str()).unwrap_or(""));
    let noun = tokens(&file.label).pop().unwrap_or_default();
    words.iter().all(|w| name.contains(w)) && (!SHARED_EXTENSIONS.contains(&ext.as_str()) || name.contains(&noun))
}

/// What one `glob` listed: its paths, and whether the tool cut the list short (it lists the newest
/// fifty, so a cut list may be missing the file looked for).
fn listed(said: &str) -> (Vec<&str>, bool) {
    let paths = said.lines().take_while(|l| !l.trim().is_empty()).filter(|l| l.starts_with('/')).collect();
    (paths, said.lines().any(|l| l.starts_with("... and ")))
}

/// Find `file` among the files in `roots`, with the one `glob` the tools offer.
pub fn locate(hands: &mut Hands<'_>, file: &NamedFile, kind: FileKind, roots: &[String]) -> Located {
    if distinguishing(file).is_empty() {
        return Located::TooGeneral;
    }
    let mut candidates: Vec<String> = Vec::new();
    let mut cut = false;
    for ext in kind.extensions() {
        for root in roots {
            if hands.out_of_time() {
                return Located::Later(Failed::Later("I ran out of time and will pick this up again".into()).words().to_string());
            }
            let said = hands.call("glob", json!({ "pattern": format!("**/*.{ext}"), "path": root }));
            let (paths, truncated) = listed(&said);
            cut |= truncated;
            for path in paths.into_iter().filter(|p| could_be(file, kind, p)) {
                if !candidates.iter().any(|c| c == path) {
                    candidates.push(path.to_string());
                }
            }
        }
    }
    match candidates.len() {
        1 => Located::Found(candidates.remove(0)),
        0 if cut => Located::TooMany,
        0 => Located::Missing,
        n => Located::Ambiguous(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commitment_files::Role;

    fn file(label: &str) -> (NamedFile, FileKind) {
        let f = NamedFile { role: Role::Attach, label: label.into() };
        let kind = f.kind().unwrap();
        (f, kind)
    }

    #[test]
    fn a_name_must_carry_the_distinguishing_word_as_a_whole_word() {
        let (deck, kind) = file("Q4 deck");
        assert!(could_be(&deck, kind, "/h/Documents/Q4-deck.pptx"));
        assert!(could_be(&deck, kind, "/h/Documents/q4_board_review.ydeck"), "its own extension needs no noun");
        assert!(!could_be(&deck, kind, "/h/Documents/q4-tax-return.pdf"), "a pdf must say deck too");
        assert!(!could_be(&deck, kind, "/h/Documents/Q40-deck.pptx"), "Q40 is not Q4");
        assert!(!could_be(&deck, kind, "/h/Documents/Q4-budget.csv"), "not a deck's extension");
    }

    #[test]
    fn a_label_with_no_distinguishing_word_names_no_file() {
        let (contract, kind) = file("contract");
        assert!(!could_be(&contract, kind, "/h/Documents/journal.md"));
        assert!(!could_be(&contract, kind, "/h/Documents/contract.md"));
        let (new_deck, kind) = file("new deck");
        assert!(!could_be(&new_deck, kind, "/h/Documents/new-deck.pptx"), "'new' only says newer");
    }

    #[test]
    fn a_shared_extension_needs_the_noun_in_the_name() {
        let (c, kind) = file("Acme contract");
        assert!(could_be(&c, kind, "/h/Documents/acme-contract.docx"));
        assert!(could_be(&c, kind, "/h/Documents/Acme-contract-v2.pdf"));
        assert!(!could_be(&c, kind, "/h/Documents/acme-notes.pdf"));
        assert!(!could_be(&c, kind, "/h/Documents/acme-journal.md"));
    }

    #[test]
    fn roots_are_the_document_folders_and_never_home_itself() {
        let home = std::env::temp_dir().join(format!("yantrik-roots-{}", std::process::id()));
        for d in ["Documents", "Desktop", "Downloads", "Work/Docs", "Music"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let home = home.canonicalize().unwrap();
        let names = |roots: Vec<PathBuf>| -> Vec<String> {
            roots.iter().map(|r| r.strip_prefix(&home).unwrap().to_string_lossy().into_owned()).collect()
        };
        assert_eq!(names(search_roots_in(&home, None)), ["Documents", "Desktop", "Downloads"]);
        let moved = "XDG_DOCUMENTS_DIR=\"$HOME/Work/Docs\"\nXDG_DESKTOP_DIR=\"/etc\"\n";
        // A moved folder is followed; one that leads outside home is not searched.
        assert_eq!(names(search_roots_in(&home, Some(moved))), ["Work/Docs", "Downloads"]);
        let _ = std::fs::remove_dir_all(&home);
    }
}
