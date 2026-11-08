//! The carded copy out of a workshop (#1120, #1124, H2, M3 and round 5, findings 2, 5 and 6).
//!
//! Plain bytes, and nothing that a program reads as configuration or runs:
//!
//! - **regular files by content**: no link, pipe, socket or device, no mode bits (0644), no
//!   extended attributes, never over a name that is there;
//! - **an allow list** ([`carried`]): a file is carried only when its name is not hidden (no
//!   leading dot, at any depth) and it is a kind of file that is read and not run: a document,
//!   a picture, a sound or a video, text, data, source. Everything else (`.mise.toml`,
//!   `.tool-versions`, `.devcontainer`, `.idea`, `.direnv`, a `.desktop` file, a script) is left
//!   out and counted in `dropped`. A deny list names what someone thought of; this names what is
//!   taken;
//! - **no repository**, by what a thing is and not what it is called. A folder is refused, and
//!   with it the whole export, when it is named `.git`, `.hg`, `.jj`, `.svn` or `.bzr` (any case),
//!   when it holds a `gitdir` or `commondir` file, or when it has a git directory's own shape
//!   (`HEAD` a file, `objects` and `refs` folders): `git` takes such a folder for a repository
//!   wherever it sits, and its `config` can name a program to run.
//!
//! # Order of work (finding 5)
//!
//! 1. The destination is opened **by descriptor**: the home, then each name below it with
//!    `openat(O_DIRECTORY|O_NOFOLLOW)`. A link anywhere on the way is refused, and a link swapped
//!    in afterwards is never reached, because nothing is looked up by path again.
//! 2. The *source* is judged first, before one byte is written ([`preflight`]): a repository by
//!    name or shape refuses the export with nothing made.
//! 3. The copy is built under a fresh hidden name inside the destination descriptor, and is judged
//!    again on the copy, which a running job cannot touch.
//! 4. Only then is it `renameat2(RENAME_NOREPLACE)`d to its name, after the destination is shown to
//!    still be where it was approved. On any failure the half copy is removed with `unlinkat`
//!    relative to the descriptor, never by path (finding 2).
//!
//! So `~/Projects/<name>` never exists until it is whole and clean, and there is no window in
//! which a companion's `git status` or a prompt can find a repository in it.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use crate::workshop_out::{below, kind_of, open_below, open_workshop, own};

/// Entries and bytes one export may carry.
const MAX_ENTRIES: usize = 20_000;
const MAX_BYTES: u64 = 8 << 30;
const MAX_DEPTH: usize = 32;

/// Version control state: a folder or file by one of these names makes the whole export refused.
const REPOSITORY_NAMES: [&str; 5] = [".git", ".hg", ".jj", ".svn", ".bzr"];

/// Files carried by their (lowercased) extension: read, never run.
const CARRIED_EXTENSIONS: &[&str] = &[
    "txt", "md", "rst", "pdf", "csv", "tsv", "json", "xml", "css", "png", "jpg", "jpeg", "gif", "webp", "bmp", "mp4", "m4v", "mkv",
    "webm", "avi", "mov", "mp3", "m4a", "ogg", "opus", "flac", "wav", "srt", "vtt", "rs", "py", "js", "ts", "c", "h", "cpp", "go", "java",
    "toml", "yaml", "yml", "lock", "log",
];

/// Files with no extension that are carried by name.
const CARRIED_NAMES: &[&str] = &["readme", "license", "licence", "notice", "copying", "authors", "changelog", "contributing"];

/// Is the file `name` one an export carries? Not hidden, and either a known kind or a known name.
/// A leading dot is refused at every depth; `mise.toml`, `Makefile` and the like are not on the list.
pub fn carried(name: &[u8]) -> bool {
    let n = lower(name);
    if n.starts_with('.') || n == "mise.toml" {
        return false;
    }
    match n.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => CARRIED_EXTENSIONS.contains(&ext),
        _ => CARRIED_NAMES.contains(&n.as_str()),
    }
}

/// What an export carried.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub files: usize,
    pub bytes: u64,
    /// Entries left out because the allow list does not take them.
    pub dropped: usize,
}

struct Budget {
    entries: usize,
    bytes: u64,
    out: Exported,
}

fn lower(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_lowercase()
}

/// Why a name may not be part of an export, if it may not.
fn barred(name: &[u8]) -> Option<String> {
    let n = lower(name);
    REPOSITORY_NAMES.contains(&n.as_str()).then(|| format!("{} is version-control state, which can name a program to run; a workshop's repository is not exported.", String::from_utf8_lossy(name)))
}

pub(crate) fn names_in(dir: RawFd) -> Vec<CString> {
    let dup = unsafe { libc::fcntl(dir, libc::F_DUPFD_CLOEXEC, 3) };
    if dup < 0 {
        return Vec::new();
    }
    let stream = unsafe { libc::fdopendir(dup) };
    if stream.is_null() {
        unsafe { libc::close(dup) };
        return Vec::new();
    }
    // The duplicate shares its offset with the descriptor it came from: read from the start.
    unsafe { libc::rewinddir(stream) };
    let mut out = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            out.push(name.to_owned());
        }
    }
    unsafe { libc::closedir(stream) };
    out
}

/// What `name` in `dir` is, without following a link.
pub(crate) fn kind_at(dir: RawFd, name: &CStr) -> Option<libc::mode_t> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::fstatat(dir, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } == 0).then_some(st.st_mode & libc::S_IFMT)
}

/// Is the folder `dir` shaped like something a version-control tool takes for its own?
pub fn repository_shape(dir: RawFd) -> bool {
    let at = |n: &str, want: libc::mode_t| kind_at(dir, &CString::new(n).unwrap()) == Some(want);
    let git_dir = at("HEAD", libc::S_IFREG) && at("objects", libc::S_IFDIR) && at("refs", libc::S_IFDIR);
    git_dir || at("commondir", libc::S_IFREG) || at("gitdir", libc::S_IFREG)
}

const SHAPE_REFUSED: &str = "a folder in the copy is shaped like a git repository (HEAD, objects, refs, or a gitdir file); its config could name a program to run, so nothing was exported.";

pub(crate) const OPEN_DIR: i32 = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

/// A new file `name` in `dir`, never over one, mode 0644, with none of a source's attributes.
fn create_at(dir: RawFd, name: &CStr) -> Result<File, String> {
    use std::os::fd::FromRawFd;
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(dir, name.as_ptr(), flags, 0o644 as libc::c_uint) };
    if fd < 0 {
        return Err(format!("{} could not be made ({})", name.to_string_lossy(), std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// A new folder `name` in `dir`, opened.
pub(crate) fn mkdir_at(dir: RawFd, name: &CStr, mode: libc::mode_t) -> Result<OwnedFd, String> {
    if unsafe { libc::mkdirat(dir, name.as_ptr(), mode) } != 0 {
        return Err(format!("{} could not be made ({})", name.to_string_lossy(), std::io::Error::last_os_error()));
    }
    own(unsafe { libc::openat(dir, name.as_ptr(), OPEN_DIR) })
}

/// Remove `name` in `dir` and everything under it, relative to descriptors and without following a
/// link: what is removed is what is there, never what a path now leads to.
pub(crate) fn remove_at(dir: RawFd, name: &CStr, depth: usize) {
    if kind_at(dir, name) == Some(libc::S_IFDIR) && depth < MAX_DEPTH + 2 {
        if let Ok(sub) = own(unsafe { libc::openat(dir, name.as_ptr(), OPEN_DIR) }) {
            for child in names_in(sub.as_raw_fd()) {
                remove_at(sub.as_raw_fd(), &child, depth + 1);
            }
        }
        unsafe { libc::unlinkat(dir, name.as_ptr(), libc::AT_REMOVEDIR) };
    } else {
        unsafe { libc::unlinkat(dir, name.as_ptr(), 0) };
    }
}

/// Copy the content of a regular file into `out`: the bytes and nothing else.
fn copy_file(src: OwnedFd, mut out: File, budget: &mut Budget) -> Result<(), String> {
    let mut from = File::from(src);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = from.read(&mut buf).map_err(|e| format!("a file could not be read ({e})"))?;
        if n == 0 {
            break;
        }
        budget.bytes += n as u64;
        if budget.bytes > MAX_BYTES {
            return Err("the copy is larger than one export may carry.".into());
        }
        out.write_all(&buf[..n]).map_err(|e| format!("a copy could not be written ({e})"))?;
        budget.out.bytes += n as u64;
    }
    budget.out.files += 1;
    Ok(())
}

/// Judge the source folder `src` before anything is written: a repository by name or by shape, at
/// any depth, refuses the export. Nothing is made.
fn preflight(src: RawFd, depth: usize, seen: &mut usize) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("the folders are nested deeper than one export carries.".into());
    }
    if repository_shape(src) {
        return Err(SHAPE_REFUSED.into());
    }
    for name in names_in(src) {
        if let Some(why) = barred(name.to_bytes()) {
            return Err(why);
        }
        *seen += 1;
        if *seen > MAX_ENTRIES {
            return Err("there are more files than one export carries.".into());
        }
        if kind_at(src, &name) == Some(libc::S_IFDIR) {
            if let Ok(sub) = own(unsafe { libc::openat(src, name.as_ptr(), OPEN_DIR) }) {
                preflight(sub.as_raw_fd(), depth + 1, seen)?;
            }
        }
    }
    Ok(())
}

fn copy_dir(src: RawFd, dest: RawFd, depth: usize, budget: &mut Budget) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("the folders are nested deeper than one export carries.".into());
    }
    for name in names_in(src) {
        if let Some(why) = barred(name.to_bytes()) {
            return Err(why);
        }
        budget.entries += 1;
        if budget.entries > MAX_ENTRIES {
            return Err("there are more files than one export carries.".into());
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(src, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            continue;
        }
        let hidden = name.to_bytes().first() == Some(&b'.');
        match st.st_mode & libc::S_IFMT {
            libc::S_IFREG if !carried(name.to_bytes()) => budget.out.dropped += 1,
            libc::S_IFDIR if hidden => budget.out.dropped += 1,
            libc::S_IFREG => {
                let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
                let Ok(fd) = own(unsafe { libc::openat(src, name.as_ptr(), flags) }) else { continue };
                // What opened must be what was seen: a swap for a link or a pipe is skipped.
                match kind_of(fd.as_raw_fd()) {
                    Ok(now) if now.st_ino == st.st_ino && now.st_dev == st.st_dev && now.st_mode & libc::S_IFMT == libc::S_IFREG => {}
                    _ => continue,
                }
                copy_file(fd, create_at(dest, &name)?, budget)?;
            }
            libc::S_IFDIR => {
                let Ok(fd) = own(unsafe { libc::openat(src, name.as_ptr(), OPEN_DIR) }) else { continue };
                let sub = mkdir_at(dest, &name, 0o755)?;
                copy_dir(fd.as_raw_fd(), sub.as_raw_fd(), depth + 1, budget)?;
            }
            // A link, a pipe, a socket, a device: not bytes, and not taken.
            _ => {}
        }
    }
    // Judged on the copy, which a running job cannot change.
    match repository_shape(dest) {
        true => Err(SHAPE_REFUSED.into()),
        false => Ok(()),
    }
}

/// The destination folder `to` (a path below `home`), opened a name at a time without following a
/// link, and where it really is. Nothing but the home itself is looked up by path.
fn open_dest(home: &Path, to: &Path) -> Result<(OwnedFd, PathBuf), String> {
    let real_home = home.canonicalize().map_err(|e| format!("the home folder cannot be resolved ({e})"))?;
    let rel = to
        .strip_prefix(home)
        .or_else(|_| to.strip_prefix(&real_home))
        .map_err(|_| format!("{} is not in the home folder.", to.display()))?;
    let gone = |p: &Path| format!("{} is not a folder that is there (a link is not followed).", p.display());
    let home_c = CString::new(real_home.as_os_str().as_bytes()).map_err(|_| "a folder with a NUL in it".to_string())?;
    let mut held = own(unsafe { libc::open(home_c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) }).map_err(|_| gone(&real_home))?;
    for c in rel.components() {
        let Component::Normal(name) = c else { return Err(format!("{} has no `..` or `.` in it.", to.display())) };
        let name = CString::new(name.as_bytes()).map_err(|_| "a folder with a NUL in it".to_string())?;
        held = own(unsafe { libc::openat(held.as_raw_fd(), name.as_ptr(), OPEN_DIR) }).map_err(|_| gone(to))?;
    }
    let at = std::fs::read_link(format!("/proc/self/fd/{}", held.as_raw_fd())).map_err(|_| gone(to))?;
    Ok((held, at))
}

/// The destination judged as a place to make `last` in: the same rule the card applied.
fn may_land(at: &Path, last: &str, home: &Path) -> Result<(), String> {
    yantrik_ipc_contracts::home_paths::may_create(&format!("{}/{last}", at.display()), home)
}

/// `renameat2(old -> new)` inside `dir`, failing if `new` is there.
fn rename_noreplace(dir: RawFd, old: &CStr, new: &CStr) -> Result<(), String> {
    let done = unsafe { libc::syscall(libc::SYS_renameat2, dir, old.as_ptr(), dir, new.as_ptr(), libc::RENAME_NOREPLACE) };
    if done == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EEXIST) | Some(libc::ENOTEMPTY) => Err(format!("{} could not be made (it is there already)", new.to_string_lossy())),
        // A filesystem with no such flag: look first, then rename.
        Some(libc::EINVAL) | Some(libc::ENOSYS) if kind_at(dir, new).is_none() => {
            match unsafe { libc::renameat(dir, old.as_ptr(), dir, new.as_ptr()) } {
                0 => Ok(()),
                _ => Err(format!("{} could not be made ({})", new.to_string_lossy(), std::io::Error::last_os_error())),
            }
        }
        _ => Err(format!("{} could not be made ({err})", new.to_string_lossy())),
    }
}

/// A name that is not taken and that no program treats as a project: hidden, and this call's.
fn scratch_name() -> CString {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    CString::new(format!(".yantrik-export-{}-{nanos}", std::process::id())).unwrap()
}

/// Copy what is at `path` in `workshop` into the folder `to` (below `home`), as a new entry of the
/// same name (it never replaces one). A regular file by content, or a folder of them.
pub fn export(workshop: &Path, path: &Path, home: &Path, to: &Path) -> Result<(PathBuf, Exported), String> {
    export_with(workshop, path, home, to, &|| {})
}

/// [`export`], calling `between` once the destination is open and the source has been judged, and
/// before the first byte is copied. A test uses it to do to the destination what a job could.
pub fn export_with(workshop: &Path, path: &Path, home: &Path, to: &Path, between: &dyn Fn()) -> Result<(PathBuf, Exported), String> {
    let names = below(workshop, path)?;
    let Some(last) = names.last() else { return Err("name a file or folder inside the workshop.".into()) };
    if let Some(why) = barred(last.as_bytes()) {
        return Err(why);
    }
    if last.as_bytes().first() == Some(&b'.') {
        return Err(format!("{} starts with a dot, which is a name programs read settings from; it is not exported.", last.to_string_lossy()));
    }
    let name = CString::new(last.as_bytes()).map_err(|_| "a name with a NUL in it".to_string())?;
    let shown = last.to_string_lossy().into_owned();
    // 1. The destination, by descriptor, judged where it really is.
    let (dest, approved) = open_dest(home, to)?;
    may_land(&approved, &shown, home)?;
    if kind_at(dest.as_raw_fd(), &name).is_some() {
        return Err(format!("{shown} could not be made (it is there already)"));
    }
    // 2. The source, judged before a byte is written.
    let root = open_workshop(workshop)?;
    let fd = open_below(root.as_raw_fd(), &names, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)?;
    let kind = kind_of(fd.as_raw_fd())?.st_mode & libc::S_IFMT;
    match kind {
        libc::S_IFREG if !carried(last.as_bytes()) => return Err(format!("{shown} is not a kind of file an export carries (documents, pictures, sound, video, text, data and source are).")),
        libc::S_IFREG => {}
        libc::S_IFDIR => preflight(fd.as_raw_fd(), 0, &mut 0)?,
        _ => return Err("only a regular file or a folder of them is exported.".into()),
    }
    between();
    // 3. Built under a hidden name of this call's, and judged again on the copy.
    let scratch = scratch_name();
    let mut budget = Budget { entries: 0, bytes: 0, out: Exported::default() };
    let built = (|| -> Result<(), String> {
        if kind == libc::S_IFDIR {
            let sub = mkdir_at(dest.as_raw_fd(), &scratch, 0o700)?;
            copy_dir(fd.as_raw_fd(), sub.as_raw_fd(), 0, &mut budget)?;
            unsafe { libc::fchmod(sub.as_raw_fd(), 0o755) };
        } else {
            copy_file(fd, create_at(dest.as_raw_fd(), &scratch)?, &mut budget)?;
        }
        // 4. Still where it was approved? Then, and only then, it takes its name.
        let now = std::fs::read_link(format!("/proc/self/fd/{}", dest.as_raw_fd())).unwrap_or_default();
        if now != approved {
            return Err(format!("the destination was moved to {} while the copy ran; nothing was exported.", now.display()));
        }
        may_land(&now, &shown, home)?;
        rename_noreplace(dest.as_raw_fd(), &scratch, &name)
    })();
    if let Err(e) = built {
        // Only what this call made, by descriptor: never a path that may lead somewhere else now.
        remove_at(dest.as_raw_fd(), &scratch, 0);
        return Err(e);
    }
    Ok((approved.join(last), budget.out))
}

#[cfg(all(test, unix))]
#[path = "workshop_export_tests.rs"]
mod tests;
