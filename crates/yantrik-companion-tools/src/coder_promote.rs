//! The shell's side of a dependency install: stage and promote (#1265, round 8).
//!
//! An install runs in the outside view with its working directory and target in the job's own
//! `/tmp`, which no other command and no later script can see, and prints the result as a tar
//! stream on its standard output. The dependencies folder is never visible or writable to the job.
//! This module, which runs in the shell and outside the view, reads that stream with a strict
//! reader and writes the result as plain files, the way `workshop_export` copies out:
//!
//! - **only a regular file or a folder**: a link of either kind, a device, a pipe, a pax or GNU
//!   header, an unknown kind, anything else refuses the whole install;
//! - **plain names**: no absolute path, no `..`, no empty or NUL part; a name too long for a plain
//!   header refuses;
//! - **a size limit** on the stream, the bytes written and the entries;
//! - **plain modes** (0644 files, 0755 folders and executables; an executable is a regular file
//!   whose header has the owner's execute bit; setuid, setgid and sticky are never kept), never over
//!   a name the stream made twice;
//! - **atomic**: the complete new folder (the old one's entries that the stream does not replace,
//!   hard-linked, plus the new ones) is built in a fresh folder with a name of its own beside the
//!   target, then swapped in with one `renameat2(RENAME_EXCHANGE)` (or two renames where the file
//!   system has none). A refusal or a failure at any point leaves the old folder whole. The replaced
//!   folder is kept beside the target as `<name>.replaced-<date>-...`; nothing is deleted.

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Makes a name of its own for each call, together with the process id.
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str, leaf: &str) -> String {
    format!(".{leaf}.{tag}-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// The most bytes of file content, and of stream, an install may carry.
pub const MAX_BYTES: u64 = 256 << 20;
/// The most entries.
const MAX_ENTRIES: usize = 50_000;
/// The deepest folder.
const MAX_DEPTH: usize = 32;

fn octal(field: &[u8]) -> Option<u64> {
    let text: String = field.iter().take_while(|b| **b != 0).map(|b| *b as char).collect();
    u64::from_str_radix(text.trim(), 8).ok().or(if text.trim().is_empty() { Some(0) } else { None })
}

/// 0755 for a regular file whose header has the owner's execute bit, else 0644; never setuid,
/// setgid or sticky.
fn file_mode(field: &[u8]) -> u32 {
    if octal(field).is_some_and(|m| m & 0o100 != 0) { 0o755 } else { 0o644 }
}

fn cstr(field: &[u8]) -> Option<&str> {
    std::str::from_utf8(&field[..field.iter().position(|b| *b == 0).unwrap_or(field.len())]).ok()
}

/// The plain relative path a header names (`None` for the root `.`), or the refusal.
fn plain(name: &str) -> Result<Option<PathBuf>, String> {
    let mut out = PathBuf::new();
    for part in Path::new(name).components() {
        match part {
            Component::CurDir => {}
            Component::Normal(p) if !p.is_empty() => out.push(p),
            _ => return Err(format!("a name that is not plain ({name:?})")),
        }
    }
    match out.components().count() {
        0 => Ok(None),
        n if n > MAX_DEPTH => Err(format!("a place too deep ({name:?})")),
        _ => Ok(Some(out)),
    }
}

/// Write `stream` (a ustar archive) under a fresh folder beside `dest`, judged as above, build the
/// whole new `dest` from it and the old one's other entries, and swap it in at once. Answers how many
/// files were written, or why nothing was.
pub fn promote(stream: &[u8], dest: &Path) -> Result<usize, String> {
    let refuse = |why: String| format!("the install result was refused: {why}");
    let parent = dest.parent().ok_or_else(|| refuse("no folder to put it in".into()))?;
    std::fs::create_dir_all(parent).map_err(|e| refuse(e.to_string()))?;
    if dest.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
        return Err(refuse(format!("{} is not a plain folder", dest.display())));
    }
    let leaf = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staging = parent.join(unique("staging", &leaf));
    let whole = parent.join(unique("new", &leaf));
    std::fs::DirBuilder::new().mode(0o755).create(&staging).map_err(|e| refuse(e.to_string()))?;
    let done = write_all(stream, &staging).and_then(|n| build_whole(&staging, dest, &whole).map(|_| n)).and_then(|n| swap_in(&whole, dest, &leaf).map(|_| n));
    let _ = std::fs::remove_dir_all(&staging);
    if done.is_err() {
        // Only our own half-built folder, never the target or an archive.
        let _ = std::fs::remove_dir_all(&whole);
    }
    done.map_err(refuse)
}

fn write_all(stream: &[u8], into: &Path) -> Result<usize, String> {
    if stream.len() as u64 > MAX_BYTES {
        return Err("it is too big".into());
    }
    let (mut at, mut files, mut entries, mut bytes) = (0usize, 0usize, 0usize, 0u64);
    while at + 512 <= stream.len() {
        let h = &stream[at..at + 512];
        if h.iter().all(|b| *b == 0) {
            return Ok(files);
        }
        entries += 1;
        if entries > MAX_ENTRIES {
            return Err("too many entries".into());
        }
        let sum: u64 = h.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u64 }).sum();
        if octal(&h[148..156]) != Some(sum) || &h[257..262] != b"ustar" {
            return Err("it is not a plain archive".into());
        }
        let (name, prefix) = (cstr(&h[0..100]).ok_or("a name that is not text")?, cstr(&h[345..500]).ok_or("a name that is not text")?);
        let full = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        let size = octal(&h[124..136]).ok_or("a size that is not a number")?;
        let rel = plain(&full)?;
        at += 512;
        match h[156] {
            b'5' => {
                if let Some(rel) = rel {
                    std::fs::create_dir_all(into.join(rel)).map_err(|e| e.to_string())?;
                }
            }
            b'0' | 0 => {
                let rel = rel.ok_or("a file with no name")?;
                bytes += size;
                let end = at.checked_add(size as usize).filter(|e| *e <= stream.len() && bytes <= MAX_BYTES).ok_or("it is cut short or too big")?;
                let path = into.join(&rel);
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                // `create_new` (`O_EXCL`) never opens a link and never writes over a name made before.
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(file_mode(&h[100..108]))
                    .open(&path)
                    .map_err(|e| format!("{}: {e}", rel.display()))?;
                f.write_all(&stream[at..end]).map_err(|e| e.to_string())?;
                files += 1;
                at += (size as usize).div_ceil(512) * 512;
            }
            other => return Err(format!("{full:?} is not a plain file or folder (kind {:?})", other as char)),
        }
    }
    // No end block: the stream was cut short.
    Err("it is cut short".into())
}

/// The complete new folder at `whole`: what `dest` holds now, apart from the names `staging` brings
/// (hard-linked, so nothing is copied twice), then the top-level entries of `staging` renamed in.
fn build_whole(staging: &Path, dest: &Path, whole: &Path) -> Result<(), String> {
    std::fs::DirBuilder::new().mode(0o755).create(whole).map_err(|e| e.to_string())?;
    let replaced: Vec<OsString> = std::fs::read_dir(staging).map_err(|e| e.to_string())?.flatten().map(|e| e.file_name()).collect();
    if dest.is_dir() {
        for entry in std::fs::read_dir(dest).map_err(|e| e.to_string())?.flatten() {
            if !replaced.contains(&entry.file_name()) {
                carry(&entry.path(), &whole.join(entry.file_name()))?;
            }
        }
    }
    for name in replaced {
        std::fs::rename(staging.join(&name), whole.join(&name)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Hard-link a file, or make a folder and carry its entries; a link is not carried.
fn carry(from: &Path, to: &Path) -> Result<(), String> {
    let kind = from.symlink_metadata().map_err(|e| e.to_string())?;
    if kind.is_dir() {
        std::fs::DirBuilder::new().mode(0o755).create(to).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
            carry(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if kind.is_file() {
        std::fs::hard_link(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ())).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Put `whole` in place of `dest` in one step, and keep what was there beside it, dated.
fn swap_in(whole: &Path, dest: &Path, leaf: &str) -> Result<(), String> {
    if !dest.exists() {
        return std::fs::rename(whole, dest).map_err(|e| e.to_string());
    }
    let parent = dest.parent().unwrap_or(Path::new("."));
    let archive = parent.join(format!("{leaf}.replaced-{}-{}-{}", chrono::Local::now().format("%Y%m%d"), std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    if exchange(whole, dest).is_ok() {
        // `whole` now holds the old folder.
        return std::fs::rename(whole, &archive).map_err(|e| format!("the new folder is in place, but the old one could not be kept aside: {e}"));
    }
    std::fs::rename(dest, &archive).map_err(|e| e.to_string())?;
    std::fs::rename(whole, dest).map_err(|e| {
        let _ = std::fs::rename(&archive, dest);
        e.to_string()
    })
}

/// `renameat2(RENAME_EXCHANGE)`: the two folders trade places in one step.
fn exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput));
    let (a, b) = (c(a)?, c(b)?);
    // SAFETY: both are valid NUL-terminated paths for the call; the syscall reads them only.
    let r = unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_EXCHANGE) };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn header(name: &str, kind: u8, size: usize) -> [u8; 512] {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000644");
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[148..156].fill(32);
        let sum: u64 = h.iter().map(|b| *b as u64).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        h
    }

    fn tar(entries: &[(&str, u8, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, kind, body) in entries {
            out.extend(header(name, *kind, body.len()));
            out.extend(body.as_bytes());
            out.extend(vec![0u8; (512 - body.len() % 512) % 512]);
        }
        out.extend(vec![0u8; 1024]);
        out
    }

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("yantrik-promote-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn plain_files_arrive_as_plain_files_and_replace_what_was_there() {
        let top = scratch("ok");
        let dest = top.join(".deps");
        std::fs::create_dir_all(dest.join("pkg")).unwrap();
        std::fs::write(dest.join("pkg/old.py"), "old").unwrap();
        std::fs::write(dest.join("keep.txt"), "kept").unwrap();
        let t = tar(&[("./", b'5', ""), ("./pkg/", b'5', ""), ("./pkg/new.py", b'0', "x = 1\n"), ("./other/mod.py", b'0', "y = 2\n")]);
        assert_eq!(promote(&t, &dest).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(dest.join("pkg/new.py")).unwrap(), "x = 1\n");
        assert!(!dest.join("pkg/old.py").exists(), "the same top-level name is replaced");
        assert!(dest.join("keep.txt").exists());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dest.join("other/mod.py")).unwrap().permissions().mode() & 0o7777, 0o644);
        // No staging or half-built folder is left; the replaced folder is kept beside it, dated.
        let left: Vec<String> = std::fs::read_dir(&top).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.len(), 2, "{left:?}");
        let archive = left.iter().find(|n| n.starts_with(".deps.replaced-")).expect("the old folder is kept");
        assert!(archive.contains(&chrono::Local::now().format("%Y%m%d").to_string()), "{archive}");
        assert_eq!(std::fs::read_to_string(top.join(archive).join("pkg/old.py")).unwrap(), "old", "nothing of the old folder is deleted");
        let _ = std::fs::remove_dir_all(top);
    }

    #[test]
    fn anything_but_a_file_or_a_folder_refuses_and_leaves_the_target() {
        for (kind, name) in [(b'2', "link"), (b'1', "hard"), (b'3', "dev"), (b'6', "fifo"), (b'x', "pax"), (b'L', "long")] {
            let top = scratch("kind");
            let dest = top.join(".deps");
            std::fs::create_dir_all(&dest).unwrap();
            std::fs::write(dest.join("a"), "a").unwrap();
            let t = tar(&[("ok.py", b'0', "1"), (name, kind, "")]);
            let why = promote(&t, &dest).unwrap_err();
            assert!(why.contains("refused"), "{why}");
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 1, "{name}: the target is as it was");
            assert_eq!(std::fs::read_dir(&top).unwrap().count(), 1, "{name}: no staging folder is left");
            let _ = std::fs::remove_dir_all(top);
        }
    }

    #[test]
    fn a_name_that_is_not_plain_refuses() {
        for bad in ["/etc/x", "../x", "a/../../x", "a//b/../../../x"] {
            let top = scratch("name");
            let t = tar(&[(bad, b'0', "1")]);
            assert!(promote(&t, &top.join(".deps")).is_err(), "{bad}");
            assert!(!top.join("x").exists() && !top.parent().unwrap().join("x").exists());
            let _ = std::fs::remove_dir_all(top);
        }
    }

    #[test]
    fn a_file_over_a_name_made_before_a_cut_stream_and_a_big_one_refuse() {
        let top = scratch("dup");
        let t = tar(&[("a", b'0', "1"), ("a", b'0', "2")]);
        assert!(promote(&t, &top.join(".deps")).is_err());
        let cut = tar(&[("a", b'0', "123")]);
        assert!(promote(&cut[..cut.len() - 1024], &top.join(".deps")).is_err(), "no end block");
        assert!(promote(&cut[..600], &top.join(".deps")).is_err());
        let mut big = header("a", b'0', (MAX_BYTES + 1) as usize).to_vec();
        big.extend(vec![0u8; 1024]);
        assert!(promote(&big, &top.join(".deps")).is_err(), "a size over the limit");
        assert!(promote(b"not a tar at all", &top.join(".deps")).is_err());
        assert!(!top.join(".deps/a").exists());
        let _ = std::fs::remove_dir_all(top);
    }

    #[test]
    fn a_target_that_is_a_link_refuses() {
        let top = scratch("link");
        let real = top.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, top.join(".deps")).unwrap();
        assert!(promote(&tar(&[("a", b'0', "1")]), &top.join(".deps")).is_err());
        assert!(!real.join("a").exists());
        let _ = std::fs::remove_dir_all(top);
    }

    /// A header with a mode, for the execute-bit test.
    fn tar_mode(name: &str, mode: &str) -> Vec<u8> {
        let mut h = header(name, b'0', 1);
        h[100..107].copy_from_slice(mode.as_bytes());
        h[148..156].fill(32);
        let sum: u64 = h.iter().map(|b| *b as u64).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        let mut out = h.to_vec();
        out.extend(b"x");
        out.extend(vec![0u8; 511 + 1024]);
        out
    }

    /// Round 9 item 7: an executable regular file stays one (0755); setuid, setgid and sticky never are kept.
    #[test]
    fn the_execute_bit_is_kept_and_setuid_never() {
        use std::os::unix::fs::PermissionsExt;
        let top = scratch("exec");
        let dest = top.join(".deps");
        for (name, mode, want) in [("run", "0000755", 0o755), ("plain", "0000644", 0o644), ("suid", "0004755", 0o755), ("sgid", "0002750", 0o755), ("sticky", "0001777", 0o755), ("none", "0004600", 0o644)] {
            promote(&tar_mode(name, mode), &dest).unwrap();
            let got = std::fs::metadata(dest.join(name)).unwrap().permissions().mode();
            assert_eq!(got & 0o7777, want, "{name}: {got:o}");
        }
        let _ = std::fs::remove_dir_all(top);
    }

    /// Round 9 item 2: a refusal part-way (a file was already staged) leaves the old folder whole and
    /// nothing else beside it; two calls never share a staging name.
    #[test]
    fn a_failure_in_the_middle_leaves_the_old_folder_whole() {
        let top = scratch("whole");
        let dest = top.join(".deps");
        std::fs::create_dir_all(dest.join("pkg")).unwrap();
        std::fs::write(dest.join("pkg/a.py"), "a").unwrap();
        let before: Vec<_> = std::fs::read_dir(&top).unwrap().flatten().map(|e| e.file_name()).collect();
        let t = tar(&[("pkg/new.py", b'0', "n"), ("other/x.py", b'0', "x"), ("bad", b'2', "")]);
        assert!(promote(&t, &dest).is_err());
        assert_eq!(std::fs::read_to_string(dest.join("pkg/a.py")).unwrap(), "a");
        assert!(!dest.join("pkg/new.py").exists() && !dest.join("other").exists());
        assert_eq!(std::fs::read_dir(&top).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>(), before);
        assert_ne!(unique("staging", "x"), unique("staging", "x"));
        let _ = std::fs::remove_dir_all(top);
    }

    /// Entries the stream does not replace are carried into the new folder, so earlier installs stay.
    #[test]
    fn what_the_stream_does_not_replace_is_carried_over() {
        let top = scratch("carry");
        let dest = top.join("node_modules");
        std::fs::create_dir_all(dest.join("left-pad/lib")).unwrap();
        std::fs::write(dest.join("left-pad/lib/i.js"), "1").unwrap();
        promote(&tar(&[("other/i.js", b'0', "2")]), &dest).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("left-pad/lib/i.js")).unwrap(), "1");
        assert_eq!(std::fs::read_to_string(dest.join("other/i.js")).unwrap(), "2");
        let _ = std::fs::remove_dir_all(top);
    }
}
