//! Whether a path exists, answered for a mind that cannot see for itself.
//!
//! A mind that runs as an account of its own (#411) cannot see the person's home. Its own look
//! at the filesystem says "not found" for what is only hidden from it, and it told the person a
//! file it had just saved "was not created". The desktop runs as the person, so it can say which
//! of the two it is, and it has to say it in a way that keeps them apart: `exists` is true, false
//! or "unknown", and a `reason` says why whenever it is not true.
//!
//! Only the person's home is answered for, and never its protected places. Anything else is
//! `unknown`, with a reason that says nothing about whether it is there.

use std::io::ErrorKind;
use std::path::Path;

use serde_json::{json, Value};

use super::{expand, is_protected};

/// The answer for `asked`, with `~` meaning `home`: `exists` true (with `kind`, `size`,
/// `modified`, `changed` (ctime, which cannot be set back), and `real`/`via_link`: where it resolved, and whether a link took it there), false with reason `not_found`, or "unknown" with reason `outside`, `protected`,
/// `not_allowed`, `broken_link`, `hard_link` or `not_a_path`.
pub fn stat(asked: &str, home: &Path) -> Value {
    let Some(path) = expand(asked.trim(), home) else {
        return unknown(asked, "not_a_path");
    };
    if !home.is_absolute() || home == Path::new("/") || !path.starts_with(home) {
        return unknown(asked, "outside");
    }
    let Ok(real_home) = home.canonicalize() else {
        return unknown(asked, "outside");
    };
    if real_home == Path::new("/") {
        return unknown(asked, "outside");
    }
    if is_protected(&path) {
        return unknown(asked, "protected");
    }

    // Up from the path to the deepest part of it that resolves. That part decides where the path
    // goes; whatever is below it does not exist, unless it is a link that exists and leads
    // nowhere, which is not known to be anything.
    let mut probe = path.clone();
    loop {
        match probe.canonicalize() {
            Ok(real) => {
                if !real.starts_with(&real_home) {
                    return unknown(asked, "outside");
                }
                // The part that does not exist yet, put where the rest really is: a protected
                // name split across a link (`c -> ~/.config`, asked as `c/labwc/x`) is only
                // whole once the two are joined.
                let tail = path.strip_prefix(&probe).unwrap_or(Path::new(""));
                if is_protected(&real.join(tail)) {
                    return unknown(asked, "protected");
                }
                if probe == path {
                    return describe(&path, &real, through_a_link(&path, home));
                }
                return not_found(&path);
            }
            Err(e) if e.kind() == ErrorKind::PermissionDenied => return unknown(asked, "not_allowed"),
            Err(_) => {
                if let Ok(meta) = probe.symlink_metadata() {
                    return unknown(asked, if meta.file_type().is_symlink() { "broken_link" } else { "not_allowed" });
                }
            }
        }
        match probe.parent() {
            Some(parent) if parent.starts_with(home) => probe = parent.to_path_buf(),
            _ => return unknown(asked, "outside"),
        }
    }
}

/// What is at `real`, which `path` resolved to inside the home. `real` and `via_link` let a mind
/// that cannot look for itself tell a file the person named from a link to somewhere else in the
/// home: only the resolved path is the file that will be read.
fn describe(path: &Path, real: &Path, via_link: bool) -> Value {
    match std::fs::metadata(real) {
        // A file with a second name is the same bytes as whatever that name is, and the other
        // name may be anywhere on the filesystem, ~/.ssh or /etc included: no link to follow
        // says where. A hard link to /etc/shadow looks like any file in the home.
        Ok(meta) if meta.is_file() && links(&meta) > 1 => unknown(&path.to_string_lossy(), "hard_link"),
        Ok(meta) => {
            let kind = if meta.is_dir() {
                "directory"
            } else if meta.is_file() {
                "file"
            } else {
                "other"
            };
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            json!({
                "path": path.to_string_lossy(),
                "exists": true,
                "kind": kind,
                "size": if meta.is_file() { Some(meta.len()) } else { None },
                "modified": modified,
                // When the file's bytes or metadata last changed (ctime). Unlike `modified`, no
                // one but root can set it back: `touch -d` moves mtime and bumps this. A mind
                // checks it to know a file was not written after the person named it.
                "changed": changed(&meta),
                "real": real.to_string_lossy(),
                "via_link": via_link,
            })
        }
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => not_found(path),
        Err(_) => unknown(&path.to_string_lossy(), "not_allowed"),
    }
}

/// Whether any part of `path` below `home` is a link: the file it names is then somewhere else.
fn through_a_link(path: &Path, home: &Path) -> bool {
    path.ancestors()
        .take_while(|p| p.starts_with(home) && *p != home)
        .any(|p| p.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()))
}

#[cfg(unix)]
fn changed(meta: &std::fs::Metadata) -> Option<i64> {
    Some(std::os::unix::fs::MetadataExt::ctime(meta))
}

#[cfg(not(unix))]
fn changed(_meta: &std::fs::Metadata) -> Option<i64> {
    None
}

#[cfg(unix)]
fn links(meta: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::nlink(meta)
}

#[cfg(not(unix))]
fn links(_meta: &std::fs::Metadata) -> u64 {
    1
}

fn not_found(path: &Path) -> Value {
    json!({ "path": path.to_string_lossy(), "exists": false, "reason": "not_found" })
}

fn unknown(asked: &str, reason: &str) -> Value {
    json!({ "path": asked, "exists": "unknown", "reason": reason })
}
