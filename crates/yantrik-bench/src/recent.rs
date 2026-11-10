//! The one thing the Continue row offers.
//!
//! The desktop's Continue row used to read the Lens's recents, which the Lens empties when it
//! closes, so the row was there while the Lens was open and gone the moment it was not. It has
//! a record of its own now: the last file or app the person opened, kept in
//! `~/.yantrik/last_opened.json` and read back at the next start.
//!
//! What is kept is a name to show and what to open. Not a URL (a link can carry a token) and
//! not what was typed into the Lens. A file is offered only while it is still there.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::items::display_name;

const MAX_BYTES: u64 = 16 * 1024;
const MAX_TEXT: usize = 120;

/// What Continue would open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    File { path: String },
    App { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastOpened {
    pub target: Target,
    /// What the row says: the file's name, or the app's.
    pub title: String,
    /// Where it is, or what it is: "Desktop", "Notes".
    pub subtitle: String,
    /// Seconds since the epoch.
    pub at: u64,
}

impl LastOpened {
    pub fn new(target: Target, title: &str, subtitle: &str, at: u64) -> LastOpened {
        LastOpened { target, title: clip(&display_name(title)), subtitle: clip(&display_name(subtitle)), at }
    }

    /// Whether it can still be offered: an app always can (the shell checks it launches); a file
    /// only while it exists.
    pub fn is_open_able(&self) -> bool {
        match &self.target {
            Target::File { path } => std::fs::symlink_metadata(path).is_ok(),
            Target::App { .. } => true,
        }
    }
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

/// The record at `path`, or `None` when there is none or it is not one.
pub fn load(path: &Path) -> Option<LastOpened> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_BYTES).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(path: &Path, record: &LastOpened) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic::write(path, &serde_json::to_vec(record).map_err(std::io::Error::other)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_thing_opened_comes_back_after_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(".yantrik/last_opened.json");
        let rec = LastOpened::new(Target::File { path: "/h/Desktop/summit.blend".into() }, "summit.blend", "Desktop", 100);
        save(&p, &rec).unwrap();
        assert_eq!(load(&p), Some(rec));
        let app = LastOpened::new(Target::App { id: "notes".into() }, "Notes", "App", 200);
        save(&p, &app).unwrap();
        assert_eq!(load(&p), Some(app), "only the last one is kept");
    }

    #[test]
    fn a_missing_or_foreign_file_is_no_record() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load(&tmp.path().join("none.json")), None);
        std::fs::write(tmp.path().join("x.json"), "{\"hello\": 1}").unwrap();
        assert_eq!(load(&tmp.path().join("x.json")), None);
    }

    #[test]
    fn a_file_is_offered_only_while_it_is_there() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.txt");
        std::fs::write(&f, "x").unwrap();
        let rec = LastOpened::new(Target::File { path: f.to_string_lossy().into_owned() }, "a.txt", "", 1);
        assert!(rec.is_open_able());
        std::fs::remove_file(&f).unwrap();
        assert!(!rec.is_open_able());
    }

    #[test]
    fn the_words_are_cleaned_and_cut() {
        let rec = LastOpened::new(Target::App { id: "x".into() }, &format!("a\u{202e}b{}", "z".repeat(500)), "ok", 1);
        assert!(rec.title.starts_with("a\u{fffd}b") && rec.title.chars().count() == MAX_TEXT);
    }
}
