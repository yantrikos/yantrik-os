//! What the desktop folder holds, and what each thing is.
//!
//! The folder is read as it is: a name is whatever the file system says, so it can hold control
//! characters, a right-to-left override or a very long line, and a person (or a mind) chose it.
//! [`display_name`] is the one place a name is made safe to draw; the icon label, `describe` and
//! the notices all go through it.

use std::path::Path;
use std::time::SystemTime;

/// How many entries are listed. A desktop with more than this is a folder somebody dumped
/// something into; the rest are left out, and [`Listing::more`] says how many.
pub const MAX_ITEMS: usize = 1500;

/// What an icon shows when it has no picture of its own, and how Arrange sorts by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// An app shortcut: a `.desktop` file of Type=Application. Sorts first, as in the design.
    App,
    Folder,
    Image,
    Pdf,
    Blend,
    Document,
    Sheet,
    Audio,
    Video,
    Archive,
    Code,
    Text,
    Other,
}

impl Kind {
    /// The word `describe` and the icon glyph table use. Stable: a caller may branch on it.
    pub fn word(self) -> &'static str {
        match self {
            Kind::App => "app",
            Kind::Folder => "folder",
            Kind::Image => "image",
            Kind::Pdf => "pdf",
            Kind::Blend => "blend",
            Kind::Document => "document",
            Kind::Sheet => "sheet",
            Kind::Audio => "audio",
            Kind::Video => "video",
            Kind::Archive => "archive",
            Kind::Code => "code",
            Kind::Text => "text",
            Kind::Other => "file",
        }
    }
}

/// The kind a name says it is. A folder is a folder whatever it is called.
pub fn kind_of(name: &str, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let ext = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => return Kind::Other,
    };
    match ext.as_str() {
        "desktop" => Kind::App,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "tif" | "tiff" | "avif" => Kind::Image,
        "pdf" => Kind::Pdf,
        "blend" => Kind::Blend,
        "odt" | "doc" | "docx" | "rtf" | "ydoc" => Kind::Document,
        "ods" | "xls" | "xlsx" | "csv" | "tsv" => Kind::Sheet,
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" => Kind::Audio,
        "mp4" | "mkv" | "webm" | "mov" | "avi" => Kind::Video,
        "zip" | "tar" | "gz" | "xz" | "bz2" | "zst" | "7z" | "rar" => Kind::Archive,
        "rs" | "py" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "sh" | "json" | "toml" | "yaml"
        | "yml" | "html" | "css" | "slint" => Kind::Code,
        "txt" | "md" | "log" | "ini" | "conf" => Kind::Text,
        _ => Kind::Other,
    }
}

/// One thing on the desktop. `name` is the real file name, the key everything else uses;
/// [`display_name`] is what is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

impl Item {
    pub fn label(&self) -> String {
        display_name(&self.name)
    }
}

/// The desktop folder as one listing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Listing {
    pub items: Vec<Item>,
    /// Entries past [`MAX_ITEMS`] that were left out.
    pub more: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ItemsError {
    #[error("the desktop folder cannot be read: {0}")]
    Read(#[from] std::io::Error),
}

/// Read `dir`. Dot-files are left out, as Files leaves them out. A link is listed as what it
/// points at (a link to a folder is a folder), and a dangling link as a plain file, so the
/// person can still see it and put it away.
pub fn scan(dir: &Path) -> Result<Listing, ItemsError> {
    let mut items = Vec::new();
    let mut more = 0usize;
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if items.len() >= MAX_ITEMS {
            more += 1;
            continue;
        }
        // `metadata` follows a link; a dangling one falls back to the link's own entry.
        let meta = std::fs::metadata(entry.path()).or_else(|_| entry.metadata());
        let Ok(meta) = meta else { continue };
        items.push(Item {
            kind: kind_of(&name, meta.is_dir()),
            size: if meta.is_dir() { 0 } else { meta.len() },
            mtime: meta.modified().ok(),
            name,
        });
    }
    Ok(Listing { items, more })
}

/// The order a fresh desktop fills in: shortcuts, then folders, then files, each by name without
/// regard to case. Column-first placement walks this list.
pub fn flow_order(items: &[Item]) -> Vec<String> {
    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by(|a, b| {
        group(a.kind)
            .cmp(&group(b.kind))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted.into_iter().map(|i| i.name.clone()).collect()
}

fn group(kind: Kind) -> u8 {
    match kind {
        Kind::App => 0,
        Kind::Folder => 1,
        _ => 2,
    }
}

/// `name` made safe to draw on one label: control characters, the bidi controls that can make
/// one name read as another, and zero-width characters are replaced, and a name that is nothing
/// but those is shown as `?`. Length is left to the label's own ellipsis.
pub fn display_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || is_invisible(c) { '\u{fffd}' } else { c })
        .collect();
    if cleaned.chars().all(|c| c == '\u{fffd}' || c.is_whitespace()) {
        "?".to_string()
    } else {
        cleaned
    }
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{feff}'
    )
}

/// Whether `name` can be the name of one thing directly inside the desktop: one path component,
/// not `.` or `..`, no separator, no NUL. Every action that takes a name from outside (a mind's
/// `desktop_archive`, an inline rename) goes through this, so none of them can reach out of the
/// folder.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
        && name.len() <= 255
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_says_what_it_is() {
        assert_eq!(kind_of("Report.PDF", false), Kind::Pdf);
        assert_eq!(kind_of("summit.blend", false), Kind::Blend);
        assert_eq!(kind_of("shot.JPG", false), Kind::Image);
        assert_eq!(kind_of("blender.desktop", false), Kind::App);
        assert_eq!(kind_of("README", false), Kind::Other);
        assert_eq!(kind_of(".png", false), Kind::Other, "a dot-file with no stem has no extension");
        assert_eq!(kind_of("Starfall.d", true), Kind::Folder, "a folder is a folder whatever it is called");
    }

    #[test]
    fn a_fresh_desktop_fills_in_shortcuts_then_folders_then_files() {
        let mk = |n: &str, k| Item { name: n.into(), kind: k, size: 0, mtime: None };
        let items = vec![
            mk("b.txt", Kind::Text),
            mk("Zeta", Kind::Folder),
            mk("A.txt", Kind::Text),
            mk("notes.desktop", Kind::App),
            mk("alpha", Kind::Folder),
        ];
        assert_eq!(flow_order(&items), ["notes.desktop", "alpha", "Zeta", "A.txt", "b.txt"]);
    }

    #[test]
    fn a_name_cannot_fake_another_or_hide() {
        assert_eq!(display_name("gpj.\u{202e}fdp"), "gpj.\u{fffd}fdp");
        assert_eq!(display_name("a\nb\x1b[2J"), "a\u{fffd}b\u{fffd}[2J");
        assert_eq!(display_name("\u{200b}\u{200b}"), "?");
        assert_eq!(display_name("Report.pdf"), "Report.pdf");
    }

    #[test]
    fn only_a_single_component_is_a_name_on_the_desktop() {
        for bad in ["", ".", "..", "a/b", "../x", "/etc/passwd", "a\0b"] {
            assert!(!is_plain_name(bad), "{bad:?}");
        }
        assert!(is_plain_name("Report (2).pdf"));
        assert!(!is_plain_name(&"x".repeat(256)));
    }

    #[test]
    fn scan_lists_what_is_there_without_dot_files_and_follows_links_to_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::write(d.join("a.txt"), "hi").unwrap();
        std::fs::write(d.join(".hidden"), "x").unwrap();
        std::fs::create_dir(d.join("Folder")).unwrap();
        std::os::unix::fs::symlink(d.join("Folder"), d.join("LinkToFolder")).unwrap();
        std::os::unix::fs::symlink(d.join("gone"), d.join("Dangling")).unwrap();
        let listing = scan(d).unwrap();
        let mut names: Vec<_> = listing.items.iter().map(|i| (i.name.as_str(), i.kind)).collect();
        names.sort();
        assert_eq!(
            names,
            [("Dangling", Kind::Other), ("Folder", Kind::Folder), ("LinkToFolder", Kind::Folder), ("a.txt", Kind::Text)]
        );
        assert_eq!(listing.more, 0);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_empty_desktop() {
        assert!(scan(Path::new("/nonexistent/yantrik/desktop")).is_err());
    }
}
