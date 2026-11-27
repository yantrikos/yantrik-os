//! Discard: a prepared folder is moved aside, never deleted (#1416, round 2).
//!
//! `~/Documents/Prepared/<day>-<slug>/` goes to `~/Documents/Prepared/.discarded/<same name>`
//! (`-2`, `-3` … when that name is taken). The person can still take it back from Files; nothing
//! here removes a byte. Only a folder directly inside `Prepared/`, that is not a link and not the
//! `.discarded` folder itself, moves.

use std::path::{Path, PathBuf};

/// The folder prepared work goes to.
pub fn prepared_root(documents: &Path) -> PathBuf {
    documents.join("Prepared")
}

/// Where a discarded folder goes.
pub const DISCARDED: &str = ".discarded";

/// Move `folder` to `Prepared/.discarded/`, and say where it went.
pub fn discard(documents: &Path, folder: &Path) -> Result<PathBuf, String> {
    let root = prepared_root(documents);
    let name = folder.file_name().and_then(|n| n.to_str()).filter(|n| !n.starts_with('.') && !n.is_empty()).ok_or("that is not a prepared folder")?;
    if folder.parent() != Some(root.as_path()) || folder.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("that folder is not inside Prepared".into());
    }
    let meta = std::fs::symlink_metadata(folder).map_err(|_| "that folder is not there any more".to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("that is not a folder of its own".into());
    }
    let aside = root.join(DISCARDED);
    std::fs::create_dir_all(&aside).map_err(|e| format!("could not make the .discarded folder: {e}"))?;
    let mut to = aside.join(name);
    let mut n = 2;
    while to.exists() {
        to = aside.join(format!("{name}-{n}"));
        n += 1;
    }
    std::fs::rename(folder, &to).map_err(|e| format!("could not move it: {e}"))?;
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("yos-discard-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn discard_moves_the_folder_and_keeps_every_file_and_never_deletes() {
        let docs = tmp();
        let f = prepared_root(&docs).join("2026-11-26-demo");
        std::fs::create_dir_all(&f).unwrap();
        std::fs::write(f.join("summary.md"), "keep me").unwrap();
        let to = discard(&docs, &f).unwrap();
        assert_eq!(to, prepared_root(&docs).join(DISCARDED).join("2026-11-26-demo"));
        assert!(!f.exists());
        assert_eq!(std::fs::read_to_string(to.join("summary.md")).unwrap(), "keep me");
        // The same name again: the first is not replaced.
        std::fs::create_dir_all(&f).unwrap();
        std::fs::write(f.join("summary.md"), "second").unwrap();
        let again = discard(&docs, &f).unwrap();
        assert_eq!(again.file_name().unwrap(), "2026-11-26-demo-2");
        assert_eq!(std::fs::read_to_string(to.join("summary.md")).unwrap(), "keep me");
        std::fs::remove_dir_all(&docs).ok();
    }

    #[test]
    fn discard_refuses_what_is_not_a_prepared_folder() {
        let docs = tmp();
        let root = prepared_root(&docs);
        std::fs::create_dir_all(root.join(DISCARDED)).unwrap();
        std::fs::create_dir_all(docs.join("Mine")).unwrap();
        std::fs::create_dir_all(root.join("a").join("deeper")).unwrap();
        std::os::unix::fs::symlink(docs.join("Mine"), root.join("link")).unwrap();
        for bad in [docs.join("Mine"), root.join(DISCARDED), root.join("a").join("deeper"), root.join("link"), root.join("gone"), root.join("..").join("Mine"), root.clone()] {
            assert!(discard(&docs, &bad).is_err(), "{}", bad.display());
        }
        assert!(docs.join("Mine").is_dir() && root.join("a").join("deeper").is_dir());
        std::fs::remove_dir_all(&docs).ok();
    }
}
