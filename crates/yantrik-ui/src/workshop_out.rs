//! What leaves a Mind's workshop, and the only two ways it can (#1120, #1124).
//!
//! What a Mind builds in a workshop can hold anything a program reads or runs, so nothing outside
//! the two mount views touches it. Bytes leave two ways:
//!
//! - **`present`**: the file is opened here, `O_RDONLY|O_NOFOLLOW`, and must be a regular file of a
//!   media type its first bytes name (not its extension); the descriptor, not the path, goes to the
//!   OS's own player, which says whose workshop it came from. No card: a decoder reads bytes and
//!   runs no hooks.
//! - **`export`**: a copy of regular files by content, carded by the caller. No `.git`, no link,
//!   no device or pipe, no mode bits, no extended attributes: what lands is plain bytes the person
//!   chose to take out.
//!
//! Both walk from a descriptor for the workshop and open each name with `O_NOFOLLOW`, so a link a
//! sealed job swaps in while this runs is never followed.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

/// Entries and bytes one export may carry.
const MAX_ENTRIES: usize = 20_000;
const MAX_BYTES: u64 = 8 << 30;
const MAX_DEPTH: usize = 32;

fn cstr(name: &std::ffi::OsStr) -> Result<CString, String> {
    CString::new(name.as_bytes()).map_err(|_| "a name with a NUL in it".to_string())
}

fn own(fd: RawFd) -> Result<OwnedFd, String> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The directory `workshop`, opened without following a link.
fn open_workshop(workshop: &Path) -> Result<OwnedFd, String> {
    let c = cstr(workshop.as_os_str())?;
    own(unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })
        .map_err(|e| format!("the workshop cannot be opened ({e})"))
}

/// `path` as names below `workshop`: relative, or absolute and inside it. No `..`, no `.`.
fn below(workshop: &Path, path: &Path) -> Result<Vec<std::ffi::OsString>, String> {
    let rel = if path.is_absolute() {
        path.strip_prefix(workshop).map_err(|_| "that path is outside the workshop.".to_string())?
    } else {
        path
    };
    let mut names = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(n) => names.push(n.to_os_string()),
            _ => return Err("the path is names inside the workshop, with no `..` or `.` in it.".into()),
        }
    }
    Ok(names)
}

/// Open the thing at `names` below `dir`, each name `O_NOFOLLOW`; the last one as `last` says.
fn open_below(dir: RawFd, names: &[std::ffi::OsString], last: i32) -> Result<OwnedFd, String> {
    let mut held: Option<OwnedFd> = None;
    for (i, name) in names.iter().enumerate() {
        let at = held.as_ref().map_or(dir, AsRawFd::as_raw_fd);
        let flags = if i + 1 == names.len() { last } else { libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC };
        let c = cstr(name)?;
        let next = own(unsafe { libc::openat(at, c.as_ptr(), flags) }).map_err(|e| format!("{} could not be opened without following a link ({e})", name.to_string_lossy()))?;
        held = Some(next);
    }
    held.ok_or_else(|| "name a file inside the workshop.".to_string())
}

fn kind_of(fd: RawFd) -> Result<libc::stat, String> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(st)
}

/// The media type the first bytes of a file name, or `None`. Not the extension: that is a word the
/// Mind chose.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, s: &[u8]| head.get(i..i + s.len()) == Some(s);
    Some(match () {
        _ if at(4, b"ftyp") => "video/mp4",
        _ if at(0, &[0x1A, 0x45, 0xDF, 0xA3]) => "video/webm",
        _ if at(0, b"OggS") => "audio/ogg",
        _ if at(0, b"RIFF") && at(8, b"WAVE") => "audio/wav",
        _ if at(0, b"RIFF") && at(8, b"WEBP") => "image/webp",
        _ if at(0, b"RIFF") && at(8, b"AVI ") => "video/avi",
        _ if at(0, b"fLaC") => "audio/flac",
        _ if at(0, b"ID3") || (head.len() > 1 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0) => "audio/mpeg",
        _ if at(0, &[0x89, b'P', b'N', b'G']) => "image/png",
        _ if at(0, &[0xFF, 0xD8, 0xFF]) => "image/jpeg",
        _ if at(0, b"GIF8") => "image/gif",
        _ => return None,
    })
}

/// The file at `path` in `workshop`, opened for the player: a regular file of a media type.
pub fn checked_media(workshop: &Path, path: &Path) -> Result<(OwnedFd, &'static str), String> {
    let names = below(workshop, path)?;
    let root = open_workshop(workshop)?;
    // O_NONBLOCK: a pipe left there does not hold this open; it is refused below.
    let fd = open_below(root.as_raw_fd(), &names, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)?;
    if kind_of(fd.as_raw_fd())?.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err("that is not a regular file (a link, pipe, device or folder is not shown).".into());
    }
    let mut head = [0u8; 64];
    let n = unsafe { libc::pread(fd.as_raw_fd(), head.as_mut_ptr().cast(), head.len(), 0) };
    let media = (n > 0).then(|| sniff(&head[..n as usize])).flatten();
    match media {
        Some(kind) => Ok((fd, kind)),
        None => Err("its first bytes are not a video, audio or picture format, so it is not shown.".into()),
    }
}

/// "Hermes" for `hermes:main`, with nothing in it that draws differently from how it reads.
pub fn mind_name(agent: &str) -> String {
    let id: String = agent.split(':').next().unwrap_or(agent).chars().filter(|c| !c.is_control() && !crate::approvals::is_format_char(*c)).take(40).collect();
    let mut chars = id.chars();
    chars.next().map(|f| f.to_uppercase().chain(chars).collect()).unwrap_or_else(|| "A Mind".into())
}

/// Show the file in the OS's player, from its descriptor. The player's title says whose it is.
pub fn present(workshop: &Path, path: &Path, agent: &str) -> Result<String, String> {
    let (fd, media) = checked_media(workshop, path)?;
    let line = format!("From {}'s workshop", mind_name(agent));
    let mut cmd = std::process::Command::new("mpv");
    cmd.args(["--player-operation-mode=pseudo-gui", "--ytdl=no"])
        .arg(format!("--force-media-title={line}"))
        .arg(format!("--osd-playing-msg={line}"))
        .args(["--", "fd://3"])
        .env_remove("SLINT_FULLSCREEN")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let raw = fd.as_raw_fd();
    // Safety: only dup2 and fcntl between fork and exec. The player gets the file as descriptor 3.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            if raw == 3 {
                let flags = libc::fcntl(3, libc::F_GETFD);
                if flags < 0 || libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(raw, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("no media player (mpv) could be started ({e})"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(media.to_string())
}

/// What an export carried.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub files: usize,
    pub bytes: u64,
}

struct Budget {
    entries: usize,
    bytes: u64,
    out: Exported,
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

/// A new file at `to`, never over one, mode 0644, with none of a source's attributes.
fn create_new(to: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(to)
        .map_err(|e| format!("{} could not be made ({e})", to.display()))
}

/// Copy the content of a regular file into `out`: the bytes and nothing else.
fn copy_file(src: OwnedFd, mut out: File, to: &Path, budget: &mut Budget) -> Result<(), String> {
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
        std::io::Write::write_all(&mut out, &buf[..n]).map_err(|e| format!("{} could not be written ({e})", to.display()))?;
        budget.out.bytes += n as u64;
    }
    budget.out.files += 1;
    Ok(())
}

fn copy_dir(dir: RawFd, to: &Path, depth: usize, budget: &mut Budget) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("the folders are nested deeper than one export carries.".into());
    }
    for name in names_in(dir) {
        // A repository's own folder (or the file that points at one) holds hooks and settings.
        if name.to_bytes() == b".git" {
            continue;
        }
        budget.entries += 1;
        if budget.entries > MAX_ENTRIES {
            return Err("there are more files than one export carries.".into());
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(dir, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            continue;
        }
        let target = to.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
        match st.st_mode & libc::S_IFMT {
            libc::S_IFREG => {
                let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
                let Ok(fd) = own(unsafe { libc::openat(dir, name.as_ptr(), flags) }) else { continue };
                // What opened must be what was seen: a swap for a link or a pipe is skipped.
                match kind_of(fd.as_raw_fd()) {
                    Ok(now) if now.st_ino == st.st_ino && now.st_dev == st.st_dev && now.st_mode & libc::S_IFMT == libc::S_IFREG => {}
                    _ => continue,
                }
                copy_file(fd, create_new(&target)?, &target, budget)?;
            }
            libc::S_IFDIR => {
                let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                let Ok(fd) = own(unsafe { libc::openat(dir, name.as_ptr(), flags) }) else { continue };
                std::fs::create_dir(&target).map_err(|e| format!("{} could not be made ({e})", target.display()))?;
                copy_dir(fd.as_raw_fd(), &target, depth + 1, budget)?;
            }
            // A link, a pipe, a socket, a device: not bytes, and not taken.
            _ => {}
        }
    }
    Ok(())
}

/// Copy what is at `path` in `workshop` into the existing folder `into`, as a new entry of the
/// same name (it never replaces one). A regular file by content, or a folder of them.
pub fn export(workshop: &Path, path: &Path, into: &Path) -> Result<(PathBuf, Exported), String> {
    let names = below(workshop, path)?;
    let Some(last) = names.last() else { return Err("name a file or folder inside the workshop.".into()) };
    if last.as_bytes() == b".git" {
        return Err("a repository's .git folder is not exported.".into());
    }
    if !into.is_dir() {
        return Err(format!("{} is not a folder that is there.", into.display()));
    }
    let root = open_workshop(workshop)?;
    let fd = open_below(root.as_raw_fd(), &names, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)?;
    let kind = kind_of(fd.as_raw_fd())?.st_mode & libc::S_IFMT;
    let target = into.join(last);
    let mut budget = Budget { entries: 0, bytes: 0, out: Exported::default() };
    let done = match kind {
        // A name that is taken is refused here, before anything is made, so nothing is removed below.
        libc::S_IFREG => {
            let out = create_new(&target)?;
            copy_file(fd, out, &target, &mut budget)
        }
        libc::S_IFDIR => {
            std::fs::create_dir(&target).map_err(|e| format!("{} could not be made ({e})", target.display()))?;
            copy_dir(fd.as_raw_fd(), &target, 0, &mut budget)
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

    const MP4: &[u8] = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom";

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

    #[test]
    fn a_media_file_is_opened_and_named_by_its_bytes() {
        let (_h, shop) = shop();
        std::fs::write(shop.join("episode.mp4"), MP4).unwrap();
        std::fs::create_dir(shop.join("out")).unwrap();
        std::fs::write(shop.join("out/song"), b"OggS\0\x02").unwrap();
        assert_eq!(checked_media(&shop, Path::new("episode.mp4")).unwrap().1, "video/mp4");
        assert_eq!(checked_media(&shop, &shop.join("out/song")).unwrap().1, "audio/ogg");
        assert_eq!(sniff(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(sniff(b"#!/bin/sh\nrm -rf"), None);
    }

    #[test]
    fn present_refuses_a_link_a_pipe_a_non_media_file_and_a_path_outside() {
        let (h, shop) = shop();
        let outside = h.path().join("outside.mp4");
        std::fs::write(&outside, MP4).unwrap();
        std::fs::write(shop.join("notes.mp4"), b"just text, named like a video").unwrap();
        std::fs::write(shop.join("real.mp4"), MP4).unwrap();
        symlink(&outside, shop.join("link.mp4")).unwrap();
        symlink(shop.join("real.mp4"), shop.join("inner-link.mp4")).unwrap();
        symlink(h.path(), shop.join("dir-link")).unwrap();
        fifo(&shop.join("pipe.mp4"));
        std::fs::create_dir(shop.join("folder")).unwrap();
        for bad in ["link.mp4", "inner-link.mp4", "dir-link/outside.mp4", "pipe.mp4", "notes.mp4", "folder", "missing.mp4", "", "../outside.mp4", "./real.mp4"] {
            assert!(checked_media(&shop, Path::new(bad)).is_err(), "{bad:?} must be refused");
        }
        for bad in [outside.to_str().unwrap(), "/etc/passwd", "/"] {
            let why = checked_media(&shop, Path::new(bad)).unwrap_err();
            assert!(why.contains("outside"), "{bad}: {why}");
        }
        // A sibling that merely shares the workshop's prefix.
        std::fs::create_dir(h.path().join("shop2")).unwrap();
        std::fs::write(h.path().join("shop2/x.mp4"), MP4).unwrap();
        assert!(checked_media(&shop, &shop.parent().unwrap().join("shop2/x.mp4")).is_err());
        assert!(checked_media(&shop, Path::new("real.mp4")).is_ok());
    }

    #[test]
    fn the_player_is_told_whose_workshop_it_is() {
        assert_eq!(mind_name("hermes:main"), "Hermes");
        assert_eq!(mind_name("pi"), "Pi");
        assert_eq!(mind_name("\u{202E}x\ny"), "Xy");
        assert_eq!(mind_name(""), "A Mind");
    }

    #[test]
    fn export_copies_regular_files_by_content_and_nothing_else() {
        let (h, shop) = shop();
        let secret = h.path().join("secret");
        std::fs::write(&secret, "host secret").unwrap();
        let src = shop.join("pkg");
        std::fs::create_dir_all(src.join("sub/deep")).unwrap();
        std::fs::create_dir_all(src.join(".git/hooks")).unwrap();
        std::fs::write(src.join(".git/config"), "[core]\nfsmonitor = evil").unwrap();
        std::fs::write(src.join(".git/hooks/pre-commit"), "#!/bin/sh").unwrap();
        std::fs::write(src.join("a.txt"), "alpha").unwrap();
        std::fs::write(src.join("sub/deep/b.txt"), "beta").unwrap();
        std::fs::write(src.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(src.join("run.sh"), std::fs::Permissions::from_mode(0o4755)).unwrap();
        symlink(&secret, src.join("leak")).unwrap();
        symlink(h.path(), src.join("dir-leak")).unwrap();
        fifo(&src.join("pipe"));
        // An extended attribute, where the filesystem takes one.
        let name = CString::new("user.mark").unwrap();
        let path = CString::new(src.join("a.txt").as_os_str().as_bytes()).unwrap();
        let tagged = unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), b"1".as_ptr().cast(), 1, 0) } == 0;
        let dest = h.path().join("Videos");
        std::fs::create_dir(&dest).unwrap();
        let (at, done) = export(&shop, Path::new("pkg"), &dest).unwrap();
        assert_eq!(at, dest.join("pkg"));
        assert_eq!(done, Exported { files: 3, bytes: 5 + 4 + 18 });
        assert_eq!(std::fs::read_to_string(at.join("a.txt")).unwrap(), "alpha");
        assert_eq!(std::fs::read_to_string(at.join("sub/deep/b.txt")).unwrap(), "beta");
        for gone in [".git", "leak", "dir-leak", "pipe"] {
            assert!(!at.join(gone).exists() && at.join(gone).symlink_metadata().is_err(), "{gone} must not be copied");
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
    fn export_refuses_links_pipes_git_outside_paths_and_a_missing_folder() {
        let (h, shop) = shop();
        let dest = h.path().join("Videos");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(h.path().join("outside.txt"), "x").unwrap();
        symlink(h.path().join("outside.txt"), shop.join("link.txt")).unwrap();
        symlink(h.path(), shop.join("dir-link")).unwrap();
        fifo(&shop.join("pipe"));
        std::fs::create_dir_all(shop.join(".git")).unwrap();
        for bad in ["link.txt", "dir-link", "dir-link/outside.txt", "pipe", ".git", "../outside.txt", "missing", "", "/etc/passwd"] {
            assert!(export(&shop, Path::new(bad), &dest).is_err(), "{bad:?}");
        }
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0, "nothing landed");
        std::fs::write(shop.join("ok.txt"), "ok").unwrap();
        assert!(export(&shop, Path::new("ok.txt"), &dest.join("nowhere")).is_err());
        assert!(export(&shop, Path::new("ok.txt"), &dest).is_ok());
    }
}
