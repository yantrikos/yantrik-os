//! The one locked, atomic writer for the config file.
//!
//! Two places rewrite config.yaml: the OAuth token refresh (`email::update_config_token`)
//! and `CompanionService::save_config`. Both read the file (or the in-memory config),
//! edit in memory and write it all back; nothing serialised them, so a token refresh
//! landing during a save_config could clobber the fresh token, and a crash mid-write
//! could leave a torn config. Both now go through the lock here, and every write lands
//! through a temp file plus rename so the target is never half-written.

use std::sync::Mutex;

/// Held for the whole read-modify-write so two writers cannot interleave. A panic
/// while the lock is held leaves nothing half-applied (the edit is computed before
/// any write), so recovering the guard instead of propagating the panic is safe —
/// and keeps a failed persist a warning rather than a crash for the caller.
static CONFIG_LOCK: Mutex<()> = Mutex::new(());

/// Read `path`, hand the current text to `edit`, write back what it returns;
/// nothing is written if the closure returns `None`.
pub fn edit_locked(path: &str, edit: impl FnOnce(String) -> Option<String>) -> Result<(), String> {
    let _guard = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let content = std::fs::read_to_string(path).map_err(|e| format!("Read config: {}", e))?;
    if let Some(new_content) = edit(content) {
        write_atomic(path, &new_content)?;
    }
    Ok(())
}

/// Overwrite `path` with `content` without ever being seen mid-write. Takes the same
/// lock as [`edit_locked`] so a full save cannot land inside a token refresh's edit.
pub fn write_locked(path: &str, content: &str) -> Result<(), String> {
    let _guard = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    write_atomic(path, content)
}

/// Write to a temp file next to the target, then rename over it: a rename within one
/// directory is atomic, so readers see either the old file or the new one, never a
/// half-written config.
fn write_atomic(path: &str, content: &str) -> Result<(), String> {
    let tmp = format!("{}.tmp", path);
    std::fs::write(&tmp, content).map_err(|e| format!("Write config: {}", e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Write config: {}", e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    /// A fresh subdirectory of the system temp dir, unique to one test.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yos_config_store_{}_{}",
            tag,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn concurrent_edits_both_survive() {
        let dir = temp_dir("race");
        let path = dir.join("config.yaml");
        std::fs::write(&path, "app: yantrik\n").unwrap();

        let (tx_a, rx_a) = channel();
        let (tx_b, rx_b) = channel();
        let pa = path.clone();
        let pb = path.clone();
        // Both threads exchange a word first, so they enter the writer together, and
        // each edit then holds the read file for a moment before returning its result —
        // an unlocked writer loses one marker every time, a locked one serialises them.
        let a = std::thread::spawn(move || {
            let _ = tx_a.send('x');
            let _ = rx_b.recv_timeout(Duration::from_millis(750));
            edit_locked(&pa.to_string_lossy(), |c| {
                std::thread::sleep(Duration::from_millis(50));
                Some(format!("{}\nx: yes", c))
            })
            .unwrap();
        });
        let b = std::thread::spawn(move || {
            let _ = tx_b.send('y');
            let _ = rx_a.recv_timeout(Duration::from_millis(750));
            edit_locked(&pb.to_string_lossy(), |c| {
                std::thread::sleep(Duration::from_millis(50));
                Some(format!("{}\ny: yes", c))
            })
            .unwrap();
        });
        a.join().unwrap();
        b.join().unwrap();

        let final_text = std::fs::read_to_string(&path).unwrap();
        assert!(final_text.contains("x: yes"), "marker x lost: {}", final_text);
        assert!(final_text.contains("y: yes"), "marker y lost: {}", final_text);
    }

    #[test]
    fn edit_returning_none_leaves_file_untouched() {
        let dir = temp_dir("noop");
        let path = dir.join("config.yaml");
        std::fs::write(&path, "app: yantrik\nuntouched: 42\n").unwrap();

        edit_locked(&path.to_string_lossy(), |_| None).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "app: yantrik\nuntouched: 42\n");
    }

    #[test]
    fn token_refresh_rewrites_only_its_own_account() {
        let dir = temp_dir("token");
        let path = dir.join("config.yaml");
        // The parser ends an account block only at an unindented key line, so any
        // `- email:` list item — even at column 0 — has always leaked the rewrite to
        // every later account. That is a pre-existing defect, left as it is; these
        // accounts use plain keys, the shape the parser was written for.
        std::fs::write(
            &path,
            r#"account_one:
  email: one@example.com
  oauth_access_token: "tok-one"
  oauth_token_expiry: 100.0
account_two:
  email: two@example.com
  oauth_access_token: "tok-two"
  oauth_token_expiry: 200.0
"#,
        )
        .unwrap();

        crate::email::update_config_token(
            &path.to_string_lossy(),
            "one@example.com",
            "new-tok",
            999.5,
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("oauth_access_token: \"new-tok\""), "new token missing: {}", text);
        assert!(text.contains("oauth_token_expiry: 999.5"), "new expiry missing: {}", text);
        assert!(text.contains("oauth_access_token: \"tok-two\""), "other account's token changed: {}", text);
        assert!(text.contains("oauth_token_expiry: 200.0"), "other account's expiry changed: {}", text);
        assert!(!text.contains("tok-one"), "old token left behind: {}", text);
    }

    #[test]
    fn write_locked_replaces_and_survives_concurrent_edit() {
        let dir = temp_dir("save");
        let path = dir.join("config.yaml");
        std::fs::write(&path, "old: 1\n").unwrap();
        write_locked(&path.to_string_lossy(), "new: 2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new: 2\n");
    }
}
