//! Filesystem operations with no replacement, bounded copy buffers and recoverable trash.
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub type Result<T> = std::result::Result<T, String>;
pub fn name(value: &str) -> Result<&str> {
    if value.is_empty() || matches!(value, "." | "..") || value.contains(['/', '\0']) {
        Err("Use a name without /, NUL, . or ...".into())
    } else {
        Ok(value)
    }
}
fn error(path: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e}", path.display())
}
pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
pub fn rename_no_replace(src: &Path, dst: &Path) -> Result<()> {
    // The one renameat2 call lives in the bench crate, where the archive uses it too.
    yantrik_bench::rename::rename_no_replace(src, dst).map_err(|e| {
        if e.raw_os_error() == Some(libc::EXDEV) {
            "Cross-filesystem moves are not supported. The source was left in place.".to_string()
        } else {
            error(dst, e)
        }
    })
}
/// A folder held open across the check of one of its entries and the act on it (#1078).
///
/// The entry is resolved *inside* the held fd — `fstatat` with no `AT_SYMLINK_FOLLOW`, so a leaf
/// that is a link is refused, not followed — and named by `(fd, name)` for the act, never walked
/// from the root again. Between the verdict and the act, `still_holds` compares the fd and leaf
/// with what was resolved first; a leaf replaced in that window — for a link, or for another
/// entry, or gone — is refused before anything moves.
#[derive(Debug)]
pub struct Held {
    dir: PathBuf,
    fd: i32,
    leaf: std::ffi::CString,
    dev: u64,
    ino: u64,
}

/// `stat` of one entry inside a held directory: the entry itself, a link told as one.
fn stat_at(dir_fd: i32, leaf: &std::ffi::CString) -> std::io::Result<libc::stat> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: a zeroed stat and a NUL-terminated name that outlives the call.
    if unsafe { libc::fstatat(dir_fd, leaf.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(st)
}

/// A path as the C library takes it.
fn c_name(name: &str) -> std::io::Result<std::ffi::CString> {
    std::ffi::CString::new(name.as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput.into())
}

impl Held {
    /// Open `dir` without following a link on its last part, and resolve `leaf` inside it. A leaf
    /// that is a symbolic link is refused: an entry a Mind may reach through a link is not the one
    /// the verdict was about.
    pub fn open(dir: &Path, leaf: &str) -> Result<Self> {
        name(leaf)?;
        use std::os::unix::ffi::OsStrExt;
        let c_dir = std::ffi::CString::new(dir.as_os_str().as_bytes())
            .map_err(|_| error(dir, "is not a path the system can name"))?;
        // SAFETY: a NUL-terminated path; the fd this returns is closed once, in Drop.
        let fd = unsafe {
            libc::open(
                c_dir.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(error(dir, std::io::Error::last_os_error()));
        }
        let leaf_c = c_name(leaf).map_err(|e| error(dir, e))?;
        let st = stat_at(fd, &leaf_c).map_err(|e| error(&dir.join(leaf), e))?;
        if st.st_mode & libc::S_IFMT == libc::S_IFLNK {
            // SAFETY: an fd this function opened.
            unsafe { libc::close(fd) };
            return Err(format!(
                "{} is a symbolic link; it is refused rather than followed.",
                dir.join(leaf).display()
            ));
        }
        Ok(Self { dir: dir.to_path_buf(), fd, leaf: leaf_c, dev: st.st_dev, ino: st.st_ino })
    }

    /// The entry's name, as the system holds it.
    pub fn leaf(&self) -> std::path::PathBuf {
        self.dir.join(c_path(&self.leaf))
    }

    /// Whether `dir` is the folder this entry would be moved into, which needs no second fd.
    fn same_dir(&self, dir: &Path) -> bool {
        self.dir == dir
    }

    /// Nothing about the entry has changed since it was resolved: the same name, in the folder
    /// held open, still names the same file. Call it as the last word before the act.
    pub fn still_holds(&self) -> Result<()> {
        let st = stat_at(self.fd, &self.leaf).map_err(|e| error(&self.leaf(), e))?;
        if st.st_dev != self.dev || st.st_ino != self.ino {
            return Err(format!(
                "{} changed while it was being checked; nothing was moved.",
                self.leaf().display()
            ));
        }
        Ok(())
    }

    /// Rename the held entry to `new` in the same folder, or fail if a name is taken. The entry
    /// renamed is the entry resolved above, and the same directory fd names both sides.
    pub fn rename_in(&self, new: &str) -> Result<()> {
        let to = c_name(new).map_err(|e| error(&self.dir, e))?;
        self.move_to(&self.dir, &to)
    }

    /// Move the held entry into `(dir_to, to)` without replacing what is there, both sides named
    /// by a directory fd. The entry is looked at once more inside, immediately before the rename:
    /// a caller cannot put the verdict and the act on either side of a window.
    fn move_to(&self, dir_to: &Path, to: &std::ffi::CString) -> Result<()> {
        self.still_holds()?;
        let (to_fd, held_the_folder) = self.target(dir_to)?;
        // SAFETY: NUL-terminated names; both fds are open, and the destination fd when it is not
        // the held one is closed below.
        let rc = unsafe {
            libc::renameat2(
                self.fd,
                self.leaf.as_ptr(),
                to_fd,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        // Read the failure before anything else can set errno again.
        let e = (rc != 0).then(std::io::Error::last_os_error);
        let dst = dir_to.join(c_path(to));
        let outcome = match e {
            None => Ok(()),
            // A file system that cannot do an atomic no-replace rename (some network and FUSE
            // ones): the entry moved is still the one held open, so what is left to race for is a
            // name appearing at the destination, which is checked for here.
            Some(e) if no_replace_unavailable(&e) => {
                tracing::debug!("renameat2(RENAME_NOREPLACE) is not supported here; checking {dst:?} before the move");
                if fs::symlink_metadata(&dst).is_ok() {
                    Err(rename_err(&dst, std::io::ErrorKind::AlreadyExists.into()))
                } else {
                    self.rename_at(to_fd, to).map_err(|e| rename_err(&dst, e))
                }
            }
            Some(e) => Err(rename_err(&dst, e)),
        };
        if !held_the_folder {
            // SAFETY: the fd `target` opened.
            unsafe { libc::close(to_fd) };
        }
        outcome
    }

    /// A plain `renameat` inside the held folder, for a file system with no no-replace rename.
    fn rename_at(&self, to_fd: i32, to: &std::ffi::CString) -> std::io::Result<()> {
        // SAFETY: NUL-terminated names; `to_fd` is open for the call.
        let rc = unsafe { libc::renameat(self.fd, self.leaf.as_ptr(), to_fd, to.as_ptr()) };
        if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }

    /// A directory fd for the folder the entry moves into — the held one when it is the same
    /// folder, so a rename inside a folder names both sides with a single fd.
    fn target(&self, dir_to: &Path) -> Result<(i32, bool)> {
        if self.same_dir(dir_to) {
            return Ok((self.fd, true));
        }
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir_to.as_os_str().as_bytes())
            .map_err(|_| error(dir_to, "is not a path the system can name"))?;
        // SAFETY: a NUL-terminated path; the fd is closed by the caller of this.
        let fd = unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(error(dir_to, std::io::Error::last_os_error()));
        }
        Ok((fd, false))
    }
}

/// The name the system holds as a path, for the words of an answer.
fn c_path(name: &std::ffi::CString) -> std::path::PathBuf {
    std::path::PathBuf::from(String::from_utf8_lossy(name.to_bytes()).as_ref())
}

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: the fd `open` returned, closed once.
        unsafe { libc::close(self.fd) };
    }
}

/// Whether the failure is the file system not offering an atomic no-replace rename, rather than
/// anything about the two entries — the same three codes the bench's `rename_no_replace` treats so.
fn no_replace_unavailable(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP))
}

/// Why a rename did not happen, in the words the Files operations have always used.
fn rename_err(path: &Path, e: std::io::Error) -> String {
    if e.raw_os_error() == Some(libc::EXDEV) {
        "Cross-filesystem moves are not supported. The source was left in place.".to_string()
    } else {
        error(path, e)
    }
}

pub fn create_file(dir: &Path, value: &str) -> Result<()> {
    let path = dir.join(name(value)?);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(|_| ())
        .map_err(|e| error(&path, e))
}
pub fn create_folder(dir: &Path, value: &str) -> Result<()> {
    let path = dir.join(name(value)?);
    fs::create_dir(&path).map_err(|e| error(&path, e))
}
fn unique() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err("Canceled. Completed items are kept; the current copy was removed.".into())
    } else {
        Ok(())
    }
}
fn remove_owned(path: &Path) -> std::io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
/// Copy to an owned staging name, then atomically publish without replacing anything.
/// Symbolic links are copied as links, never traversed. Directory cycles cannot recurse.
pub fn transfer(
    src: &Path,
    destination: &Path,
    cut: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    cancelled(cancel)?;
    let leaf = src.file_name().ok_or("Cannot transfer a filesystem root")?;
    let parent = fs::canonicalize(destination).map_err(|e| error(destination, e))?;
    let dst = parent.join(leaf);
    let meta = fs::symlink_metadata(src).map_err(|e| error(src, e))?;
    let source_parent =
        fs::canonicalize(src.parent().ok_or("Source has no parent")?).map_err(|e| error(src, e))?;
    if source_parent.join(leaf) == dst {
        return Err("Source and destination are the same item.".into());
    }
    if exists(&dst) {
        return Err(format!(
            "{} already exists. Rename it or choose another folder; nothing was replaced.",
            dst.display()
        ));
    }
    if meta.is_dir() {
        let original = fs::canonicalize(src).map_err(|e| error(src, e))?;
        if parent.starts_with(&original) {
            return Err("A folder cannot be placed inside itself.".into());
        }
    }
    if cut {
        return rename_no_replace(src, &dst);
    }
    let stage = parent.join(format!(".yantrik-copy-{}", unique()));
    let result = copy_node(src, &stage, cancel, progress, 0).and_then(|_| {
        cancelled(cancel)?;
        rename_no_replace(&stage, &dst)
    });
    if result.is_err() && exists(&stage) {
        if let Err(e) = remove_owned(&stage) {
            return Err(format!(
                "{}; temporary copy could not be removed: {}",
                result.unwrap_err(),
                error(&stage, e)
            ));
        }
    }
    result
}
fn copy_node(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    depth: usize,
) -> Result<()> {
    cancelled(cancel)?;
    if depth > 128 {
        return Err("Folder nesting exceeds 128 levels.".into());
    }
    let meta = fs::symlink_metadata(src).map_err(|e| error(src, e))?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(src).map_err(|e| error(src, e))?;
        return std::os::unix::fs::symlink(target, dst).map_err(|e| error(dst, e));
    }
    if meta.is_dir() {
        fs::create_dir(dst).map_err(|e| error(dst, e))?;
        for entry in fs::read_dir(src).map_err(|e| error(src, e))? {
            let entry = entry.map_err(|e| error(src, e))?;
            copy_node(
                &entry.path(),
                &dst.join(entry.file_name()),
                cancel,
                progress,
                depth + 1,
            )?;
        }
        fs::set_permissions(dst, meta.permissions()).map_err(|e| error(dst, e))?;
    } else if meta.is_file() {
        // O_NOFOLLOW prevents a link substituted after inspection from being followed.
        use std::os::unix::fs::OpenOptionsExt;
        let mut input = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(src)
            .map_err(|e| error(src, e))?;
        if !input.metadata().map_err(|e| error(src, e))?.is_file() {
            return Err("Source changed while copying.".into());
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dst)
            .map_err(|e| error(dst, e))?;
        let mut buffer = [0u8; 65536];
        loop {
            cancelled(cancel)?;
            let n = input.read(&mut buffer).map_err(|e| error(src, e))?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n]).map_err(|e| error(dst, e))?;
            progress(n as u64);
        }
        output.sync_all().map_err(|e| error(dst, e))?;
        fs::set_permissions(dst, meta.permissions()).map_err(|e| error(dst, e))?;
    } else {
        return Err(format!(
            "{} is a device, socket or pipe; it cannot be copied.",
            src.display()
        ));
    }
    Ok(())
}
#[derive(Clone)]
pub struct TrashItem {
    pub id: String,
    pub name: String,
    pub original: PathBuf,
    pub stored: PathBuf,
}
pub fn trash_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("yantrik/trash-v2")
}
pub fn trash(src: &Path, root: &Path) -> Result<String> {
    let parent = fs::canonicalize(src.parent().ok_or("Cannot trash a filesystem root")?)
        .map_err(|e| error(src, e))?;
    let source = parent.join(src.file_name().ok_or("Cannot trash a filesystem root")?);
    trash_guard(&source, root)?;
    let id = trash_leave(root, &source)?;
    if let Err(e) = rename_no_replace(&source, &root.join("files").join(&id)) {
        let _ = fs::remove_file(root.join("info").join(&id));
        return Err(e);
    }
    Ok(id)
}
/// Move an entry held open in its folder to the Trash: the same storage and the same metadata as
/// [`trash`], and the same entry throughout — the folder is not walked again, and the entry is
/// looked at once more inside its folder immediately before it moves (#1078).
pub fn trash_held(entry: &Held, root: &Path) -> Result<String> {
    let source = entry.leaf();
    trash_guard(&source, root)?;
    let id = trash_leave(root, &source)?;
    let stored = c_name(&id).map_err(|e| error(root, e))?;
    if let Err(e) = entry.move_to(&root.join("files"), &stored) {
        let _ = fs::remove_file(root.join("info").join(&id));
        return Err(e);
    }
    Ok(id)
}
/// Neither side of a move to the Trash may be the Trash's own storage.
fn trash_guard(source: &Path, root: &Path) -> Result<()> {
    if source.starts_with(root) || root.starts_with(source) {
        return Err("The Trash storage folder cannot be moved to Trash.".into());
    }
    Ok(())
}
/// The Trash's folders and this move's metadata, ready for the file to arrive under `id`. The
/// metadata says where the entry was, so that Undo puts it back.
fn trash_leave(root: &Path, source: &Path) -> Result<String> {
    use std::os::unix::ffi::OsStrExt;
    fs::create_dir_all(root.join("files")).map_err(|e| error(root, e))?;
    fs::create_dir_all(root.join("info")).map_err(|e| error(root, e))?;
    let id = unique();
    let info = root.join("info").join(&id);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&info)
        .map_err(|e| error(&info, e))?;
    let data = serde_json::to_vec(&source.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&data)
        .and_then(|_| f.sync_all())
        .map_err(|e| error(&info, e))?;
    Ok(id)
}
pub fn trash_items(root: &Path) -> Result<Vec<TrashItem>> {
    use std::os::unix::ffi::OsStringExt;
    let folder = root.join("files");
    if !folder.exists() {
        return Ok(vec![]);
    }
    let mut result = vec![];
    for e in fs::read_dir(&folder).map_err(|e| error(&folder, e))? {
        let e = e.map_err(|e| error(&folder, e))?;
        let id = e
            .file_name()
            .into_string()
            .map_err(|_| "Invalid Trash identifier")?;
        let data =
            fs::read(root.join("info").join(&id)).map_err(|e| error(&e_path(root, &id), e))?;
        let bytes: Vec<u8> = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        let original = PathBuf::from(std::ffi::OsString::from_vec(bytes));
        if !original.is_absolute() {
            return Err("Trash metadata has an invalid original path.".into());
        }
        result.push(TrashItem {
            id,
            name: original
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            original,
            stored: e.path(),
        });
    }
    result.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(result)
}
fn e_path(root: &Path, id: &str) -> PathBuf {
    root.join("info").join(id)
}
pub fn restore(item: &TrashItem, root: &Path) -> Result<()> {
    name(&item.id)?;
    rename_no_replace(&root.join("files").join(&item.id), &item.original)?;
    fs::remove_file(e_path(root, &item.id))
        .map_err(|e| format!("Restored, but could not remove Trash metadata: {e}"))
}
pub fn empty_trash(root: &Path, cancel: &AtomicBool) -> Result<()> {
    for item in trash_items(root)? {
        cancelled(cancel)?;
        remove_owned(&item.stored).map_err(|e| error(&item.stored, e))?;
        fs::remove_file(e_path(root, &item.id)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A folder under /tmp, removed when the test ends.
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temp(what: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("yantrik1078-{what}-{}", unique()));
        fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    /// An entry in its folder, or the reason it could not be checked.
    fn hold(dir: &Path, leaf: &str) -> Held {
        match Held::open(dir, leaf) {
            Ok(entry) => entry,
            Err(e) => panic!("could not check {}/{}: {e}", dir.display(), leaf),
        }
    }

    /// What a folder holds, or nothing where there is no folder.
    fn count(dir: &Path) -> usize {
        fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
    }

    /// An entry replaced by a link to outside the home partway through is refused, and stays
    /// where it was: what moves is the entry the verdict was about, never what a link planted in
    /// the window points at (#1078).
    #[test]
    fn a_leaf_swapped_for_a_link_partway_through_is_refused() {
        for swap in ["trash", "rename"] {
            let home = temp("home");
            let out = temp("out");
            let docs = home.0.join("docs");
            fs::create_dir(&docs).unwrap();
            let victim = out.0.join("keep");
            fs::write(&victim, b"not yours").unwrap();
            fs::create_dir(docs.join("notes")).unwrap();

            // The entry is resolved in its folder; only then is it swapped for a link out.
            let entry = hold(&docs, "notes");
            fs::remove_dir(docs.join("notes")).unwrap();
            symlink(&victim, docs.join("notes")).unwrap();

            let root = home.0.join("trash");
            let e = match swap {
                "trash" => trash_held(&entry, &root).unwrap_err(),
                _ => entry.rename_in("other").unwrap_err(),
            };
            assert!(e.contains("changed"), "{swap} moved a leaf swapped under it: {e}");
            assert_eq!(fs::read(&victim).unwrap(), b"not yours", "{swap} reached outside the home");
            assert!(
                fs::symlink_metadata(docs.join("notes")).unwrap().file_type().is_symlink(),
                "{swap} moved the link out of the folder"
            );
            assert_eq!(count(&root.join("files")), 0, "{swap} left something in the Trash");
        }
    }

    /// An entry that is already a link is refused where it is resolved, so it is never what a
    /// verdict is given about, let alone what moves.
    #[test]
    fn a_leaf_that_is_a_link_is_refused_as_it_is_resolved() {
        let home = temp("home");
        let out = temp("out");
        let docs = home.0.join("docs");
        fs::create_dir(&docs).unwrap();
        let victim = out.0.join("keep");
        fs::write(&victim, b"not yours").unwrap();
        symlink(&victim, docs.join("notes")).unwrap();

        let e = Held::open(&docs, "notes").unwrap_err();
        assert!(e.contains("symbolic link"), "a link was accepted as an entry: {e}");
        assert_eq!(fs::read(&victim).unwrap(), b"not yours");
    }

    /// The ordinary case still works: a name not taken moves, one already taken is refused
    /// without replacing anything, and the Trash keeps its layout, Undo's record included.
    #[test]
    fn an_ordinary_entry_still_renames_and_reaches_the_trash() {
        let home = temp("home");
        let docs = home.0.join("docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("notes"), b"mine").unwrap();

        hold(&docs, "notes").rename_in("kept").unwrap();
        assert_eq!(fs::read(docs.join("kept")).unwrap(), b"mine");

        fs::write(docs.join("other"), b"different").unwrap();
        assert!(hold(&docs, "kept").rename_in("other").is_err());
        assert_eq!(fs::read(docs.join("other")).unwrap(), b"different");

        let root = home.0.join("trash");
        let id = trash_held(&hold(&docs, "kept"), &root).unwrap();
        assert_eq!(fs::read(root.join("files").join(&id)).unwrap(), b"mine");
        assert!(!docs.join("kept").exists());
        let items = match trash_items(&root) {
            Ok(items) => items,
            Err(e) => panic!("the Trash lost its record: {e}"),
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].original, docs.join("kept"));
    }
}
