//! What leaves a Mind's workshop, and the only two ways it can (#1120, #1124).
//!
//! What a Mind builds in a workshop can hold anything a program reads or runs, so nothing outside
//! the two mount views touches it. Bytes leave two ways:
//!
//! - **`present`**: the file is opened here, `O_RDONLY|O_NOFOLLOW`, and must be a regular file of a
//!   media type its first bytes name (not its extension); the descriptor, not the path, goes to the
//!   OS's own player, which says whose workshop it came from. No card: a decoder reads bytes and
//!   runs no hooks.
//! - **`export`** (`workshop_export.rs`): a copy of regular files by content, carded by the
//!   caller. No repository, no link, no device or pipe, no mode bits, no extended attributes: what
//!   lands is plain bytes the person chose to take out.
//!
//! The player runs under `bwrap` with no network and no home (`workshop_play.rs`), and its demuxer
//! is the one the sniffed type names (`demuxer`), so the file is read as the format it was
//! admitted as and as nothing else.
//!
//! Both walk from a descriptor for the workshop and open each name with `O_NOFOLLOW`, so a link a
//! sealed job swaps in while this runs is never followed.

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

pub(crate) fn cstr(name: &std::ffi::OsStr) -> Result<CString, String> {
    CString::new(name.as_bytes()).map_err(|_| "a name with a NUL in it".to_string())
}

pub(crate) fn own(fd: RawFd) -> Result<OwnedFd, String> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The directory `workshop`, opened without following a link.
pub(crate) fn open_workshop(workshop: &Path) -> Result<OwnedFd, String> {
    let c = cstr(workshop.as_os_str())?;
    own(unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })
        .map_err(|e| format!("the workshop cannot be opened ({e})"))
}

/// `path` as names below `workshop`: relative, or absolute and inside it. No `..`, no `.`.
pub(crate) fn below(workshop: &Path, path: &Path) -> Result<Vec<std::ffi::OsString>, String> {
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
pub(crate) fn open_below(dir: RawFd, names: &[std::ffi::OsString], last: i32) -> Result<OwnedFd, String> {
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

pub(crate) fn kind_of(fd: RawFd) -> Result<libc::stat, String> {
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

/// The libavformat demuxer a sniffed type is read with, and no other: the player is told this
/// name, so a file that starts like an MP3 or an MP4 is never probed against the playlist, HLS or
/// reference formats that fetch or open other things.
pub fn demuxer(media: &str) -> &'static str {
    match media {
        "video/mp4" => "mp4",
        "video/webm" => "matroska",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "image/webp" => "webp_pipe",
        "video/avi" => "avi",
        "audio/flac" => "flac",
        "audio/mpeg" => "mp3",
        "image/png" => "png_pipe",
        "image/jpeg" => "jpeg_pipe",
        "image/gif" => "gif",
        // Unreachable for what `sniff` names; any other type gets a name no demuxer has, so
        // the player refuses the file.
        _ => "none",
    }
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

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
}
