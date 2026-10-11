//! The checks behind `private_dir`: making a directory chain we can trust, and creating a file in
//! it without being steered through a link.

use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
pub(super) fn current_uid() -> u32 {
    // SAFETY: geteuid cannot fail and touches no memory.
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
pub(super) fn current_uid() -> u32 {
    0
}

/// Make `base/rel` and every directory between them, checking each on the way down; returns the
/// canonical path of the result.
///
/// The leaf being ours and 0700 is not enough on its own: whoever can write to a directory above
/// it can rename it away and put a link in its place after we have looked. So `base` is judged
/// before anything is made under it, and every directory from there down must pass [`trusted`].
/// That is also what turns away a `HOME` or runtime dir that is really `/tmp`, rather than
/// quietly building our private dir inside it.
///
/// Links are followed on the way down and judged by where they land — `/home` pointing at
/// `/usr/home`, or `~/.cache` living on another disk, are ordinary machines — and each step starts
/// from the canonical result of the one before, so the check and the path handed back agree.
/// Only the leaf may not be a link: it is ours to have made.
pub(super) fn prepare(base: &Path, rel: &Path) -> io::Result<PathBuf> {
    let uid = current_uid();
    let mut cur = std::fs::canonicalize(base)?;
    trusted(&cur, &std::fs::metadata(&cur)?, uid)?;
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for part in rel.components() {
        match part {
            Component::Normal(p) => parts.push(p),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not a plain relative path: {}", rel.display()))),
        }
    }
    let leaf = parts.pop().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty path"))?;
    for part in parts {
        let next = cur.join(part);
        if std::fs::symlink_metadata(&next).is_err() {
            make_dir_0700(&next)?;
        }
        cur = std::fs::canonicalize(&next)?;
        trusted(&cur, &std::fs::metadata(&cur)?, uid)?;
    }
    let dir = cur.join(leaf);
    ensure_private_as(&dir, uid)?;
    Ok(dir)
}

/// Whether `dir` is still what [`prepare`] made of it: a 0700 directory of ours, not a link.
/// Cheap enough to run on every use of a remembered answer.
#[cfg(unix)]
pub(super) fn still_private(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(dir).is_ok_and(|m| {
        m.file_type().is_dir() && m.uid() == current_uid() && m.mode() & 0o777 == 0o700
    })
}

#[cfg(not(unix))]
pub(super) fn still_private(dir: &Path) -> bool {
    dir.is_dir()
}

/// A directory on the way down to ours: a real directory, owned by us or root, and writable by
/// nobody else — anyone who can write to it can rename what is inside it.
#[cfg(unix)]
pub(super) fn trusted(p: &Path, meta: &std::fs::Metadata, uid: u32) -> io::Result<()> {
    trusted_as(p, meta, uid, super::upg::private_group(uid))
}

/// [`trusted`], with our user-private group (if we have one) passed in so a test can stand in
/// for one.
///
/// Group-writable is allowed only in that group: on a umask-002 system with user-private groups,
/// `~/.cache` and `~/.local/state` are 0775 and the group holds nobody but us. A group-writable
/// directory of any other group is refused — including our own primary group when it is a shared
/// one like `users` — and world-writable always is.
#[cfg(unix)]
pub(super) fn trusted_as(p: &Path, meta: &std::fs::Metadata, uid: u32, private_gid: Option<u32>) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let refuse = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {why}", p.display()));
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(refuse("is not a plain directory".into()));
    }
    if meta.uid() != uid && meta.uid() != 0 {
        return Err(refuse(format!("owned by uid {}, neither us ({uid}) nor root", meta.uid())));
    }
    if meta.mode() & 0o002 != 0 {
        return Err(refuse("is writable by every account".into()));
    }
    if meta.mode() & 0o020 != 0 && Some(meta.gid()) != private_gid {
        return Err(refuse(format!("is writable by group {}, which is not a group of ours alone", meta.gid())));
    }
    Ok(())
}

/// Windows dev builds: the profile's own directories are per user already.
#[cfg(not(unix))]
pub(super) fn trusted(_p: &Path, _meta: &std::fs::Metadata, _uid: u32) -> io::Result<()> {
    Ok(())
}

/// Make `dir` if it is missing, then refuse it unless it is a directory we own; tighten it to
/// 0700 if it is not already. The parent must exist.
#[cfg(unix)]
pub(super) fn ensure_private_as(dir: &Path, uid: u32) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    // Only `mkdir` when it is missing. On an existing directory `mkdir` is still a request to
    // make one, and a Landlock ruleset without MAKE_DIR answers that with EACCES before the
    // kernel gets as far as EEXIST — the trap `socket_dir` in yantrik-ipc-transport fell into.
    if std::fs::symlink_metadata(dir).is_err() {
        make_dir_0700(dir)?;
    }
    // `symlink_metadata`, not `metadata`: a link to a directory someone else controls must be
    // refused as a link, not followed and judged by where it points.
    let meta = std::fs::symlink_metadata(dir)?;
    let refuse = |why: &str| io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {why}", dir.display()));
    if meta.file_type().is_symlink() {
        return Err(refuse("is a symlink"));
    }
    if !meta.is_dir() {
        return Err(refuse("is not a directory"));
    }
    if meta.uid() != uid {
        return Err(refuse(&format!("owned by uid {}, not {uid}", meta.uid())));
    }
    // Ours but loose (made under an odd umask, or by an older build): tighten rather than refuse.
    // Skipped when already 0700, so a sandboxed caller is never asked for a chmod it cannot do.
    if meta.mode() & 0o777 != 0o700 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn ensure_private_as(dir: &Path, _uid: u32) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn make_dir_0700(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(dir) {
        // Somebody made it between the look and the mkdir; the caller's checks decide.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        other => other,
    }
}

/// Create (or empty) a file for writing, 0600, without being steered somewhere else.
///
/// The directories are ours, but the names in them are not only ours to make: the file tools
/// may write into scratch, so a name there can already be a link to a file the person cares
/// about, or a second hard link to one, or a FIFO that would block the write for ever. So:
/// `O_NOFOLLOW` refuses a link at the name; `O_NONBLOCK` turns a FIFO with no reader into an
/// error instead of a hang; and the file is only emptied after checking it is a plain file with
/// no other name — `O_TRUNC` would have emptied a hard-linked `authorized_keys` before any check
/// could run.
#[cfg(unix)]
pub fn create_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.nlink() != 1 || meta.uid() != current_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: not a plain file of ours with a single name; refusing to overwrite it", path.display()),
        ));
    }
    file.set_len(0)?;
    if meta.mode() & 0o777 != 0o600 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

#[cfg(not(unix))]
pub fn create_private_file(path: &Path) -> io::Result<File> {
    File::create(path)
}

/// Open a file of ours for reading, without being steered somewhere else.
///
/// The reading side of [`create_private_file`]. The file tools can put anything at a scratch
/// name — extracting an archive is enough — so `yantrik-scrollback.txt -> ~/.ssh/id_ed25519` would
/// have had the terminal tools hand the key to the model as "the terminal". `O_NOFOLLOW` refuses
/// the link, `O_NONBLOCK` keeps a FIFO from hanging the read, and the open file itself must be a
/// regular file owned by us with no other name — a hard link to the key is the same key, and
/// reads should refuse what writes refuse. A caller that judges the file (its age, say) should ask the
/// returned `File` for its metadata, so it judges the file it reads and not the name.
#[cfg(unix)]
pub fn open_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.nlink() != 1 || meta.uid() != current_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: not a plain file of ours with a single name; refusing to read it", path.display()),
        ));
    }
    Ok(file)
}

#[cfg(not(unix))]
pub fn open_private_file(path: &Path) -> io::Result<File> {
    File::open(path)
}
