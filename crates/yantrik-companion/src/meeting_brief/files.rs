//! Section 4: files in Documents, Desktop and Downloads whose names carry a word of the title.
//!
//! By name only: no file is ever opened or read here. The walk is bounded so that a large home
//! folder costs the same as a small one: three levels deep, five thousand entries in all, and
//! nothing hidden.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::model::{age, clamp, Open, Row, LOOKBACK_DAYS, MAX_FILES, MAX_ROW_CHARS};

/// How many levels below a root are read.
pub const MAX_DEPTH: usize = 3;
/// How many entries, in all roots, are looked at before the walk stops.
pub const MAX_ENTRIES: usize = 5_000;

/// How long one walk may take. It runs on the fetcher's thread, never the worker's, but a folder
/// that is a network or FUSE mount can stall each call, and the walk gives up between calls.
pub const WALK_BUDGET: Duration = Duration::from_millis(400);

/// Where file names are searched: the real folders, or a stand-in in a test.
pub trait FileSource {
    /// Up to two rows for files whose names carry any of `keywords`.
    fn find(&self, keywords: &[String]) -> Result<Vec<Row>, String>;
}

impl FileSource for FileRoots {
    fn find(&self, keywords: &[String]) -> Result<Vec<Row>, String> {
        Ok(section(self, keywords, SystemTime::now()).0)
    }
}

/// The folders searched.
#[derive(Debug, Clone, Default)]
pub struct FileRoots(pub Vec<PathBuf>);

impl FileRoots {
    /// `~/Documents`, `~/Desktop` and `~/Downloads`.
    pub fn in_home(home: &Path) -> Self {
        Self(["Documents", "Desktop", "Downloads"].iter().map(|d| home.join(d)).collect())
    }
}

/// What the walk looked at, for the tests that hold it to its bounds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Walked {
    pub entries: usize,
}

struct Found {
    path: PathBuf,
    name: String,
    folder: String,
    modified: SystemTime,
}

/// Up to two rows, the newest first: "gate-report.pdf · Documents · 3 d".
pub fn section(roots: &FileRoots, keywords: &[String], now: SystemTime) -> (Vec<Row>, Walked) {
    section_by(roots, keywords, now, Instant::now() + WALK_BUDGET)
}

/// [`section`] that gives up at `deadline`, with what it has found by then.
pub fn section_by(roots: &FileRoots, keywords: &[String], now: SystemTime, deadline: Instant) -> (Vec<Row>, Walked) {
    if keywords.is_empty() {
        return (Vec::new(), Walked::default());
    }
    let (mut found, walked) = walk(roots, keywords, now, deadline);
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    found.truncate(MAX_FILES);
    let now_secs = secs(now);
    let rows = found
        .into_iter()
        .map(|f| Row {
            text: clamp(&format!("{} \u{b7} {} \u{b7} {}", f.name, f.folder, age(now_secs, secs(f.modified))), MAX_ROW_CHARS),
            open: Some(Open::File { path: f.path.to_string_lossy().into_owned() }),
        })
        .collect();
    (rows, walked)
}

fn secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Whether `dir` is on the same filesystem as `root`: a mount below a root (a share, a FUSE
/// folder) is not walked.
#[cfg(unix)]
fn same_device(root: &Path, dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(root), std::fs::symlink_metadata(dir)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_device(_: &Path, _: &Path) -> bool {
    true
}

fn walk(roots: &FileRoots, keywords: &[String], now: SystemTime, deadline: Instant) -> (Vec<Found>, Walked) {
    let oldest = now.checked_sub(Duration::from_secs(LOOKBACK_DAYS as u64 * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut found = Vec::new();
    let mut entries = 0usize;
    'roots: for root in &roots.0 {
        let folder = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // (directory, depth of its entries)
        let mut pending: Vec<(PathBuf, usize)> = vec![(root.clone(), 1)];
        while let Some((dir, depth)) = pending.pop() {
            if Instant::now() >= deadline {
                break 'roots;
            }
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            for entry in read.flatten() {
                if entries >= MAX_ENTRIES || (entries % 64 == 0 && Instant::now() >= deadline) {
                    break 'roots;
                }
                entries += 1;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                // `file_type` does not follow a link: a link is neither descended nor listed.
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    if depth < MAX_DEPTH && same_device(root, &entry.path()) {
                        pending.push((entry.path(), depth + 1));
                    }
                } else if kind.is_file() {
                    let lower = name.to_lowercase();
                    if !keywords.iter().any(|k| lower.contains(k.as_str())) {
                        continue;
                    }
                    let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) else { continue };
                    if modified >= oldest && modified <= now + Duration::from_secs(86_400) {
                        found.push(Found { path: entry.path(), name, folder: folder.clone(), modified });
                    }
                }
            }
        }
    }
    (found, Walked { entries })
}
