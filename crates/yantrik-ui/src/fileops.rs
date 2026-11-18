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
    use std::os::unix::ffi::OsStrExt;
    let parent = fs::canonicalize(src.parent().ok_or("Cannot trash a filesystem root")?)
        .map_err(|e| error(src, e))?;
    let source = parent.join(src.file_name().ok_or("Cannot trash a filesystem root")?);
    if source.starts_with(root) || root.starts_with(&source) {
        return Err("The Trash storage folder cannot be moved to Trash.".into());
    }
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
    if let Err(e) = rename_no_replace(&source, &root.join("files").join(&id)) {
        let _ = fs::remove_file(info);
        return Err(e);
    }
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
