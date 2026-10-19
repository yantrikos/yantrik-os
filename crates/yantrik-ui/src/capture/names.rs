//! Where a capture is saved and what it is called.
//!
//! `Screenshot 2026-10-15 at 14.32.07.png` and `Recording 2026-10-15 at 14.32.07.mp4`, in local
//! time: the name a person sees in Files sorts by when it was taken and says what it is. The
//! old names were `screenshot-2026-10-15-093207.png` in UTC, so a picture taken at 15:02 in
//! Kolkata was filed as 09:32 — the time on no clock the person owns. Dots in the time, not
//! colons: a colon is a path separator to half the machines a file gets copied to.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

/// What a capture is, which decides its folder, its word and its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Screenshot,
    Recording,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Screenshot => "Screenshot",
            Kind::Recording => "Recording",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Kind::Screenshot => "png",
            Kind::Recording => "mp4",
        }
    }

    /// The folder under the home: `~/Pictures/Screenshots` or `~/Videos/Recordings`.
    pub fn dir(self, home: &Path) -> PathBuf {
        match self {
            Kind::Screenshot => home.join("Pictures/Screenshots"),
            Kind::Recording => home.join("Videos/Recordings"),
        }
    }
}

/// The file name for a capture taken at `at`.
pub fn file_name(kind: Kind, at: &DateTime<Local>) -> String {
    format!("{} {} at {}.{}", kind.word(), at.format("%Y-%m-%d"), at.format("%H.%M.%S"), kind.extension())
}

/// The person's home, from the environment the session gave the shell.
pub fn home() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set, so there is nowhere to save a capture".to_string())
}

/// The kind's folder, private to the person: made 0700 if it was missing, and set to 0700 if it was
/// not. Refused when it is a link, not a folder, or someone else's.
///
/// A capture's name is predictable to the second, and grim and wf-recorder open it with O_TRUNC,
/// following links: anyone who could put `Screenshot <next second>.png -> elsewhere` in the folder
/// had the person's desktop written there, and a 0755 folder of 0644 files was readable to every
/// account on the machine, a mind's own account among them (security review of #828, finding 3).
/// A folder only the person can enter closes both, whatever happens inside it.
pub fn folder(kind: Kind, home: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let dir = kind.dir(home);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not make {}: {e}", parent.display()))?;
    }
    match std::fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("could not make {}: {e}", dir.display()))?,
        Err(e) => return Err(format!("could not look at {}: {e}", dir.display())),
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!("{} is a link, so nothing is saved through it", dir.display()))
        }
        Ok(meta) if !meta.is_dir() => return Err(format!("{} is not a folder", dir.display())),
        Ok(meta) if meta.uid() != unsafe { libc::geteuid() } => {
            return Err(format!("{} belongs to another account, so nothing is saved in it", dir.display()))
        }
        Ok(meta) if meta.mode() & 0o777 != 0o700 => {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("could not make {} private: {e}", dir.display()))?
        }
        Ok(_) => {}
    }
    Ok(dir)
}

/// The names to try for a capture taken at `at`, in order. Two screenshots in one second
/// (Shift+Print pressed twice) are `… at 14.32.07.png` and `… at 14.32.07 (2).png`, never one
/// written over the other.
fn names_for(kind: Kind, at: &DateTime<Local>) -> impl Iterator<Item = String> {
    let name = file_name(kind, at);
    let stem = name.trim_end_matches(&format!(".{}", kind.extension())).to_string();
    std::iter::once(name).chain((2..1000).map(move |n| format!("{stem} ({n}).{}", kind.extension())))
}

/// A path in the kind's private [`folder`] that nothing is at yet, not even a dangling link
/// (`symlink_metadata`, not `exists`, which follows one and says no). For a recording, which
/// wf-recorder opens itself; a screenshot is [`create_fresh`].
pub fn fresh_path(kind: Kind, home: &Path, at: &DateTime<Local>) -> Result<PathBuf, String> {
    let dir = folder(kind, home)?;
    names_for(kind, at)
        .map(|name| dir.join(name))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .ok_or_else(|| format!("{} already holds a thousand captures with this name", dir.display()))
}

/// A new file for a capture, made here and handed back open: `O_CREAT|O_EXCL|O_NOFOLLOW`, mode
/// 0600, in the kind's private [`folder`]. Nothing already at a name - a file, a folder, a link
/// that leads anywhere or nowhere - is opened or followed; the next name is tried.
pub fn create_fresh(kind: Kind, home: &Path, at: &DateTime<Local>) -> Result<(PathBuf, std::fs::File), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = folder(kind, home)?;
    for name in names_for(kind, at) {
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not make {}: {e}", path.display())),
        }
    }
    Err(format!("{} already holds a thousand captures with this name", dir.display()))
}

/// Whether `path` is a capture this desktop saved: a file directly in one of the two folders,
/// named as above. The notification buttons (Open, Show in Files, Delete) act only on such a
/// path, so a notification that names any other file — whoever filed it — moves nothing.
pub fn is_capture(path: &Path, home: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    let Some(parent) = path.parent() else { return false };
    [Kind::Screenshot, Kind::Recording].into_iter().any(|kind| {
        parent == kind.dir(home)
            && name.starts_with(&format!("{} ", kind.word()))
            && name.ends_with(&format!(".{}", kind.extension()))
            && !name.contains('/')
    }) && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 15, 14, 32, 7).unwrap()
    }

    #[test]
    fn a_capture_is_named_for_what_it_is_and_when_by_the_local_clock() {
        assert_eq!(file_name(Kind::Screenshot, &at()), "Screenshot 2026-10-15 at 14.32.07.png");
        assert_eq!(file_name(Kind::Recording, &at()), "Recording 2026-10-15 at 14.32.07.mp4");
        let home = Path::new("/home/p");
        assert_eq!(Kind::Screenshot.dir(home), Path::new("/home/p/Pictures/Screenshots"));
        assert_eq!(Kind::Recording.dir(home), Path::new("/home/p/Videos/Recordings"));
    }

    #[test]
    fn two_captures_in_one_second_are_two_files() {
        let home = std::env::temp_dir().join(format!("yantrik-names-{}", std::process::id()));
        let first = fresh_path(Kind::Screenshot, &home, &at()).unwrap();
        std::fs::write(&first, b"x").unwrap();
        let second = fresh_path(Kind::Screenshot, &home, &at()).unwrap();
        assert_ne!(first, second);
        assert!(second.ends_with("Screenshot 2026-10-15 at 14.32.07 (2).png"), "{second:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Security review of #828, finding 3: a capture is made private and fresh. The folders are
    /// 0700 (made so, or set so); a planted link at the next name, dangling or not, is passed
    /// over, never written through; the file is 0600; and a folder that is a link is refused.
    #[test]
    fn a_capture_is_private_and_never_written_through_a_link() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let home = std::env::temp_dir().join(format!("yantrik-names-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mode = |p: &Path| std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777;

        // An existing folder, the old 0755, is made private.
        std::fs::create_dir_all(Kind::Screenshot.dir(&home)).unwrap();
        std::fs::set_permissions(Kind::Screenshot.dir(&home), std::fs::Permissions::from_mode(0o755)).unwrap();
        let elsewhere = home.join("elsewhere.png");
        let planted = Kind::Screenshot.dir(&home).join(file_name(Kind::Screenshot, &at()));
        symlink(&elsewhere, &planted).unwrap();
        let (path, mut file) = create_fresh(Kind::Screenshot, &home, &at()).unwrap();
        std::io::Write::write_all(&mut file, b"png").unwrap();
        assert_eq!(mode(&Kind::Screenshot.dir(&home)), 0o700);
        assert_ne!(path, planted, "the planted link is passed over");
        assert!(path.ends_with("Screenshot 2026-10-15 at 14.32.07 (2).png"), "{path:?}");
        assert!(!elsewhere.exists(), "nothing was written where the link leads");
        assert_eq!(mode(&path), 0o600);

        // A recording's name skips a dangling link too (exists() said there was nothing there),
        // and its folder is made 0700.
        let rec_dir = Kind::Recording.dir(&home);
        std::fs::create_dir_all(rec_dir.parent().unwrap()).unwrap();
        let fresh = fresh_path(Kind::Recording, &home, &at()).unwrap();
        assert_eq!(mode(&rec_dir), 0o700);
        symlink(&elsewhere, &fresh).unwrap();
        assert_ne!(fresh_path(Kind::Recording, &home, &at()).unwrap(), fresh);

        // A folder that is a link is no place to save.
        let other = home.join("other-place");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::remove_dir_all(&rec_dir).unwrap();
        symlink(&other, &rec_dir).unwrap();
        assert!(fresh_path(Kind::Recording, &home, &at()).unwrap_err().contains("is a link"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn only_a_file_this_desktop_saved_counts_as_a_capture() {
        let home = Path::new("/home/p");
        assert!(is_capture(Path::new("/home/p/Pictures/Screenshots/Screenshot 2026-10-15 at 14.32.07.png"), home));
        assert!(is_capture(Path::new("/home/p/Videos/Recordings/Recording 2026-10-15 at 14.32.07 (2).mp4"), home));
        for not in [
            "/home/p/Documents/taxes.pdf",
            "/home/p/Pictures/Screenshots/notes.txt",
            "/home/p/Pictures/Screenshots/../../.ssh/id_ed25519",
            "/home/p/Pictures/Screenshots/sub/Screenshot x.png",
            "/home/p/Videos/Recordings/Screenshot 2026-10-15 at 14.32.07.png",
            "/etc/Pictures/Screenshots/Screenshot a.png",
        ] {
            assert!(!is_capture(Path::new(not), home), "{not}");
        }
    }
}
