//! Where acknowledged notices go instead of being deleted, and the two disk calls that make
//! "stored" mean "on the disk".
//!
//! The live store drops a dismissed notification a week after it was made, and the oldest past
//! its cap. For a mind's notice that would delete something the OS answered `stored` for
//! (design/initiative-inbox-2026-10-16.md: "archive, never delete"). So a notice leaving the live
//! file is appended here first, one JSON record per line, and flushed before it goes.

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::notifications::Notification;

/// Past this, the archive is rolled to `notices-archive.N.jsonl` before the next append, so no one
/// file grows without end. Nothing is deleted.
pub const ROLL_BYTES: u64 = 16 * 1024 * 1024;

/// Past this, across the archive and its rolled files, `describe` says so: the person (or an
/// administrator) decides what to keep, never this service.
pub const WARN_BYTES: u64 = 64 * 1024 * 1024;

/// Only the person's account reads what minds told them and what they answered.
const PRIVATE: u32 = 0o600;

/// `notices-archive.jsonl`, beside the store.
pub fn path_beside(store: &Path) -> PathBuf {
    store.with_file_name("notices-archive.jsonl")
}

/// Append these records, one per line, and flush them to the disk. Nothing to append is not a
/// write at all.
pub fn append<'a>(path: &Path, records: impl Iterator<Item = &'a Notification>) -> std::io::Result<()> {
    let values: Vec<serde_json::Value> = records.map(serde_json::to_value).collect::<Result<_, _>>().map_err(std::io::Error::other)?;
    append_values(path, values.into_iter())
}

/// Append these JSON values, one per line, and flush them to the disk.
pub fn append_values(path: &Path, records: impl Iterator<Item = serde_json::Value>) -> std::io::Result<()> {
    let mut lines = String::new();
    for record in records {
        lines.push_str(&record.to_string());
        lines.push('\n');
    }
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() >= ROLL_BYTES) {
        roll(path)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).mode(PRIVATE).open(path)?;
    // A file made by an older build, under the umask, is made private too.
    file.set_permissions(std::fs::Permissions::from_mode(PRIVATE))?;
    file.write_all(lines.as_bytes())?;
    file.sync_all()?;
    sync_dir(path);
    Ok(())
}

/// Move the archive aside to the first free `notices-archive.N.jsonl`.
fn roll(path: &Path) -> std::io::Result<()> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let next = (1u32..)
        .map(|n| path.with_file_name(format!("{stem}.{n}.jsonl")))
        .find(|p| !p.exists())
        .expect("a free number");
    std::fs::rename(path, &next)?;
    sync_dir(path);
    Ok(())
}

/// What `describe` says about the archive, when its files together are past [`WARN_BYTES`].
pub fn size_note(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let total: u64 = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&stem))
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    (total > WARN_BYTES).then(|| {
        format!(
            "the notice archive beside {} holds {} MB; nothing is deleted from it, so it is the person's to keep or clear",
            path.display(),
            total / (1024 * 1024)
        )
    })
}

/// Write `bytes` to `path`, readable by this account only, and flush them to the disk before
/// answering.
pub fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(PRIVATE).open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(PRIVATE))?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Flush the directory holding `path`, so a rename or a new file in it is on the disk too. Best
/// effort: not every filesystem lets a directory be opened for this.
pub fn sync_dir(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(handle) = std::fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_archive_is_private_and_rolls_instead_of_growing() {
        let dir = std::env::temp_dir().join(format!("notices-archive-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = path_beside(&dir.join("notifications.json"));
        // An archive an older build made under the umask, already past the roll size.
        std::fs::write(&path, vec![b'x'; ROLL_BYTES as usize]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let record: Notification = serde_json::from_value(serde_json::json!({
            "id": "1", "app": "a", "title": "t", "body": "", "urgency": "normal",
            "created_at": "2026-10-16T00:00:00Z", "read": false, "dismissed": true,
            "actions": [], "source": "yantrik", "revision": 1
        }))
        .unwrap();
        append(&path, std::iter::once(&record)).unwrap();
        let rolled = dir.join("notices-archive.1.jsonl");
        assert_eq!(std::fs::metadata(&rolled).unwrap().len(), ROLL_BYTES, "rolled whole, not deleted");
        assert!(std::fs::metadata(&path).unwrap().len() < 1024);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, PRIVATE);
        let store = dir.join("store.tmp");
        write_synced(&store, b"{}").unwrap();
        assert_eq!(std::fs::metadata(&store).unwrap().permissions().mode() & 0o777, PRIVATE);
        assert!(size_note(&path).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
