//! Writing a file so that it is whole or absent, never half: the one atomic writer
//! (design/app-framework-2026-10-21.md, §6). Seven apps carried a copy, each a little different.
//!
//! `fs::write` truncates before it writes, so a crash, a full disk or a kill between the
//! truncate and the last byte leaves a file that parses as nothing, and everything the person
//! saved is gone although nothing deleted it. Here the bytes go to a temporary file in the same
//! directory (a rename across filesystems is a copy, and a copy is not atomic), with owner-only
//! permissions, synced to disk, then renamed over the target, and the directory is synced so the
//! new entry is durable too. The temporary is removed on every failing path, so a failed save
//! leaves nothing beside the file for the next person to wonder at.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path`, whole or not at all. The directory is made if it is missing.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let temp = dir.join(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        // The rename is atomic only with respect to bytes that have reached the disk.
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    // The directory entry too, where the platform allows a directory to be synced.
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// [`write_atomically`], for text.
pub fn write_text_atomically(path: &Path, text: &str) -> io::Result<()> {
    write_atomically(path, text.as_bytes())
}

/// Removes the temporaries a killed process left beside `path` (`.<name>.<pid>-<n>.tmp`), once
/// they are older than `older_than`: young enough to be another process mid-write, they stay.
/// Nothing else ever sweeps them. Failures are nothing; this is housekeeping.
pub fn sweep_stale_temps(path: &Path, older_than: std::time::Duration) {
    let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) else {
        return;
    };
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
        return;
    };
    let prefix = format!(".{name}.");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if !(file.starts_with(&prefix) && file.ends_with(".tmp")) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= older_than);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-json-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_file_is_whole_and_nothing_is_left_beside_it() {
        let dir = temp_dir("whole");
        let path = dir.join("nested/settings.json");
        write_atomically(&path, b"{\"a\":1}").unwrap();
        write_atomically(&path, b"{\"a\":2}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":2}");
        let left: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("settings.json")], "no temporary stays behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_temporary_is_swept_and_a_fresh_one_is_left() {
        let dir = temp_dir("sweep");
        let path = dir.join("settings.json");
        write_atomically(&path, b"{}").unwrap();
        let stale = dir.join(".settings.json.1-1.tmp");
        let fresh = dir.join(".settings.json.2-1.tmp");
        let other = dir.join(".other.json.1-1.tmp");
        for p in [&stale, &fresh, &other] {
            fs::write(p, b"half").unwrap();
        }
        // Zero age sweeps everything of this file's; a day's age sweeps nothing.
        sweep_stale_temps(&path, std::time::Duration::from_secs(86_400));
        assert!(stale.exists() && fresh.exists());
        sweep_stale_temps(&path, std::time::Duration::ZERO);
        assert!(!stale.exists() && !fresh.exists());
        assert!(other.exists(), "another file's temporary is not this file's business");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_as_it_was() {
        let dir = temp_dir("failed");
        let path = dir.join("settings.json");
        write_atomically(&path, b"old").unwrap();
        // The target is now a directory, so the rename fails after the temporary was written.
        let blocked = dir.join("settings.json.d");
        write_atomically(&blocked.join("x"), b"x").unwrap();
        let outcome = write_atomically(&blocked, b"new");
        assert!(outcome.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        let temps: Vec<_> = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(temps.is_empty(), "the temporary was removed");
        let _ = fs::remove_dir_all(&dir);
    }
}
