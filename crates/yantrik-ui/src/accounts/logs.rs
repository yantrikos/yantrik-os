//! Reading a vendor program's own session logs, a little at a time.
//!
//! Claude Code and Codex each write a JSON line per event into files under their directory, and
//! those lines carry what the panel counts: tokens, and — for Codex — the plan's own meters. The
//! files grow to tens of megabytes, so they are read the way `tail -f` reads: each file from where
//! the last read stopped, whole lines only, and only files written since the day began.
//!
//! Bounded on every side, because anything running as the person can write these files and the
//! shell must not be the thing that falls over: how deep a directory is walked and how many
//! entries it visits, how many files are looked at, how many bytes one read takes from one file
//! and from all of them ([`Budget`]), how long a line may be before it is skipped unread — a line
//! that never ends is skipped too. A line is only handed on when it contains a word the caller
//! names, so the tool results and file contents that make up most of these logs are never parsed.
//!
//! Every file is opened with [`open_own`]: never through a link, and only a regular file with one
//! name that this account owns — so a link or a hard link to a sign-in file is never read.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How deep under a vendor's log directory files are looked for.
pub const DEPTH: usize = 5;
/// The most files one read looks at.
pub const MOST_FILES: usize = 400;
/// The most directory entries one walk visits, files and directories together.
pub const MOST_ENTRIES: usize = 20_000;
/// A line longer than this is a tool result or a pasted file, never a count: skipped unparsed.
pub const LONGEST_LINE: usize = 256 * 1024;
/// The most one read takes from one file, and from every file together. What is left is read on
/// the next tick, from where this one stopped.
pub const FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const TICK_BYTES: u64 = 64 * 1024 * 1024;

/// Open `path` for reading only if it is a regular file with one name, owned by this account —
/// checked on the file that was opened, not on the path, so nothing can be swapped in between.
/// With its length.
pub fn open_own(path: &Path) -> Option<(std::fs::File, u64)> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let f = options.open(path).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: getuid cannot fail.
        let me = unsafe { libc::getuid() };
        if meta.nlink() != 1 || meta.uid() != me {
            return None;
        }
    }
    Some((f, meta.len()))
}

/// Every `.jsonl` under `root`, at most `DEPTH` down, written at or after `since`, newest first.
/// Never through a link.
pub fn written_since(root: &Path, since: SystemTime) -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    let mut visited = 0;
    walk(root, since, DEPTH, &mut out, &mut visited);
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out.truncate(MOST_FILES);
    out
}

fn walk(dir: &Path, since: SystemTime, depth: usize, out: &mut Vec<(PathBuf, SystemTime)>, visited: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        *visited += 1;
        if *visited > MOST_ENTRIES || out.len() >= MOST_FILES * 4 {
            return;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            if depth > 0 {
                walk(&path, since, depth - 1, out, visited);
            }
        } else if ft.is_file() && path.extension().is_some_and(|x| x == "jsonl") {
            if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                if m >= since {
                    out.push((path, m));
                }
            }
        }
    }
}

/// How many bytes are left to read on this tick.
pub struct Budget(pub u64);

impl Budget {
    pub fn tick() -> Budget {
        Budget(TICK_BYTES)
    }
}

#[derive(Clone, Copy, Default)]
struct At {
    offset: u64,
    /// Inside a line that grew past `LONGEST_LINE` without ending: everything up to its newline
    /// is thrown away, however many reads that takes.
    skipping: bool,
}

/// Where each file was read to.
#[derive(Default)]
pub struct Tails {
    at: HashMap<PathBuf, At>,
}

impl Tails {
    /// Hand every whole line written to `path` since the last call, that contains `needle`, to
    /// `each`. A file that is shorter than where it was read to was replaced: read from the start.
    pub fn read(&mut self, path: &Path, needle: &str, budget: &mut Budget, mut each: impl FnMut(&str)) {
        if budget.0 == 0 {
            return;
        }
        let Some((mut f, len)) = open_own(path) else { return };
        let mut at = self.at.get(path).copied().unwrap_or_default();
        if at.offset > len {
            at = At::default();
        }
        if at.offset == len || f.seek(SeekFrom::Start(at.offset)).is_err() {
            self.at.insert(path.to_path_buf(), at);
            return;
        }
        let allowed = FILE_BYTES.min(budget.0);
        let mut taken = 0u64;
        let mut reader = BufReader::with_capacity(64 * 1024, f);
        let mut line = Vec::new();
        while taken < allowed {
            line.clear();
            let (n, ended) = match read_line_bounded(&mut reader, &mut line, allowed - taken) {
                Ok(r) => r,
                Err(_) => break,
            };
            if n == 0 {
                break;
            }
            taken += n as u64;
            if !ended {
                if at.skipping || line.len() > LONGEST_LINE {
                    // Too long to ever be a count: step past what was read, and past the rest of
                    // it when it comes.
                    at.offset += n as u64;
                    at.skipping = true;
                }
                // Otherwise a line still being written, or cut by the budget: read it whole next
                // time, from its start.
                break;
            }
            at.offset += n as u64;
            if at.skipping {
                at.skipping = false;
                continue;
            }
            if line.len() <= LONGEST_LINE {
                if let Ok(text) = std::str::from_utf8(&line) {
                    if text.contains(needle) {
                        each(text.trim_end());
                    }
                }
            }
        }
        budget.0 = budget.0.saturating_sub(taken);
        self.at.insert(path.to_path_buf(), at);
    }
}

/// One line, keeping at most `LONGEST_LINE + 1` bytes of it, consuming at most `most` bytes.
/// Answers with how many bytes were consumed and whether the line ended.
fn read_line_bounded(reader: &mut impl BufRead, buf: &mut Vec<u8>, most: u64) -> std::io::Result<(usize, bool)> {
    let mut consumed = 0usize;
    loop {
        if consumed as u64 >= most {
            return Ok((consumed, false));
        }
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok((consumed, false));
        }
        let limit = chunk.len().min((most - consumed as u64) as usize);
        let (take, done) = match chunk[..limit].iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (limit, false),
        };
        let room = (LONGEST_LINE + 1).saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..take.min(room)]);
        reader.consume(take);
        consumed += take;
        if done {
            return Ok((consumed, true));
        }
    }
}

/// An RFC 3339 time, as Unix seconds.
pub fn unix_of(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|t| t.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-logs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read_all(t: &mut Tails, p: &Path) -> Vec<String> {
        let mut got = Vec::new();
        t.read(p, "usage", &mut Budget::tick(), |l| got.push(l.to_string()));
        got
    }

    #[test]
    fn a_file_is_read_from_where_it_stopped_whole_lines_only() {
        let d = tmp("tail");
        let p = d.join("s.jsonl");
        std::fs::write(&p, "{\"usage\":1}\n{\"other\":2}\n{\"usage\":3").unwrap();
        let mut t = Tails::default();
        assert_eq!(read_all(&mut t, &p), ["{\"usage\":1}"], "the unfinished line waits");
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(b"}\n{\"usage\":4}\n").unwrap();
        assert_eq!(read_all(&mut t, &p), ["{\"usage\":3}", "{\"usage\":4}"]);
        assert!(read_all(&mut t, &p).is_empty(), "nothing is read twice");
        std::fs::write(&p, "{\"usage\":9}\n").unwrap();
        assert_eq!(read_all(&mut t, &p), ["{\"usage\":9}"], "a shorter file was replaced: from the start");
    }

    #[test]
    fn a_huge_line_is_consumed_and_skipped_and_the_next_one_still_read() {
        let d = tmp("huge");
        let p = d.join("s.jsonl");
        let big = format!("{{\"usage\":\"{}\"}}\n", "x".repeat(LONGEST_LINE * 2));
        std::fs::write(&p, format!("{big}{{\"usage\":2}}\n")).unwrap();
        assert_eq!(read_all(&mut Tails::default(), &p), ["{\"usage\":2}"]);
    }

    /// A file that is one line that never ends (a sparse file, say) is stepped through once, not
    /// read from its start on every tick, and what comes after its end is still read.
    #[test]
    fn a_line_that_never_ends_is_skipped_across_reads_and_within_a_budget() {
        let d = tmp("endless");
        let p = d.join("s.jsonl");
        std::fs::write(&p, "x".repeat(LONGEST_LINE * 3)).unwrap();
        let mut t = Tails::default();
        let mut b = Budget(LONGEST_LINE as u64 * 2);
        t.read(&p, "usage", &mut b, |_| panic!("nothing to hand on"));
        assert_eq!(b.0, 0, "the read stopped at its budget");
        t.read(&p, "usage", &mut Budget::tick(), |_| panic!("nothing to hand on"));
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(b"\"usage\":\"tail of the long line\"}\n{\"usage\":5}\n").unwrap();
        assert_eq!(read_all(&mut t, &p), ["{\"usage\":5}"], "the rest of the long line is thrown away");
    }

    #[test]
    fn only_logs_written_since_are_found_and_never_through_a_link() {
        let d = tmp("walk");
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("a/b/new.jsonl"), "x\n").unwrap();
        std::fs::write(d.join("a/notes.txt"), "x\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(3600);
        assert_eq!(written_since(&d, SystemTime::UNIX_EPOCH).len(), 1);
        assert!(written_since(&d, later).is_empty());
        #[cfg(unix)]
        {
            let outside = tmp("walk-outside");
            std::fs::write(outside.join("secret.jsonl"), "x\n").unwrap();
            std::os::unix::fs::symlink(&outside, d.join("link")).unwrap();
            assert_eq!(written_since(&d, SystemTime::UNIX_EPOCH).len(), 1, "the link is not followed");
        }
    }

    /// A link, or a second name, for a file is never opened: either could be a sign-in file.
    #[cfg(unix)]
    #[test]
    fn a_link_or_a_hard_link_is_never_opened() {
        let d = tmp("links");
        let real = d.join("real.jsonl");
        std::fs::write(&real, "{\"usage\":1}\n").unwrap();
        assert!(open_own(&real).is_some());
        let soft = d.join("soft.jsonl");
        std::os::unix::fs::symlink(&real, &soft).unwrap();
        assert!(open_own(&soft).is_none());
        let hard = d.join("hard.jsonl");
        std::fs::hard_link(&real, &hard).unwrap();
        assert!(open_own(&hard).is_none(), "two names");
        assert!(open_own(&real).is_none(), "and so neither name");
        assert!(open_own(&d).is_none(), "not a file");
    }
}
