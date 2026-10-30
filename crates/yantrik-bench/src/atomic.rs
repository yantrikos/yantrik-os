//! Writing a file so that a reader sees the old one or the new one and never half of either.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Write `bytes` to `path` through a temporary file beside it: created with `create_new` (a link
/// planted at that name is never followed, and a name already taken is not ours to remove),
/// private to the person (0600), synced, then renamed over `path`. A link at `path` is replaced,
/// not written through.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = parent.join(format!(".{name}.tmp{}", std::process::id()));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    let done = f.write_all(bytes).and_then(|()| f.sync_all()).and_then(|()| std::fs::rename(&tmp, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_replaces_the_file_and_leaves_nothing_beside_it() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("f.json");
        write(&p, b"one").unwrap();
        write(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
}
