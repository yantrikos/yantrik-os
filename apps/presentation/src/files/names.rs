//! Where a deck without a name goes, and what its exports are called.

use std::path::{Path, PathBuf};

use crate::deck::{slug, EXTENSION};

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// `~/` at the start of a path, as the person and the Mind both write it.
pub fn expanded(value: &str) -> PathBuf {
    match value.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(value),
    }
}

/// Where a deck that was never given a name is put.
///
/// The screen has no file dialog, and adding a modal to a 1900-line shared screen so that one
/// button can ask a question is a larger change than the button is worth. So Save always has a
/// destination, the destination is chosen here, and the path it chose is on screen in the notes
/// bar and in `describe`: a file a person cannot find is the same as no file. `save_as` on the
/// control surface is there for anyone who wants to choose.
pub fn default_deck_dir() -> PathBuf {
    home().join("Documents/Presentations")
}

/// The name Save would try first for a deck called `title` in `dir`, the default folder. It looks
/// at no disk: the real name is chosen in the job that writes, which counts up past a name that is
/// taken.
pub fn first_choice(dir: &Path, title: &str) -> PathBuf {
    dir.join(format!("{}.{EXTENSION}", slug(title)))
}

/// A path in `dir` for `title` that nothing is using yet.
///
/// Collision-safe by counting up rather than by overwriting: two decks both called "Untitled" is
/// the normal case, and the second one silently replacing the first is how a person loses work
/// they never knew was at risk. The bound exists so a directory that cannot be read, every
/// `exists()` answering true, ends in an error rather than a spin. Reads the disk, so it runs
/// in a job.
pub fn unused_path(dir: &Path, title: &str) -> Result<PathBuf, String> {
    let stem = slug(title);
    for n in 1..=999 {
        let name = if n == 1 { format!("{stem}.{EXTENSION}") } else { format!("{stem}-{n}.{EXTENSION}") };
        let candidate = dir.join(name);
        if std::fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    Err(format!("{} already holds 999 decks called {stem}; give this one a name with Save As", dir.display()))
}

/// The path an export goes to when nobody named one: beside the deck, same stem.
pub fn export_path(deck: Option<&Path>, dir: &Path, title: &str, extension: &str) -> PathBuf {
    let base = match deck {
        Some(p) => p.to_path_buf(),
        None => first_choice(dir, title),
    };
    base.with_extension(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_decks_with_the_same_name_do_not_overwrite_each_other() {
        let dir = std::env::temp_dir().join(format!("ypresent-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = unused_path(&dir, "My Talk!").unwrap();
        assert_eq!(first.file_name().unwrap(), "my-talk.ydeck");
        std::fs::write(&first, "taken").unwrap();

        let second = unused_path(&dir, "My Talk!").unwrap();
        assert_eq!(second.file_name().unwrap(), "my-talk-2.ydeck");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "taken", "the first is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_export_goes_beside_the_deck_with_the_same_stem() {
        let deck = Path::new("/home/p/talks/talk.ydeck");
        let dir = Path::new("/home/p/Documents/Presentations");
        assert_eq!(export_path(Some(deck), dir, "Talk", "md"), Path::new("/home/p/talks/talk.md"));
        assert_eq!(export_path(Some(deck), dir, "Talk", "outline.md"), Path::new("/home/p/talks/talk.outline.md"));
        assert_eq!(export_path(None, dir, "My Talk", "md"), dir.join("my-talk.md"));
    }

    #[test]
    fn a_path_is_expanded_from_the_home_only_when_it_starts_with_one() {
        assert_eq!(expanded("/etc/hostname"), Path::new("/etc/hostname"));
        assert_eq!(expanded("relative/x"), Path::new("relative/x"));
        assert!(expanded("~/a/b").ends_with("a/b"));
    }
}
