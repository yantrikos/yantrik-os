//! What `analyze_disk` reports, read in this process: the sizes under a directory, the files not
//! touched for 30 days and the commonest extensions. It replaces three `sh -c` pipelines
//! (`du | sort | head`, `find | awk`, `find | sed | sort | uniq`) that took the path inside a
//! shell string (#1124, round 9): no program is started and no shell reads the path.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// The deepest `find -maxdepth 2` looked.
const MAX_DEPTH: usize = 2;
/// A file this old counts as "not touched".
const OLD: Duration = Duration::from_secs(30 * 24 * 3600);

/// Bytes under `path`, links not followed (`du` counts a link, not its target).
fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else { return meta.len() };
    meta.len() + entries.flatten().map(|e| tree_size(&e.path())).sum::<u64>()
}

/// The ten biggest entries of `dir` that do not start with `.` (what `dir/*` matches), largest
/// first, as `(bytes, name)`.
pub fn largest_items(dir: &Path) -> Vec<(u64, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut items: Vec<(u64, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (!name.starts_with('.')).then(|| (tree_size(&e.path()), name))
        })
        .collect();
    items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    items.truncate(10);
    items
}

/// The regular files at depth 1 and 2 under `dir`, as `(name, bytes, modified)`.
fn files(dir: &Path, depth: usize, out: &mut Vec<(String, u64, Option<SystemTime>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(meta) = e.path().symlink_metadata() else { continue };
        if meta.is_file() {
            out.push((e.file_name().to_string_lossy().into_owned(), meta.len(), meta.modified().ok()));
        } else if meta.is_dir() && depth < MAX_DEPTH {
            files(&e.path(), depth + 1, out);
        }
    }
}

/// How many files under `dir` (two levels) are older than 30 days, and their bytes.
pub fn old_files(dir: &Path, now: SystemTime) -> (u64, u64) {
    let mut all = Vec::new();
    files(dir, 1, &mut all);
    all.iter()
        .filter(|(_, _, m)| m.and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > OLD))
        .fold((0, 0), |(n, bytes), (_, len, _)| (n + 1, bytes + len))
}

/// The eight extensions most files under `dir` (two levels) carry, as `(count, extension)`. A name
/// counts when it has a `.` (`'*.*'`), and its extension is what follows the last one.
pub fn extension_counts(dir: &Path) -> Vec<(usize, String)> {
    let mut all = Vec::new();
    files(dir, 1, &mut all);
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (name, _, _) in &all {
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

    fn tree() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-disk-scan-{}", std::process::id()));
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
        let dir = tree();
        let items = largest_items(&dir);
        assert_eq!(items.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["big", "small.rs"]);
        assert!(items[0].0 >= 5010);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extensions_count_two_levels_and_old_files_need_thirty_days() {
        let dir = tree();
        // depth 1: small.rs; depth 2: big/a.txt and the .hidden/h.md; depth 3 is not looked at.
        let ext = extension_counts(&dir);
        assert_eq!(ext, vec![(1, "md".to_string()), (1, "rs".to_string()), (1, "txt".to_string())]);
        assert_eq!(old_files(&dir, SystemTime::now()), (0, 0));
        let later = SystemTime::now() + Duration::from_secs(31 * 24 * 3600);
        assert_eq!(old_files(&dir, later).0, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_path_with_shell_words_is_only_a_path() {
        assert!(largest_items(Path::new("/nonexistent/$(touch x);y")).is_empty());
    }
}
