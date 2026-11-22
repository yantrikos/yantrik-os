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
//! - **staged where nothing else can reach** (round 10): in a `0700` folder of the shell's, outside
//!   the scripts folder and in no view or sandbox, in a folder of its own per call. Every write is
//!   descriptor-relative with `O_NOFOLLOW` (`coder_dirfd`): a link planted on the way refuses;
//! - **atomic**: the complete new folder (the old one's entries that the stream does not replace,
//!   hard-linked, plus the new ones) is built in that staging folder, then swapped in with one
//!   `renameat2(RENAME_EXCHANGE)` relative to a descriptor of the target's parent (or two renames
//!   where the file system has none). A refusal or a failure at any point leaves the old folder
//!   whole. The replaced folder is kept beside the target as `<name>.replaced-<date>-...`; nothing
//!   of the user's is deleted (only the shell's own staging folder).

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::coder_dirfd::{carry, Dir};

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

/// The longest a call waits for the staging root's lock (another install's swap, a script's places).
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// A per-call folder this old that an earlier crash left (a day).
const STALE_SECS: u64 = 24 * 3600;

/// Test-only switches for the paths a real file system does not reach on demand (`cfg(test)` only).
#[cfg(test)]
pub(crate) mod hook {
    use std::cell::Cell;
    pub const NO_EXCHANGE: u8 = 1;
    pub const FAIL_ARCHIVE: u8 = 2;
    pub const FAIL_BACK: u8 = 4;
    pub const FULL_DISK: u8 = 8;
    thread_local!(static FLAGS: Cell<u8> = const { Cell::new(0) });
    pub fn set(flags: u8) {
        FLAGS.with(|f| f.set(flags));
    }
    pub fn on(flag: u8) -> bool {
        FLAGS.with(|f| f.get() & flag != 0)
    }
}
#[cfg(not(test))]
mod hook {
    pub const NO_EXCHANGE: u8 = 1;
    pub const FAIL_ARCHIVE: u8 = 2;
    pub const FAIL_BACK: u8 = 4;
    pub const FULL_DISK: u8 = 8;
    pub fn on(_: u8) -> bool {
        false
    }
}

/// The shell's own staging folder: made `0700` when missing, never a link, the account's own and
/// closed to others, or the refusal.
pub(crate) fn staging_root(stage_root: &Path) -> Result<Dir, String> {
    if let Some(up) = stage_root.parent() {
        std::fs::create_dir_all(up).map_err(|e| e.to_string())?;
    }
    if let Err(e) = std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700).create(stage_root) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(e.to_string());
        }
    }
    let dir = Dir::open(stage_root)?;
    if !dir.is_private() {
        return Err(format!("{} is not private to this account", stage_root.display()));
    }
    Ok(dir)
}

/// Make the plain folders `names` in `parent` when missing, under the staging root's lock, so a swap
/// in progress is never met half done and a folder is never made in its window. Through descriptors:
/// a link where a folder belongs refuses.
pub(crate) fn ensure_folders(parent: &Path, names: &[&str], stage_root: &Path) -> Result<(), String> {
    // Check first, with no lock: when every folder is already a plain folder, nothing is made, so a
    // slow install never delays a script start. The lock is taken only to make a missing one.
    if Dir::open(parent).is_ok_and(|dir| names.iter().all(|n| dir.mode_of(OsStr::new(n)).is_ok_and(|m| m.is_some_and(|m| m & libc::S_IFMT == libc::S_IFDIR)))) {
        return Ok(());
    }
    let stage = staging_root(stage_root)?;
    let _lock = stage.lock(LOCK_WAIT)?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let dir = Dir::open(parent)?;
    for n in names {
        dir.mkdir(OsStr::new(n), 0o755)?;
        dir.child(OsStr::new(n))?;
    }
    Ok(())
}

/// Clear what a crash left in the staging root, under the lock (so nothing in it is live): a per-call
/// folder older than a day loses its `in` (links and copies of the stream), and goes when it holds no
/// `whole`. One that holds `whole` may hold the only old copy of a folder, so it stays.
fn sweep_stale(stage: &Dir, stage_root: &Path) {
    for n in stage.names().unwrap_or_default() {
        if !n.to_string_lossy().contains(".call-") || stage.age_secs(&n).is_none_or(|a| a < STALE_SECS) {
            continue;
        }
        let folder = stage_root.join(&n);
        let _ = std::fs::remove_dir_all(folder.join("in"));
        if stage.child(&n).and_then(|d| d.mode_of(OsStr::new("whole"))).is_ok_and(|m| m.is_none()) {
            let _ = std::fs::remove_dir_all(&folder);
        }
    }
}

/// Write `stream` (a ustar archive), judged as above, into a folder of its own in `stage_root`, build
/// the whole new `dest` there from it and the old one's other entries, and swap it in at once.
/// `stage_root` is a `0700` folder of the shell's on the same file system as `dest`. Only one call
/// swaps at a time (a lock on the staging root); the per-call folder is made strictly. Answers how
/// many files were written, or why nothing was.
pub fn promote(stream: &[u8], dest: &Path, stage_root: &Path) -> Result<usize, String> {
    let refuse = |why: String| format!("the install result was refused: {why}");
    let parent = dest.parent().ok_or_else(|| refuse("no folder to put it in".into()))?;
    let leaf = dest.file_name().ok_or_else(|| refuse("no name to put it under".into()))?;
    std::fs::create_dir_all(parent).map_err(|e| refuse(e.to_string()))?;
    let parent_dir = Dir::open(parent).map_err(refuse)?;
    if parent_dir.mode_of(leaf).map_err(refuse)?.is_some_and(|m| m & libc::S_IFMT != libc::S_IFDIR) {
        return Err(refuse(format!("{} is not a plain folder", dest.display())));
    }
    let stage = staging_root(stage_root).map_err(refuse)?;
    let _lock = stage.lock(LOCK_WAIT).map_err(refuse)?;
    sweep_stale(&stage, stage_root);
    let call = unique("call", &leaf.to_string_lossy());
    stage.mkdir_new(OsStr::new(&call), 0o700).map_err(refuse)?;
    let mut keep = false;
    let done = stage.child(OsStr::new(&call)).and_then(|call_dir| {
        let n = write_all(stream, &call_dir)?;
        build_whole(&call_dir, &parent_dir, leaf)?;
        swap_in(&call_dir, &parent_dir, leaf, &stage_root.join(&call).join("whole"), &mut keep)?;
        Ok(n)
    });
    // Only the shell's own staging folder is removed, never the target or an archive, and never a
    // staging folder that holds the old copy.
    if !keep {
        let _ = std::fs::remove_dir_all(stage_root.join(&call));
    }
    done.map_err(refuse)
}

/// What the stream brings goes into `call/in`: written through descriptors, judged entry by entry.
fn write_all(stream: &[u8], call: &Dir) -> Result<usize, String> {
    call.mkdir(OsStr::new("in"), 0o700)?;
    let into = call.child(OsStr::new("in"))?;
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
                    into.mkdir_p(&rel, 0o755)?;
                }
            }
            b'0' | 0 => {
                let rel = rel.ok_or("a file with no name")?;
                bytes += size;
                let end = at.checked_add(size as usize).filter(|e| *e <= stream.len() && bytes <= MAX_BYTES).ok_or("it is cut short or too big")?;
                let folder = into.mkdir_p(rel.parent().unwrap_or(Path::new("")), 0o755)?;
                let mut f = folder.create(rel.file_name().unwrap_or_default(), file_mode(&h[100..108]))?;
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

/// The complete new folder at `call/whole`: what `leaf` of `parent` holds now, apart from the names
/// `call/in` brings (hard-linked, so nothing is copied twice), then those top-level entries moved in.
fn build_whole(call: &Dir, parent: &Dir, leaf: &OsStr) -> Result<(), String> {
    call.mkdir(OsStr::new("whole"), 0o755)?;
    let (whole, brought) = (call.child(OsStr::new("whole"))?, call.child(OsStr::new("in"))?);
    let replaced = brought.names()?;
    if parent.mode_of(leaf)?.is_some() {
        let old = parent.child(leaf)?;
        for n in old.names()?.into_iter().filter(|n| !replaced.contains(n)) {
            match old.mode_of(&n)?.map(|m| m & libc::S_IFMT) {
                Some(libc::S_IFDIR) => {
                    whole.mkdir(&n, 0o755)?;
                    carry(&old.child(&n)?, &whole.child(&n)?)?;
                }
                Some(libc::S_IFREG) => old.link_or_copy(&n, &whole)?,
                _ => {}
            }
        }
    }
    for n in replaced {
        brought.rename(&n, &whole, &n, false)?;
    }
    Ok(())
}

/// Put `call/whole` in place of `leaf` of `parent` in one step, and keep what was there beside it, dated.
/// `old_copy` is where the old folder is while it is in `call/whole`; `keep` is set when the staging
/// folder holds the only old copy and so must stay.
fn swap_in(call: &Dir, parent: &Dir, leaf: &OsStr, old_copy: &Path, keep: &mut bool) -> Result<(), String> {
    let whole = OsStr::new("whole");
    if parent.mode_of(leaf)?.is_none() {
        return call.rename(whole, parent, leaf, false);
    }
    let archive = format!("{}.replaced-{}-{}-{}", leaf.to_string_lossy(), chrono::Local::now().format("%Y%m%d"), std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let archive = OsStr::new(&archive);
    let exchanged = if hook::on(hook::NO_EXCHANGE) {
        Err((libc::EINVAL, "no exchange".to_string()))
    } else if hook::on(hook::FULL_DISK) {
        Err((libc::ENOSPC, "no space left on device".to_string()))
    } else {
        call.exchange(whole, parent, leaf)
    };
    match exchanged {
        Ok(()) => {
            // `call/whole` now holds the old folder.
            let aside = if hook::on(hook::FAIL_ARCHIVE) { Err("the archive could not be made".to_string()) } else { call.rename(whole, parent, archive, false) };
            let Err(why) = aside else { return Ok(()) };
            // The old folder could not be kept aside: trade back, so it is in place again and the failure is told as one.
            let back = if hook::on(hook::FAIL_BACK) { Err("could not exchange back".to_string()) } else { call.exchange(whole, parent, leaf).map_err(|e| e.1) };
            match back {
                Ok(()) => Err(format!("the old folder could not be kept aside ({why}); it is back in place and nothing was changed")),
                Err(back) => {
                    *keep = true;
                    // Where the old copy is goes to the log, for the person; the Mind is not told a path of the shell's.
                    tracing::error!(old_copy = %old_copy.display(), "A dependencies folder was replaced and its old copy could not be put back or kept aside: it stays in the shell's staging folder");
                    Err(format!("the old folder could not be kept aside ({why}) and could not be put back ({back}); the new folder is in place and the old copy was kept in the shell's own staging folder (the person has where, in the log)"))
                }
            }
        }
        // No exchange on this file system: the old folder aside, then the new one in (the lock keeps others out).
        Err((errno, _)) if errno == libc::EINVAL || errno == libc::ENOSYS => {
            parent.rename(leaf, parent, archive, false)?;
            call.rename(whole, parent, leaf, false).inspect_err(|_| {
                let _ = parent.rename(archive, parent, leaf, false);
            })
        }
        // Any other refusal (a full disk, a permission) leaves the old folder whole.
        Err((_, why)) => Err(why),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// One staging folder for the tests of this process, as the shell's own `deps-staging` is.
    fn stage() -> PathBuf {
        std::env::temp_dir().join(format!("yantrik-promote-stage-{}", std::process::id()))
    }

    fn promote(stream: &[u8], dest: &Path) -> Result<usize, String> {
        super::promote(stream, dest, &stage())
    }

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

    /// Round 10 A: a link where the target's parent, the target or the staging folder should be is
    /// refused, and nothing is written through it.
    #[test]
    fn a_link_in_the_target_the_parent_or_the_staging_path_is_refused() {
        let top = scratch("links");
        let outside = top.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        // the parent of the target is a link
        std::os::unix::fs::symlink(&outside, top.join("parent")).unwrap();
        assert!(promote(&tar(&[("a", b'0', "1")]), &top.join("parent/.deps")).is_err());
        // the staging folder is a link
        let real = top.join("real-stage");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, top.join("stage-link")).unwrap();
        let why = super::promote(&tar(&[("a", b'0', "1")]), &top.join(".deps"), &top.join("stage-link")).unwrap_err();
        assert!(why.contains("refused"), "{why}");
        // the staging folder is open to others
        use std::os::unix::fs::PermissionsExt;
        let open = top.join("open-stage");
        std::fs::create_dir_all(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(super::promote(&tar(&[("a", b'0', "1")]), &top.join(".deps"), &open).unwrap_err().contains("not private"));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count() + std::fs::read_dir(&real).unwrap().count() + std::fs::read_dir(&open).unwrap().count(), 0, "nothing written through a link");
        assert!(!top.join(".deps").exists());
        let _ = std::fs::remove_dir_all(top);
    }

    /// Round 10 A: a folder of the staging path swapped for a link after the descriptor walk began is
    /// refused (`O_NOFOLLOW`), and the file is not made where the link points.
    #[test]
    fn a_link_planted_mid_write_is_refused() {
        let top = scratch("mid");
        let (outside, work) = (top.join("outside"), top.join("work"));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(work.join("d")).unwrap();
        let held = Dir::open(&work).unwrap();
        held.mkdir_p(Path::new("d/e"), 0o755).unwrap();
        // A script (or anything) replaces `d` with a link to somewhere else, mid-install.
        std::fs::remove_dir_all(work.join("d")).unwrap();
        std::os::unix::fs::symlink(&outside, work.join("d")).unwrap();
        assert!(held.mkdir_p(Path::new("d/e"), 0o755).is_err(), "a folder step through a link");
        assert!(held.child(OsStr::new("d")).is_err());
        assert!(held.create(OsStr::new("d"), 0o644).is_err(), "a file over a link");
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing was made where the link points");
        // and a link as a top-level entry of the old target is not carried into the new folder
        let dest = top.join(".deps");
        std::fs::create_dir_all(&dest).unwrap();
        std::os::unix::fs::symlink(&outside, dest.join("sneaky")).unwrap();
        std::fs::write(dest.join("keep"), "k").unwrap();
        promote(&tar(&[("new", b'0', "n")]), &dest).unwrap();
        assert!(dest.join("keep").exists() && dest.join("new").exists() && dest.join("sneaky").symlink_metadata().is_err());
        let _ = std::fs::remove_dir_all(top);
    }

    /// Round 10 A: each call stages in a `0700` folder of its own in the staging root, and leaves nothing there.
    #[test]
    fn the_staging_folder_is_private_and_left_empty() {
        use std::os::unix::fs::PermissionsExt;
        let top = scratch("stage");
        let stage = top.join("deps-staging");
        promote_with(&tar(&[("a", b'0', "1")]), &top.join(".deps"), &stage);
        promote_with(&tar(&[("b", b'0', "2")]), &top.join(".deps"), &stage);
        assert_eq!(std::fs::metadata(&stage).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::read_dir(&stage).unwrap().count(), 0, "no call's folder is left");
        let _ = std::fs::remove_dir_all(top);
    }

    fn promote_with(t: &[u8], dest: &Path, stage: &Path) {
        super::promote(t, dest, stage).unwrap();
    }
}
