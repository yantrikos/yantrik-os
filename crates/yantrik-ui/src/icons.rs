//! Resolve a `.desktop` `Icon=` value to an image the UI can draw.
//!
//! The scanner has always read the `Icon=` field; it just never went anywhere, and the
//! launcher drew a single character instead. This is the missing lookup. It follows the
//! freedesktop layout loosely rather than implementing the full icon-theme spec: a fixed
//! preference order of themes, a fixed set of size directories, PNG before SVG. That covers
//! what real `.desktop` files on this target actually reference, and it degrades to "no icon"
//! rather than to a wrong one — the grid then draws a stroke glyph for the app's category.
//!
//! Absolute paths are honoured for the formats Slint can decode. `.xpm` is skipped: the
//! decoder does not read it, and a failed decode costs a warning per open of the launcher.
//!
//! Results are cached per name on the UI thread (`slint::Image` is not `Send`), because the
//! grid repopulates on every search keystroke and 49 decodes per keystroke is not free.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Themes to search, most complete first. `hicolor` is the freedesktop fallback every app
/// installs into, so it is last and always present.
const THEMES: &[&str] = &[
    "Papirus-Dark",
    "Papirus",
    "Qogir-dark",
    "Qogir",
    "breeze-dark",
    "breeze",
    "Adwaita",
    "Humanity",
    "hicolor",
];

/// Size/context directories, in the order a 28px badge prefers them.
const SIZE_DIRS: &[&str] = &[
    "scalable/apps",
    "64x64/apps",
    "48x48/apps",
    "32x32/apps",
    "24x24/apps",
    "scalable/mimetypes",
    "48x48/mimetypes",
    "scalable/places",
    "48x48/places",
    "scalable/status",
    "48x48/status",
    "scalable/devices",
    "48x48/devices",
    "48x48/categories",
];

const ICON_ROOTS: &[&str] = &["/usr/share/icons", "/usr/local/share/icons"];
const PIXMAP_DIRS: &[&str] = &["/usr/share/pixmaps"];

/// Extensions the image decoder can actually read, in preference order.
const EXTS: &[&str] = &["png", "svg"];

thread_local! {
    static CACHE: RefCell<HashMap<String, Option<slint::Image>>> = RefCell::new(HashMap::new());
}

/// Look up an icon by `.desktop` `Icon=` value. `None` when nothing decodable was found.
pub fn resolve(name: &str) -> Option<slint::Image> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(name).cloned()) {
        return hit;
    }
    let found = locate(name).and_then(|path| load(&path));
    CACHE.with(|c| c.borrow_mut().insert(name.to_string(), found.clone()));
    found
}

/// Find a file for the name without decoding it.
fn locate(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() {
        return decodable(p).then(|| p.to_path_buf());
    }

    // Some entries carry an extension already ("foo.png"); strip it so the search below
    // does not look for "foo.png.png".
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|_| p.extension().is_some_and(|e| EXTS.iter().any(|x| e == *x)))
        .unwrap_or(name);

    for root in ICON_ROOTS {
        for theme in THEMES {
            for dir in SIZE_DIRS {
                for ext in EXTS {
                    let candidate = Path::new(root)
                        .join(theme)
                        .join(dir)
                        .join(format!("{stem}.{ext}"));
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    for dir in PIXMAP_DIRS {
        for ext in EXTS {
            let candidate = Path::new(dir).join(format!("{stem}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    tracing::debug!(icon = name, "No decodable icon found in any theme");
    None
}

fn decodable(p: &Path) -> bool {
    p.is_file()
        && p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("svg")
                || e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
            .unwrap_or(false)
}

fn load(path: &Path) -> Option<slint::Image> {
    match slint::Image::load_from_path(path) {
        Ok(img) => Some(img),
        Err(e) => {
            // SVG support depends on how Slint was built; an SVG that will not decode is
            // reported once and then treated as missing, which the caller already handles.
            tracing::debug!(path = %path.display(), error = %e, "Icon found but did not decode");
            None
        }
    }
}

/// Map a freedesktop `Categories=` string to one of the launcher's category ids.
///
/// The first recognised main category wins, checked in an order that resolves the common
/// overlaps ("Development;Utility;" is a developer tool, not a utility). Anything else,
/// including an empty field, files under `utility` — a real app with a vague category is
/// still more useful in the launcher than one hidden by a filter.
pub fn category_id(categories: &str) -> &'static str {
    let has = |c: &str| categories.split(';').any(|t| t.trim().eq_ignore_ascii_case(c));
    for (main, id) in CATEGORY_TABLE {
        if has(main) {
            return id;
        }
    }
    "utility"
}

/// Human label for a category id.
pub fn category_label(id: &str) -> &'static str {
    CATEGORY_TABLE
        .iter()
        .find(|(_, cid)| *cid == id)
        .map(|(main, _)| match *main {
            "Network" => "Internet",
            "AudioVideo" => "Media",
            "Game" => "Games",
            "Utility" => "Utilities",
            other => other,
        })
        .unwrap_or("Utilities")
}

/// (freedesktop main category, launcher id), in resolution priority order.
/// Kept as a table so the id order — which is also the rail's display order — lives in one place.
pub const CATEGORY_TABLE: &[(&str, &str)] = &[
    ("Development", "development"),
    ("Office", "office"),
    ("Network", "network"),
    ("AudioVideo", "audiovideo"),
    ("Graphics", "graphics"),
    ("Game", "game"),
    ("Education", "education"),
    ("Science", "science"),
    ("System", "system"),
    ("Settings", "settings"),
    ("Utility", "utility"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_beats_utility_when_both_present() {
        assert_eq!(category_id("Utility;Development;TextEditor;"), "development");
    }

    #[test]
    fn unknown_and_empty_file_under_utility() {
        assert_eq!(category_id(""), "utility");
        assert_eq!(category_id("Wine;Emulator;"), "utility");
    }

    #[test]
    fn labels_are_friendly_where_freedesktop_names_are_not() {
        assert_eq!(category_label("network"), "Internet");
        assert_eq!(category_label("audiovideo"), "Media");
        assert_eq!(category_label("development"), "Development");
    }

    #[test]
    fn xpm_is_never_decodable() {
        assert!(!decodable(Path::new("/usr/share/pixmaps/python3.12.xpm")));
    }
}
