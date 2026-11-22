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
//! - **plain modes** (0644 files, 0755 folders), never over a name the stream made twice;
//! - built in a fresh folder beside the target, and moved into place only when it is whole, so a
//!   refused install leaves the target as it was.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

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

/// Write `stream` (a ustar archive) under a fresh folder beside `dest`, judged as above, and move
/// each of its top-level entries into `dest`, replacing the same name there. Answers how many
/// files were written, or why nothing was.
pub fn promote(stream: &[u8], dest: &Path) -> Result<usize, String> {
    let refuse = |why: String| format!("the install result was refused: {why}");
    let parent = dest.parent().ok_or_else(|| refuse("no folder to put it in".into()))?;
    std::fs::create_dir_all(parent).map_err(|e| refuse(e.to_string()))?;
    let leaf = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staging = parent.join(format!(".{leaf}.staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::DirBuilder::new().mode(0o755).create(&staging).map_err(|e| refuse(e.to_string()))?;
    let done = write_all(stream, &staging).and_then(|n| move_in(&staging, dest).map(|_| n));
    let _ = std::fs::remove_dir_all(&staging);
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
                    .mode(0o644)
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

fn move_in(staging: &Path, dest: &Path) -> Result<(), String> {
    if dest.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
        return Err(format!("{} is not a plain folder", dest.display()));
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(staging).map_err(|e| e.to_string())?.flatten() {
        let to = dest.join(entry.file_name());
        match to.symlink_metadata() {
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&to),
            Ok(_) => std::fs::remove_file(&to),
            Err(_) => Ok(()),
        }
        .map_err(|e| e.to_string())?;
        std::fs::rename(entry.path(), &to).map_err(|e| e.to_string())?;
    }
    Ok(())
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
        assert_eq!(std::fs::read_dir(&top).unwrap().count(), 1, "no staging folder is left");
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
}
