//! Descriptor-relative folder calls for `coder_promote` (#1265, round 10).
//!
//! Every call names a child of a held folder descriptor, and every folder is opened with
//! `O_NOFOLLOW`: a link planted on the way (in the staging folder or in the target) is refused
//! with `ELOOP`, never followed. Nothing here resolves a path a second time.

use std::ffi::{CString, OsStr, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path};

/// A folder, held by descriptor.
pub struct Dir(OwnedFd);

fn name(n: &OsStr) -> Result<CString, String> {
    CString::new(n.as_bytes()).map_err(|_| "a name with a NUL".to_string())
}

fn os(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

const DIR_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

impl Dir {
    /// The folder at `path`, which must not itself be a link.
    pub fn open(path: &Path) -> Result<Dir, String> {
        let c = name(path.as_os_str())?;
        // SAFETY: `c` is a valid NUL-terminated path; the call only reads it.
        let fd = unsafe { libc::open(c.as_ptr(), DIR_FLAGS) };
        if fd < 0 { Err(os(&format!("{} is not a plain folder", path.display()))) } else { Ok(Dir(unsafe { OwnedFd::from_raw_fd(fd) })) }
    }

    /// The child folder `n` (never a link).
    pub fn child(&self, n: &OsStr) -> Result<Dir, String> {
        let c = name(n)?;
        // SAFETY: as above, relative to this folder's descriptor.
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), c.as_ptr(), DIR_FLAGS) };
        if fd < 0 { Err(os(&format!("{} is not a plain folder", n.to_string_lossy()))) } else { Ok(Dir(unsafe { OwnedFd::from_raw_fd(fd) })) }
    }

    /// Make the child folder `n` (it may already be one).
    pub fn mkdir(&self, n: &OsStr, mode: u32) -> Result<(), String> {
        let c = name(n)?;
        // SAFETY: as above.
        let r = unsafe { libc::mkdirat(self.0.as_raw_fd(), c.as_ptr(), mode as libc::mode_t) };
        if r == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists { Ok(()) } else { Err(os(&n.to_string_lossy())) }
    }

    /// The folder `rel` (plain components) below this one, made as needed, step by step through
    /// descriptors: a link on the way refuses.
    pub fn mkdir_p(&self, rel: &Path, mode: u32) -> Result<Dir, String> {
        let mut at = self.child(OsStr::new("."))?;
        for part in rel.components() {
            let Component::Normal(p) = part else { return Err(format!("a name that is not plain ({})", rel.display())) };
            at.mkdir(p, mode)?;
            at = at.child(p)?;
        }
        Ok(at)
    }

    /// A new file `n` in this folder: `O_EXCL` and `O_NOFOLLOW`, so it never opens a link and never
    /// writes over a name made before.
    pub fn create(&self, n: &OsStr, mode: u32) -> Result<std::fs::File, String> {
        let c = name(n)?;
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: as above; the descriptor is made owned at once.
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), c.as_ptr(), flags, mode as libc::c_uint) };
        if fd < 0 { Err(os(&n.to_string_lossy())) } else { Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) })) }
    }

    /// An existing regular file `n`, to read (never a link).
    fn open_file(&self, n: &OsStr) -> Result<std::fs::File, String> {
        let c = name(n)?;
        // SAFETY: as above.
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 { Err(os(&n.to_string_lossy())) } else { Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) })) }
    }

    /// The names in this folder.
    pub fn names(&self) -> Result<Vec<OsString>, String> {
        // A fresh descriptor, so the listing has an offset of its own.
        let own = self.child(OsStr::new("."))?;
        let fd = own.0.as_raw_fd();
        // SAFETY: `fdopendir` takes over `fd`; it is released with `closedir` below, and `own` is
        // forgotten so it is not closed twice. `readdir` entries are read before the next call.
        let dir = unsafe { libc::fdopendir(fd) };
        if dir.is_null() {
            return Err(os("listing a folder"));
        }
        std::mem::forget(own);
        let mut out = Vec::new();
        loop {
            let e = unsafe { libc::readdir(dir) };
            if e.is_null() {
                break;
            }
            let n = unsafe { std::ffi::CStr::from_ptr((*e).d_name.as_ptr()) }.to_bytes();
            if n != b"." && n != b".." {
                out.push(OsString::from_vec(n.to_vec()));
            }
        }
        unsafe { libc::closedir(dir) };
        Ok(out)
    }

    /// The `st_mode` of the child `n`, without following a link; `None` when it is not there.
    pub fn mode_of(&self, n: &OsStr) -> Result<Option<u32>, String> {
        let c = name(n)?;
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: `st` is a valid out-pointer.
        let r = unsafe { libc::fstatat(self.0.as_raw_fd(), c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
        if r == 0 {
            return Ok(Some(st.st_mode));
        }
        match std::io::Error::last_os_error().kind() {
            std::io::ErrorKind::NotFound => Ok(None),
            _ => Err(os(&n.to_string_lossy())),
        }
    }

    /// Whether the folder is the account's own and closed to others (mode `0700`).
    pub fn is_private(&self) -> bool {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: `st` is a valid out-pointer.
        unsafe { libc::fstat(self.0.as_raw_fd(), &mut st) == 0 && st.st_uid == libc::geteuid() && st.st_mode & 0o077 == 0 }
    }

    /// `renameat2`: move `from` of this folder to `to` of `other`. `exchange` trades the two in one step;
    /// otherwise an existing `to` is not replaced.
    pub fn rename(&self, from: &OsStr, other: &Dir, to: &OsStr, exchange: bool) -> Result<(), String> {
        let (a, b) = (name(from)?, name(to)?);
        let flags = if exchange { libc::RENAME_EXCHANGE } else { libc::RENAME_NOREPLACE };
        // SAFETY: valid paths and descriptors; the call reads them only.
        let r = unsafe { libc::syscall(libc::SYS_renameat2, self.0.as_raw_fd(), a.as_ptr(), other.0.as_raw_fd(), b.as_ptr(), flags) };
        if r == 0 { Ok(()) } else { Err(os(&format!("moving {}", from.to_string_lossy()))) }
    }

    /// Put the regular file `n` of this folder into `to` under the same name: a hard link, or a copy
    /// where the file system has none.
    pub fn link_or_copy(&self, n: &OsStr, to: &Dir) -> Result<(), String> {
        let c = name(n)?;
        // SAFETY: as above; no flag, so a link is never followed.
        if unsafe { libc::linkat(self.0.as_raw_fd(), c.as_ptr(), to.0.as_raw_fd(), c.as_ptr(), 0) } == 0 {
            return Ok(());
        }
        let mut src = self.open_file(n)?;
        let exec = self.mode_of(n)?.is_some_and(|m| m & 0o100 != 0);
        let mut dst = to.create(n, if exec { 0o755 } else { 0o644 })?;
        std::io::copy(&mut src, &mut dst).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// What `carry` leaves out: the whole of a folder's plain files and folders are put in `to`; a link
/// of either kind, a device or a pipe is not carried.
pub fn carry(from: &Dir, to: &Dir) -> Result<(), String> {
    for n in from.names()? {
        match from.mode_of(&n)?.map(|m| m & libc::S_IFMT) {
            Some(libc::S_IFDIR) => {
                to.mkdir(&n, 0o755)?;
                carry(&from.child(&n)?, &to.child(&n)?)?;
            }
            Some(libc::S_IFREG) => from.link_or_copy(&n, to)?,
            _ => {}
        }
    }
    Ok(())
}
