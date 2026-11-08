//! The carded copy out of a workshop (#1120, #1124, H2 and M3).
//!
//! Plain bytes, and nothing that a program reads as configuration or runs:
//!
//! - **regular files by content**: no link, pipe, socket or device, no mode bits (0644), no
//!   extended attributes, never over a name that is there;
//! - **no repository**, by what a thing is and not what it is called. A folder is refused, and
//!   with it the whole export, when it is named `.git`, `.hg`, `.jj`, `.svn` or `.bzr` (any case),
//!   when it holds a `gitdir` or `commondir` file, or when what was copied into it has a git
//!   directory's own shape (`HEAD` a file, `objects` and `refs` folders): `git` takes such a
//!   folder for a repository wherever it sits, and its `config` can name a program to run;
//! - **no startup file**: a leading dot on the name that lands is refused, and a few files that
//!   tools run or obey on sight are left out (`dropped` says how many).
//!
//! The shape is judged on the *copy*, which the sealed job cannot touch, after the entries of
//! each folder are in: a file that vanishes from the source after it was copied does not hide it.
//!
//! Everything lands relative to a descriptor for the destination folder, opened without following
//! a link, so a link swapped in between the card and the copy is never followed.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::workshop_out::{below, kind_of, open_below, open_workshop, own};

/// Entries and bytes one export may carry.
const MAX_ENTRIES: usize = 20_000;
const MAX_BYTES: u64 = 8 << 30;
const MAX_DEPTH: usize = 32;

/// Version control state: a folder or file by one of these names makes the whole export refused.
const REPOSITORY_NAMES: [&str; 5] = [".git", ".hg", ".jj", ".svn", ".bzr"];

/// Files that a tool runs or obeys on sight in a folder, which are left out of a copy.
const LEFT_OUT: [&str; 14] = [
    ".gitattributes", ".gitmodules", ".gitconfig", ".hgrc", ".envrc", ".npmrc", ".yarnrc", ".yarnrc.yml", ".cargo", ".vscode",
    ".husky", ".pre-commit-config.yaml", ".bashrc", ".zshrc",
];

/// What an export carried.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub files: usize,
    pub bytes: u64,
    /// Entries left out because a tool would run or obey them (`LEFT_OUT`).
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

fn names_in(dir: RawFd) -> Vec<CString> {
    let dup = unsafe { libc::fcntl(dir, libc::F_DUPFD_CLOEXEC, 3) };
    if dup < 0 {
        return Vec::new();
    }
    let stream = unsafe { libc::fdopendir(dup) };
    if stream.is_null() {
        unsafe { libc::close(dup) };
        return Vec::new();
    }
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
fn kind_at(dir: RawFd, name: &CStr) -> Option<libc::mode_t> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::fstatat(dir, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } == 0).then_some(st.st_mode & libc::S_IFMT)
}

/// Is the folder `dir` shaped like something a version-control tool takes for its own?
pub fn repository_shape(dir: RawFd) -> bool {
    let at = |n: &str, want: libc::mode_t| kind_at(dir, &CString::new(n).unwrap()) == Some(want);
    let git_dir = at("HEAD", libc::S_IFREG) && at("objects", libc::S_IFDIR) && at("refs", libc::S_IFDIR);
    git_dir || at("commondir", libc::S_IFREG) || at("gitdir", libc::S_IFREG)
}

const OPEN_DIR: i32 = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

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
fn mkdir_at(dir: RawFd, name: &CStr) -> Result<OwnedFd, String> {
    if unsafe { libc::mkdirat(dir, name.as_ptr(), 0o755) } != 0 {
        return Err(format!("{} could not be made ({})", name.to_string_lossy(), std::io::Error::last_os_error()));
    }
    own(unsafe { libc::openat(dir, name.as_ptr(), OPEN_DIR) })
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

fn copy_dir(src: RawFd, dest: RawFd, depth: usize, budget: &mut Budget) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("the folders are nested deeper than one export carries.".into());
    }
    for name in names_in(src) {
        if let Some(why) = barred(name.to_bytes()) {
            return Err(why);
        }
        if LEFT_OUT.contains(&lower(name.to_bytes()).as_str()) {
            budget.out.dropped += 1;
            continue;
        }
        budget.entries += 1;
        if budget.entries > MAX_ENTRIES {
            return Err("there are more files than one export carries.".into());
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(src, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            continue;
        }
        match st.st_mode & libc::S_IFMT {
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
                let sub = mkdir_at(dest, &name)?;
                copy_dir(fd.as_raw_fd(), sub.as_raw_fd(), depth + 1, budget)?;
            }
            // A link, a pipe, a socket, a device: not bytes, and not taken.
            _ => {}
        }
    }
    // Judged on the copy, which a running job cannot change.
    match repository_shape(dest) {
        true => Err("a folder in the copy is shaped like a git repository (HEAD, objects, refs, or a gitdir file); its config could name a program to run, so nothing was exported.".into()),
        false => Ok(()),
    }
}

/// Copy what is at `path` in `workshop` into the existing folder `into`, as a new entry of the
/// same name (it never replaces one). A regular file by content, or a folder of them.
pub fn export(workshop: &Path, path: &Path, into: &Path) -> Result<(PathBuf, Exported), String> {
    let names = below(workshop, path)?;
    let Some(last) = names.last() else { return Err("name a file or folder inside the workshop.".into()) };
    if let Some(why) = barred(last.as_bytes()) {
        return Err(why);
    }
    if last.as_bytes().first() == Some(&b'.') {
        return Err(format!("{} starts with a dot, which is a name programs read settings from; it is not exported.", last.to_string_lossy()));
    }
    let name = CString::new(last.as_bytes()).map_err(|_| "a name with a NUL in it".to_string())?;
    let to = CString::new(into.as_os_str().as_bytes()).map_err(|_| "a folder with a NUL in it".to_string())?;
    let dest = own(unsafe { libc::open(to.as_ptr(), OPEN_DIR) }).map_err(|_| format!("{} is not a folder that is there (a link is not followed).", into.display()))?;
    let root = open_workshop(workshop)?;
    let fd = open_below(root.as_raw_fd(), &names, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)?;
    let kind = kind_of(fd.as_raw_fd())?.st_mode & libc::S_IFMT;
    let target = into.join(last);
    let mut budget = Budget { entries: 0, bytes: 0, out: Exported::default() };
    let done = match kind {
        // A name that is taken is refused here, before anything is made, so nothing is removed below.
        libc::S_IFREG => {
            let out = create_at(dest.as_raw_fd(), &name)?;
            copy_file(fd, out, &mut budget)
        }
        libc::S_IFDIR => {
            let sub = mkdir_at(dest.as_raw_fd(), &name)?;
            copy_dir(fd.as_raw_fd(), sub.as_raw_fd(), 0, &mut budget)
        }
        _ => return Err("only a regular file or a folder of them is exported.".into()),
    };
    if let Err(e) = done {
        // Only what this call made: the name was new, so a half copy is removed.
        let _ = if kind == libc::S_IFDIR { std::fs::remove_dir_all(&target) } else { std::fs::remove_file(&target) };
        return Err(e);
    }
    Ok((target, budget.out))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn shop() -> (tempfile::TempDir, PathBuf) {
        let h = tempfile::tempdir().unwrap();
        let shop = h.path().canonicalize().unwrap().join("shop");
        std::fs::create_dir(&shop).unwrap();
        (h, shop)
    }

    fn fifo(at: &Path) {
        let c = CString::new(at.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    fn dest(h: &tempfile::TempDir) -> PathBuf {
        let d = h.path().join("Videos");
        std::fs::create_dir(&d).unwrap();
        d
    }

    #[test]
    fn export_copies_regular_files_by_content_and_nothing_else() {
        let (h, shop) = shop();
        let secret = h.path().join("secret");
        std::fs::write(&secret, "host secret").unwrap();
        let src = shop.join("pkg");
        std::fs::create_dir_all(src.join("sub/deep")).unwrap();
        std::fs::write(src.join("a.txt"), "alpha").unwrap();
        std::fs::write(src.join("sub/deep/b.txt"), "beta").unwrap();
        std::fs::write(src.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(src.join("run.sh"), std::fs::Permissions::from_mode(0o4755)).unwrap();
        symlink(&secret, src.join("leak")).unwrap();
        symlink(h.path(), src.join("dir-leak")).unwrap();
        fifo(&src.join("pipe"));
        // Settings a tool obeys on sight are left out, and said so.
        std::fs::write(src.join(".npmrc"), "script-shell=/tmp/x").unwrap();
        std::fs::create_dir_all(src.join(".cargo")).unwrap();
        std::fs::write(src.join(".cargo/config.toml"), "[build]\nrustc-wrapper='x'").unwrap();
        // An extended attribute, where the filesystem takes one.
        let name = CString::new("user.mark").unwrap();
        let path = CString::new(src.join("a.txt").as_os_str().as_bytes()).unwrap();
        let tagged = unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), b"1".as_ptr().cast(), 1, 0) } == 0;
        let dest = dest(&h);
        let (at, done) = export(&shop, Path::new("pkg"), &dest).unwrap();
        assert_eq!(at, dest.join("pkg"));
        assert_eq!(done, Exported { files: 3, bytes: 5 + 4 + 18, dropped: 2 });
        assert_eq!(std::fs::read_to_string(at.join("a.txt")).unwrap(), "alpha");
        assert_eq!(std::fs::read_to_string(at.join("sub/deep/b.txt")).unwrap(), "beta");
        for gone in ["leak", "dir-leak", "pipe", ".npmrc", ".cargo"] {
            assert!(at.join(gone).symlink_metadata().is_err(), "{gone} must not be copied");
        }
        // Plain bytes: no setuid or execute bit, no attribute, a regular file.
        let mode = std::fs::metadata(at.join("run.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o7000, 0, "{mode:o}");
        assert_eq!(mode & 0o111, 0, "{mode:o}");
        if tagged {
            let copy = CString::new(at.join("a.txt").as_os_str().as_bytes()).unwrap();
            assert!(unsafe { libc::getxattr(copy.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) } < 0, "the attribute was copied");
        }
        // It never replaces what is there.
        assert!(export(&shop, Path::new("pkg"), &dest).unwrap_err().contains("could not be made"));
        assert_eq!(std::fs::read_to_string(at.join("a.txt")).unwrap(), "alpha");
    }

    #[test]
    fn export_refuses_links_pipes_outside_paths_and_a_missing_folder() {
        let (h, shop) = shop();
        let dest = dest(&h);
        std::fs::write(h.path().join("outside.txt"), "x").unwrap();
        symlink(h.path().join("outside.txt"), shop.join("link.txt")).unwrap();
        symlink(h.path(), shop.join("dir-link")).unwrap();
        fifo(&shop.join("pipe"));
        for bad in ["link.txt", "dir-link", "dir-link/outside.txt", "pipe", "../outside.txt", "missing", "", "/etc/passwd"] {
            assert!(export(&shop, Path::new(bad), &dest).is_err(), "{bad:?}");
        }
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0, "nothing landed");
        std::fs::write(shop.join("ok.txt"), "ok").unwrap();
        assert!(export(&shop, Path::new("ok.txt"), &dest.join("nowhere")).is_err());
        assert!(export(&shop, Path::new("ok.txt"), &dest).is_ok());
    }

    /// H2: a repository is what it looks like, not what it is called. Every one of these is
    /// refused whole, and nothing is left behind.
    #[test]
    fn export_refuses_a_repository_by_its_shape_at_every_level_and_by_name_in_any_case() {
        let (h, shop) = shop();
        let dest = dest(&h);
        // `git init --bare meta` and then `core.bare=false`, `core.worktree=..`, `core.fsmonitor`.
        let bare = |at: &Path| {
            std::fs::create_dir_all(at.join("objects")).unwrap();
            std::fs::create_dir_all(at.join("refs/heads")).unwrap();
            std::fs::write(at.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            std::fs::write(at.join("config"), "[core]\n\tbare = false\n\tworktree = ..\n\tfsmonitor = touch /tmp/PWNED; false\n").unwrap();
        };
        bare(&shop.join("proj/meta"));
        std::fs::write(shop.join("proj/readme"), "hi").unwrap();
        bare(&shop.join("whole"));
        bare(&shop.join("deep/er/still/inside"));
        std::fs::create_dir_all(shop.join("worktree/sub")).unwrap();
        std::fs::write(shop.join("worktree/sub/gitdir"), "/elsewhere/.git\n").unwrap();
        std::fs::create_dir_all(shop.join("linked")).unwrap();
        std::fs::write(shop.join("linked/commondir"), "../..\n").unwrap();
        for (i, n) in [".git", ".GIT", ".Hg", ".jj", ".svn", ".bzr"].iter().enumerate() {
            std::fs::create_dir_all(shop.join(format!("named{i}"))).unwrap();
            std::fs::write(shop.join(format!("named{i}/{n}")), "gitdir: /x\n").unwrap();
            std::fs::create_dir_all(shop.join(format!("dirnamed{i}/{n}"))).unwrap();
        }
        for bad in ["proj", "proj/meta", "whole", "deep", "worktree", "linked", "named0", "named1", "named2", "named3", "named4", "named5", "dirnamed0", "dirnamed2", "dirnamed3"] {
            let why = export(&shop, Path::new(bad), &dest).unwrap_err();
            assert!(why.contains("repository") || why.contains("version-control"), "{bad}: {why}");
        }
        for bad in [".git", ".HG", ".jj"] {
            assert!(export(&shop, Path::new(bad), &dest).is_err(), "{bad}");
        }
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0, "a refused export leaves nothing behind");
        // A folder that only has a file called HEAD is not a repository.
        std::fs::create_dir_all(shop.join("notes")).unwrap();
        std::fs::write(shop.join("notes/HEAD"), "head").unwrap();
        assert!(export(&shop, Path::new("notes"), &dest).is_ok());
    }

    /// H2, end to end where git is installed: what is not refused is not a repository to git.
    #[test]
    fn a_refused_export_leaves_nothing_for_git_to_run() {
        let git = |args: &[&str], dir: &Path| std::process::Command::new("git").args(args).current_dir(dir).output();
        let (h, shop) = shop();
        let dest = dest(&h);
        std::fs::create_dir_all(shop.join("pkg")).unwrap();
        let marker = h.path().join("PWNED");
        let init = git(&["init", "--bare", "-q", "meta"], &shop.join("pkg"));
        if init.map_or(true, |o| !o.status.success()) {
            return; // no git here
        }
        let cfg = format!("touch {}; false", marker.display());
        for kv in [["core.bare", "false"], ["core.worktree", ".."], ["core.fsmonitor", cfg.as_str()]] {
            git(&["config", kv[0], kv[1]], &shop.join("pkg/meta")).unwrap();
        }
        assert!(export(&shop, Path::new("pkg"), &dest).is_err());
        let _ = git(&["status"], &dest);
        assert!(!marker.exists(), "git ran the marker");
    }

    /// M3: the name that lands is judged like the folder it lands in, and a link in the place of
    /// the folder is not followed.
    #[test]
    fn export_refuses_a_startup_name_and_a_destination_that_is_a_link() {
        let (h, shop) = shop();
        let dest = dest(&h);
        for n in [".bash_profile", ".zshenv", ".gitconfig", ".pam_environment", ".npmrc"] {
            std::fs::write(shop.join(n), "x").unwrap();
            assert!(export(&shop, Path::new(n), &dest).unwrap_err().contains("starts with a dot"), "{n}");
            assert!(dest.join(n).symlink_metadata().is_err(), "{n} landed");
        }
        std::fs::write(shop.join("x.desktop"), "[Desktop Entry]\nExec=sh").unwrap();
        let autostart = h.path().join("autostart");
        std::fs::create_dir(&autostart).unwrap();
        let videos_link = h.path().join("Videos-link");
        symlink(&autostart, &videos_link).unwrap();
        let why = export(&shop, Path::new("x.desktop"), &videos_link).unwrap_err();
        assert!(why.contains("not a folder that is there"), "{why}");
        assert_eq!(std::fs::read_dir(&autostart).unwrap().count(), 0, "nothing landed through the link");
        // A name that is a link made after the card is not written through either.
        symlink(h.path().join("elsewhere"), dest.join("x.desktop")).unwrap();
        assert!(export(&shop, Path::new("x.desktop"), &dest).is_err());
        assert!(!h.path().join("elsewhere").exists());
    }
}
