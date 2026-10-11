//! A folder opened as a project beside the tabs (#731): which of its entries are on screen.
//!
//! Pure: it reads the folder and answers with rows, and knows nothing of Slint. The window turns
//! the rows into the shared `YTree`, and `describe` reads the same rows back to a mind.
//!
//! What is listed is bounded, because a folder is untrusted input of any size: 2,000 rows at
//! most, 300 entries of any one folder, and a dim "… N more" row where a list was cut. A link is
//! shown but never followed into a folder, so a link back up the tree cannot loop the walk or
//! take it outside the folder that was opened.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The most rows the tree holds, so the walk and the list in the window stay bounded.
pub const MAX_ROWS: usize = 2000;
/// The most entries shown of any one folder; the rest are counted in a "… N more" row.
pub const PER_FOLDER: usize = 300;
/// The most entries read out of one folder to sort, and visited by one filter.
const MAX_SCAN: usize = 100_000;
/// The most expanded folders kept in the recovery file, whose size is bounded (see
/// `document::RECOVERY_FOLDER`).
pub const MAX_SAVED_EXPANDED: usize = 64;
/// Build output, caches and history: never what a person opens a project to read.
const SKIPPED: [&str; 7] = [".git", "target", "node_modules", "__pycache__", ".venv", "dist", ".next"];
/// The dot-files worth seeing; every other hidden entry is left out.
const SHOWN_HIDDEN: [&str; 2] = [".gitignore", ".env.example"];
pub const CANT_READ: &str = "can't read this folder";

/// One visible row. A `dim` row is a note about the folder at `path` ("… N more", or that it
/// could not be read), not an entry in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub dim: bool,
}

/// The open folder: where it is, which of its folders are expanded, and the filter.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    filter: String,
}

/// What the recovery file keeps of the open folder, so it is open again at the next launch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub root: PathBuf,
    #[serde(default)]
    pub expanded: Vec<PathBuf>,
}

fn skipped(name: &str) -> bool {
    SKIPPED.contains(&name) || (name.starts_with('.') && !SHOWN_HIDDEN.contains(&name))
}

/// A folder's entries worth showing, folders first, then files, each by name ignoring case, and
/// how many there were. A link is never a folder here: `file_type` does not follow it.
fn list(dir: &Path) -> std::io::Result<Vec<(String, PathBuf, bool)>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir)?.take(MAX_SCAN) {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if skipped(&name) {
            continue;
        }
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        entries.push((name, entry.path(), is_dir));
    }
    entries.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(entries)
}

fn note(path: &Path, depth: usize, name: String) -> Node {
    Node { path: path.to_path_buf(), name, is_dir: false, depth, dim: true }
}

impl Project {
    pub fn new(root: PathBuf) -> Self {
        Self { root, expanded: BTreeSet::new(), filter: String::new() }
    }

    /// The folder kept in the recovery file, if it is still a folder; expanded folders outside
    /// it are dropped, since the file is untrusted input.
    pub fn from_saved(saved: Saved) -> Option<Self> {
        if !saved.root.is_dir() {
            return None;
        }
        let mut project = Self::new(saved.root);
        project.expanded = saved
            .expanded
            .into_iter()
            .filter(|p| p.starts_with(&project.root))
            .take(MAX_SAVED_EXPANDED)
            .collect();
        Some(project)
    }

    /// What the recovery file keeps. Only paths JSON holds as plain text are kept: a name that
    /// is not UTF-8 would fail the whole checkpoint, drafts and all, and a control character
    /// would be escaped past the file's size bound. A root like that is not kept at all.
    pub fn saved(&self) -> Option<Saved> {
        let plain = |p: &Path| p.to_str().is_some_and(|t| !t.chars().any(char::is_control));
        plain(&self.root).then(|| Saved {
            root: self.root.clone(),
            expanded: self.expanded.iter().filter(|p| plain(p)).take(MAX_SAVED_EXPANDED).cloned().collect(),
        })
    }

    /// The folder's own name, for the panel's heading.
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Expand a folder, or collapse it if it is expanded.
    pub fn toggle(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
    }

    /// Expand every folder between the root and `path`, so its row is on screen. Something
    /// outside the root is left alone.
    pub fn expand_to(&mut self, path: &Path) {
        let Ok(inner) = path.strip_prefix(&self.root) else { return };
        let mut at = self.root.clone();
        let parts: Vec<_> = inner.components().collect();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            at.push(part);
            self.expanded.insert(at.clone());
        }
    }

    pub fn set_filter(&mut self, text: &str) {
        self.filter = text.trim().to_string();
    }

    /// The rows on screen: the tree, or with a filter, every matching file as a flat list.
    pub fn rows(&self) -> Vec<Node> {
        let mut out = Vec::new();
        let mut cut = 0;
        if self.filter.is_empty() {
            self.walk(&self.root, 0, &mut out, &mut cut);
        } else {
            let mut scanned = 0;
            self.find(&self.root, &self.filter.to_lowercase(), &mut out, &mut cut, &mut scanned);
        }
        if cut > 0 {
            out.push(note(&self.root, 0, format!("… {cut} more")));
        }
        out
    }

    fn walk(&self, dir: &Path, depth: usize, out: &mut Vec<Node>, cut: &mut usize) {
        let entries = match list(dir) {
            Ok(entries) => entries,
            Err(_) => {
                if out.len() < MAX_ROWS {
                    out.push(note(dir, depth, CANT_READ.into()));
                }
                return;
            }
        };
        let more = entries.len().saturating_sub(PER_FOLDER);
        for (name, path, is_dir) in entries.into_iter().take(PER_FOLDER) {
            if out.len() >= MAX_ROWS {
                *cut += 1;
                continue;
            }
            let open = is_dir && self.expanded.contains(&path);
            out.push(Node { path: path.clone(), name, is_dir, depth, dim: false });
            if open {
                self.walk(&path, depth + 1, out, cut);
            }
        }
        if more > 0 {
            if out.len() < MAX_ROWS {
                out.push(note(dir, depth, format!("… {more} more")));
            } else {
                *cut += more;
            }
        }
    }

    fn find(&self, dir: &Path, needle: &str, out: &mut Vec<Node>, cut: &mut usize, scanned: &mut usize) {
        let Ok(entries) = list(dir) else { return };
        for (name, path, is_dir) in entries.into_iter().take(PER_FOLDER) {
            *scanned += 1;
            if *scanned > MAX_SCAN {
                return;
            }
            if is_dir {
                self.find(&path, needle, out, cut, scanned);
            } else if name.to_lowercase().contains(needle) {
                if out.len() < MAX_ROWS {
                    let name = self.relative(&path, false);
                    out.push(Node { path, name, is_dir: false, depth: 0, dim: false });
                } else {
                    *cut += 1;
                }
            }
        }
    }

    /// `path` as it reads under the root, with a "/" after a folder.
    pub fn relative(&self, path: &Path, is_dir: bool) -> String {
        let inner = path.strip_prefix(&self.root).unwrap_or(path).display().to_string();
        if is_dir {
            inner + "/"
        } else {
            inner
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("yantrik-project-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn names(rows: &[Node]) -> Vec<String> {
        rows.iter().map(|n| format!("{}{}", "  ".repeat(n.depth), n.name)).collect()
    }

    #[test]
    fn folders_come_first_then_files_each_ignoring_case_and_the_noise_is_skipped() {
        let root = tree("order");
        for d in ["beta", "Alpha", ".git", "target", "node_modules", "__pycache__", ".venv", "dist", ".next", ".cache"] {
            fs::create_dir(root.join(d)).unwrap();
        }
        for f in ["b.txt", "A.md", "c.rs", ".gitignore", ".env.example", ".env", ".hidden"] {
            fs::write(root.join(f), "x").unwrap();
        }
        let project = Project::new(root.clone());
        assert_eq!(
            names(&project.rows()),
            ["Alpha", "beta", ".env.example", ".gitignore", "A.md", "b.txt", "c.rs"]
        );
        assert!(project.rows()[0].is_dir && !project.rows()[2].is_dir);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_expanded_folders_show_their_children_and_expand_to_reveals_a_nested_file() {
        let root = tree("expand");
        fs::create_dir_all(root.join("hermes/docs")).unwrap();
        fs::write(root.join("hermes/docs/README.md"), "x").unwrap();
        fs::write(root.join("hermes/adapter.py"), "x").unwrap();
        fs::write(root.join("top.txt"), "x").unwrap();
        let mut project = Project::new(root.clone());
        assert_eq!(names(&project.rows()), ["hermes", "top.txt"]);
        project.toggle(&root.join("hermes"));
        assert_eq!(names(&project.rows()), ["hermes", "  docs", "  adapter.py", "top.txt"]);
        project.toggle(&root.join("hermes"));
        assert_eq!(names(&project.rows()), ["hermes", "top.txt"]);
        let readme = root.join("hermes/docs/README.md");
        project.expand_to(&readme);
        let rows = project.rows();
        let at = rows.iter().position(|n| n.path == readme).expect("the file is on screen");
        assert_eq!(rows[at].depth, 2);
        // Something outside the root expands nothing.
        project.expand_to(Path::new("/etc/hostname"));
        assert_eq!(project.rows().len(), rows.len());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_big_folder_is_cut_with_a_count_and_the_tree_stops_at_its_cap() {
        let root = tree("caps");
        for d in 0..8 {
            let dir = root.join(format!("d{d}"));
            fs::create_dir(&dir).unwrap();
            for f in 0..(PER_FOLDER + 10) {
                fs::write(dir.join(format!("f{f:04}")), "").unwrap();
            }
        }
        let mut project = Project::new(root.clone());
        project.toggle(&root.join("d0"));
        let rows = project.rows();
        assert_eq!(rows.len(), 8 + PER_FOLDER + 1);
        let more = &rows[1 + PER_FOLDER];
        assert!(more.dim && more.name == "… 10 more" && more.depth == 1, "{more:?}");
        for d in 1..8 {
            project.toggle(&root.join(format!("d{d}")));
        }
        let rows = project.rows();
        assert_eq!(rows.len(), MAX_ROWS + 1);
        let last = rows.last().unwrap();
        assert!(last.dim && last.name.starts_with("… ") && last.name.ends_with(" more"), "{last:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_linked_folder_is_shown_but_not_followed() {
        let root = tree("links");
        fs::create_dir(root.join("real")).unwrap();
        fs::write(root.join("real/inside.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let mut project = Project::new(root.clone());
        project.toggle(&root.join("loop"));
        let rows = project.rows();
        assert_eq!(names(&rows), ["real", "loop"]);
        assert!(!rows[1].is_dir, "a link is never a folder in the tree");
        project.set_filter("inside");
        assert_eq!(names(&project.rows()), ["real/inside.txt"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unreadable_folder_says_so_in_one_dim_row() {
        use std::os::unix::fs::PermissionsExt;
        let root = tree("unreadable");
        let locked = root.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("secret"), "x").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let mut project = Project::new(root.clone());
        project.toggle(&locked);
        let rows = project.rows();
        // Root reads everything, so this only holds for an ordinary account.
        if fs::read_dir(&locked).is_err() {
            assert_eq!(rows.len(), 2);
            assert!(rows[1].dim && rows[1].name == CANT_READ && rows[1].depth == 1);
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_filter_lists_matching_files_under_the_root_as_relative_paths() {
        let root = tree("filter");
        fs::create_dir_all(root.join("hermes")).unwrap();
        fs::create_dir_all(root.join("pi/target")).unwrap();
        fs::write(root.join("hermes/adapter.py"), "x").unwrap();
        fs::write(root.join("hermes/README.md"), "x").unwrap();
        fs::write(root.join("pi/Adapter.rs"), "x").unwrap();
        fs::write(root.join("pi/target/adapter.o"), "x").unwrap();
        let mut project = Project::new(root.clone());
        project.set_filter(" ADAPTER ");
        let rows = project.rows();
        assert_eq!(names(&rows), ["hermes/adapter.py", "pi/Adapter.rs"]);
        assert_eq!(rows[0].path, root.join("hermes/adapter.py"));
        project.set_filter("");
        assert_eq!(names(&project.rows()), ["hermes", "pi"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_saved_folder_comes_back_with_only_its_own_expanded_folders() {
        let root = tree("saved");
        fs::create_dir(root.join("a")).unwrap();
        let mut project = Project::new(root.clone());
        project.toggle(&root.join("a"));
        let mut saved = project.saved().unwrap();
        saved.expanded.push(PathBuf::from("/etc"));
        let back = Project::from_saved(saved).unwrap();
        assert!(back.is_expanded(&root.join("a")) && !back.is_expanded(Path::new("/etc")));
        fs::remove_dir_all(&root).unwrap();
        assert!(Project::from_saved(project.saved().unwrap()).is_none(), "a folder no longer there is not opened");
    }
}
