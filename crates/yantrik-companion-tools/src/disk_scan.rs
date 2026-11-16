//! What `analyze_disk` reports, read in this process: the sizes under a directory, the files not
//! touched for 30 days and the commonest extensions. It replaces three `sh -c` pipelines
//! (`du | sort | head`, `find | awk`, `find | sed | sort | uniq`) that took the path inside a
//! shell string (#1124, round 9): no program is started and no shell reads the path.

use std::fs::Metadata;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use yantrik_ipc_contracts::home_paths::Walk;

/// The deepest `find -maxdepth 2` looked.
const MAX_DEPTH: usize = 2;
/// A file this old counts as "not touched".
const OLD: Duration = Duration::from_secs(30 * 24 * 3600);

/// The longest one tool call walks. A walk that is not done by then stops and says its figures are
/// partial (review R11-M3); `du` of a tree of 20,000 files takes tens of milliseconds.
pub const MOST_TIME: Duration = Duration::from_secs(10);

/// One walk: the folder it started on, who may be shown (judged once for the whole walk, see
/// [`Walk`]), how many entries were left out, and whether the time ran out.
pub struct Scan {
    walk: Walk,
    started: Instant,
    limit: Duration,
    /// Protected entries left out of what is shown or counted.
    pub hidden: usize,
    /// The walk stopped at its time limit: every figure is a lower bound.
    pub partial: bool,
}

impl Scan {
    /// A walk over `root`, with the time limit of [`MOST_TIME`].
    pub fn new(root: &Path) -> Scan {
        Scan::with_limit(root, MOST_TIME)
    }

    pub fn with_limit(root: &Path, limit: Duration) -> Scan {
        Scan { walk: Walk::new(root), started: Instant::now(), limit, hidden: 0, partial: false }
    }

    /// The note a tool adds to its figures when the walk was cut short, or nothing.
    pub fn partial_note(&self) -> Option<String> {
        self.partial.then(|| format!("(partial: the walk stopped after {} s, so every figure here is a lower bound)", self.limit.as_secs()))
    }

    fn out_of_time(&mut self) -> bool {
        self.partial = self.partial || self.started.elapsed() > self.limit;
        self.partial
    }

    fn is_protected(&self, at: &Path, meta: &Metadata) -> bool {
        self.walk.is_protected(at, meta)
    }
}

/// The folders under `dir` down to `depth` levels with the bytes in each (what `du --max-depth`
/// lists, `dir` itself last). Read in this process, so every figure is made of the entries that are
/// shown: a protected file adds nothing to any total, and no program's total has it in (#1247, gate 12).
pub fn folder_sizes(scan: &mut Scan, dir: &Path, depth: u64) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    let total = folder(scan, dir, depth, &mut out);
    out.push((total, dir.to_string_lossy().into_owned()));
    out
}

fn folder(scan: &mut Scan, dir: &Path, depth: u64, out: &mut Vec<(u64, String)>) -> u64 {
    let Ok(meta) = dir.symlink_metadata() else { return 0 };
    let mut total = meta.len();
    let Ok(entries) = std::fs::read_dir(dir) else { return total };
    for e in entries.flatten() {
        if scan.out_of_time() {
            break;
        }
        let at = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if scan.is_protected(&at, &meta) {
            scan.hidden += 1;
            continue;
        }
        if meta.is_dir() && depth > 0 {
            let size = folder(scan, &at, depth - 1, out);
            out.push((size, at.to_string_lossy().into_owned()));
            total += size;
        } else {
            total += tree_size(scan, &at, &meta);
        }
    }
    total
}

/// Bytes under `path` (whose `meta` is already read), links not followed (`du` counts a link, not its
/// target). A protected file inside adds nothing: a total that moves with a hidden file tells its size
/// (#1247, gate 12).
fn tree_size(scan: &mut Scan, path: &Path, meta: &Metadata) -> u64 {
    if !meta.is_dir() {
        return meta.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else { return meta.len() };
    let mut total = meta.len();
    for e in entries.flatten() {
        if scan.out_of_time() {
            break;
        }
        let Ok(inner) = e.metadata() else { continue };
        if !scan.is_protected(&e.path(), &inner) {
            total += tree_size(scan, &e.path(), &inner);
        }
    }
    total
}

/// The ten biggest entries of `dir` that do not start with `.` (what `dir/*` matches), largest
/// first, as `(bytes, name)`. Entries left out because they are protected files or places (a name
/// says where a secret is kept) are counted in `scan.hidden` (#1247, review M3).
pub fn largest_items(scan: &mut Scan, dir: &Path) -> Vec<(u64, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut items: Vec<(u64, String)> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || scan.out_of_time() {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        if scan.is_protected(&e.path(), &meta) {
            scan.hidden += 1;
            continue;
        }
        items.push((tree_size(scan, &e.path(), &meta), name));
    }
    items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    items.truncate(10);
    items
}

/// A regular file found: its name, its bytes and when it was modified.
pub type Found = (String, u64, Option<SystemTime>);

/// The regular files at depth 1 and 2 under `dir`.
pub fn files(scan: &mut Scan, dir: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    files_at(scan, dir, 1, &mut out);
    out
}

fn files_at(scan: &mut Scan, dir: &Path, depth: usize, out: &mut Vec<Found>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if scan.out_of_time() {
            return;
        }
        let Ok(meta) = e.metadata() else { continue };
        // Not a protected file or place: its name, or a part of it, would show in the file types
        // (`1 Identifier` from `tokens.json:Zone.Identifier`; #1247, gate 12).
        if scan.is_protected(&e.path(), &meta) {
            continue;
        }
        if meta.is_file() {
            out.push((e.file_name().to_string_lossy().into_owned(), meta.len(), meta.modified().ok()));
        } else if meta.is_dir() && depth < MAX_DEPTH {
            files_at(scan, &e.path(), depth + 1, out);
        }
    }
}

/// How many of `found` are older than 30 days, and their bytes.
pub fn old_files(found: &[Found], now: SystemTime) -> (u64, u64) {
    found
        .iter()
        .filter(|(_, _, m)| m.and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > OLD))
        .fold((0, 0), |(n, bytes), (_, len, _)| (n + 1, bytes + len))
}

/// The eight extensions most of `found` carry, as `(count, extension)`. A name counts when it has a
/// `.` (`'*.*'`), and its extension is what follows the last one.
pub fn extension_counts(found: &[Found]) -> Vec<(usize, String)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (name, _, _) in found {
        if let Some((_, ext)) = name.rsplit_once('.') {
            *counts.entry(ext.to_string()).or_default() += 1;
        }
    }
    let mut counted: Vec<(usize, String)> = counts.into_iter().map(|(e, n)| (n, e)).collect();
    counted.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    counted.truncate(8);
    counted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-disk-scan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("big/deep/deeper")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("big/a.txt"), vec![0u8; 5000]).unwrap();
        std::fs::write(dir.join("big/deep/b.txt"), vec![0u8; 10]).unwrap();
        std::fs::write(dir.join("big/deep/deeper/c.rs"), b"x").unwrap();
        std::fs::write(dir.join("small.rs"), b"x").unwrap();
        std::fs::write(dir.join(".hidden/h.md"), vec![0u8; 99999]).unwrap();
        dir
    }

    #[test]
    fn the_biggest_items_leave_dotfiles_out_and_sort_by_size() {
        let dir = tree("items");
        let mut scan = Scan::new(&dir);
        let items = largest_items(&mut scan, &dir);
        assert_eq!(scan.hidden, 0);
        assert_eq!(items.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["big", "small.rs"]);
        assert!(items[0].0 >= 5010);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extensions_count_two_levels_and_old_files_need_thirty_days() {
        let dir = tree("ext");
        // depth 1: small.rs; depth 2: big/a.txt and the .hidden/h.md; depth 3 is not looked at.
        let found = files(&mut Scan::new(&dir), &dir);
        let ext = extension_counts(&found);
        assert_eq!(ext, vec![(1, "md".to_string()), (1, "rs".to_string()), (1, "txt".to_string())]);
        assert_eq!(old_files(&found, SystemTime::now()), (0, 0));
        let later = SystemTime::now() + Duration::from_secs(31 * 24 * 3600);
        assert_eq!(old_files(&found, later).0, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_path_with_shell_words_is_only_a_path() {
        let at = Path::new("/nonexistent/$(touch x);y");
        assert!(largest_items(&mut Scan::new(at), at).is_empty());
    }

    /// Review M3 of #1247: the names and sizes of the biggest entries are what `analyze_disk` prints.
    #[test]
    fn a_protected_file_is_counted_and_never_listed() {
        let dir = tree("secret");
        for name in ["tokens.json", "server.pem", "tokens.json.bak"] {
            std::fs::write(dir.join(name), vec![0u8; 90000]).unwrap();
        }
        std::fs::create_dir_all(dir.join("private-keys-v1.d")).unwrap();
        std::fs::write(dir.join("keynote.txt"), vec![0u8; 8000]).unwrap();
        let mut scan = Scan::new(&dir);
        let items = largest_items(&mut scan, &dir);
        let names: Vec<&str> = items.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["big", "keynote.txt", "small.rs"]);
        assert_eq!(scan.hidden, 4);
        let _ = std::fs::remove_dir_all(dir);
    }
}
