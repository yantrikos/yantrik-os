//! Crash-safe writes and boot recovery for the config file (config.yaml).
//!
//! The old in-place `std::fs::write` left a truncated or half-written file
//! behind when power died mid-write, and at the next boot the machine's whole
//! configuration — comments, defaults, provider keys — was gone. Saves keep a
//! `<path>.last-good` copy of the previous contents; the boot loader reads
//! through `load_with_recovery`, which falls back to that copy.

use std::path::{Path, PathBuf};

/// Replace `path` with `contents` without ever leaving it half-written.
///
/// If `path` already holds parseable YAML it is first copied to
/// `<path>.last-good`, so `load_with_recovery` can undo a bad save. If it
/// exists but does not parse it is renamed to `<path>.broken` instead of
/// being destroyed: config.yaml holds provider keys, so a copy that failed
/// to parse is still worth more intact than gone. The new contents go to a
/// sibling `<path>.tmp`, are fsynced, then renamed over `path`; the temp
/// file carries the existing file's permissions (else 0644) so the rename
/// cannot widen access to a file that holds provider keys.
pub fn save_atomic(path: &Path, contents: &str) -> Result<(), String> {
    #[cfg(unix)]
    let mut mode = 0o644u32;
    if let Ok(meta) = std::fs::metadata(path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            mode = meta.permissions().mode() & 0o777;
        }
        #[cfg(not(unix))]
        let _ = meta;
        match std::fs::read_to_string(path) {
            Ok(existing) => {
                if parses_as_yaml(&existing) {
                    let last_good = sibling(path, "last-good");
                    std::fs::copy(path, &last_good).map_err(|e| {
                        format!("Copy {} to {}: {}", path.display(), last_good.display(), e)
                    })?;
                } else {
                    // Unparseable: keep it aside rather than overwrite it.
                    let broken = sibling(path, "broken");
                    std::fs::rename(path, &broken).map_err(|e| {
                        format!("Rename {} to {}: {}", path.display(), broken.display(), e)
                    })?;
                }
            }
            // Unreadable; it was already lost — nothing to keep.
            Err(_) => {}
        }
    }

    let tmp = sibling(path, "tmp");
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| format!("Create {}: {}", tmp.display(), e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
        }
        file.write_all(contents.as_bytes())
            .map_err(|e| format!("Write {}: {}", tmp.display(), e))?;
        // The point of the exercise: the bytes are on disk before the rename.
        file.sync_all()
            .map_err(|e| format!("Sync {}: {}", tmp.display(), e))?;
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("Rename {} to {}: {}", tmp.display(), path.display(), e))
}

/// Read `path`, falling back to its `<path>.last-good` copy if `path` is gone
/// or does not parse as YAML — the boot-time recovery for a save that died
/// mid-write. Returns the file's contents; the caller parses them.
pub fn load_with_recovery(path: &Path) -> Result<String, String> {
    let last_good = sibling(path, "last-good");
    match std::fs::read_to_string(path) {
        Ok(contents) if parses_as_yaml(&contents) => Ok(contents),
        Err(read_err) => {
            if let Some(recovered) = recoverable(&last_good) {
                tracing::warn!(
                    path = %path.display(),
                    last_good = %last_good.display(),
                    "Could not read config; booting from last-good copy"
                );
                return Ok(recovered);
            }
            Err(format!("Read {}: {}", path.display(), read_err))
        }
        Ok(_) => {
            if let Some(recovered) = recoverable(&last_good) {
                tracing::warn!(
                    path = %path.display(),
                    last_good = %last_good.display(),
                    "Config does not parse as YAML; booting from last-good copy"
                );
                return Ok(recovered);
            }
            Err(format!("{} is not valid YAML and has no valid last-good copy", path.display()))
        }
    }
}

/// Last-good contents to boot from, if the file exists and parses.
fn recoverable(last_good: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(last_good).ok()?;
    parses_as_yaml(&contents).then_some(contents)
}

/// Whether `contents` is a parseable YAML document (an empty file parses, as
/// a null document).
fn parses_as_yaml(contents: &str) -> bool {
    serde_yaml::from_str::<serde_yaml::Value>(contents).is_ok()
}

/// `<path>.<extension>` — a sibling in the same directory, never a new dir.
fn sibling(path: &Path, extension: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "config.yaml".into());
    name.push(".");
    name.push(extension);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp dir unique to this test process and name; never /opt/yantrik.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yos-config_file-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn second_save_keeps_the_first_contents_as_last_good() {
        let dir = temp_dir("two-saves");
        let path = dir.join("config.yaml");
        save_atomic(&path, "key: one\n").unwrap();
        save_atomic(&path, "key: two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "key: two\n");
        assert_eq!(
            std::fs::read_to_string(sibling(&path, "last-good")).unwrap(),
            "key: one\n"
        );
        assert!(!sibling(&path, "tmp").exists());
    }

    #[test]
    fn truncated_config_boots_from_last_good() {
        let dir = temp_dir("recover");
        let path = dir.join("config.yaml");
        // Flow syntax so that cutting the bytes in half is certain to leave
        // the document unparseable — plain lines can survive truncation.
        save_atomic(&path, "key: [first]\n").unwrap();
        save_atomic(&path, "key: [second]\n").unwrap();
        // A crash mid-save: path now holds half its bytes and no longer parses.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(!parses_as_yaml(&std::fs::read_to_string(&path).unwrap()));
        assert_eq!(load_with_recovery(&path).unwrap(), "key: [first]\n");
    }

    #[test]
    fn unparseable_existing_file_is_kept_as_broken() {
        let dir = temp_dir("broken");
        let path = dir.join("config.yaml");
        std::fs::write(&path, "key: [unclosed\n").unwrap();
        save_atomic(&path, "key: fresh\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "key: fresh\n");
        assert_eq!(
            std::fs::read_to_string(sibling(&path, "broken")).unwrap(),
            "key: [unclosed\n"
        );
        // An unparseable file was never worth copying as last-good.
        assert!(!sibling(&path, "last-good").exists());
    }

    #[test]
    fn corrupt_config_without_last_good_is_an_error() {
        let dir = temp_dir("no-last-good");
        let path = dir.join("config.yaml");
        std::fs::write(&path, "key: [unclosed\n").unwrap();
        assert!(load_with_recovery(&path).is_err());
    }
}
