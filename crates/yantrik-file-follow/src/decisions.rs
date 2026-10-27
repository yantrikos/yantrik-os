//! The decisions, with a real directory and no filesystem event in sight: where a file went after a
//! move that only said it left (`moved_to`), and where it is after a rename the folder reported
//! (`follow_rename`). They were yDoc's `tests/document-core` assertions and live with the code.

use super::*;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("file-follow-decide-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a scratch directory");
        Self(fs::canonicalize(&path).expect("a real path"))
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| e.to_string())
}

/// Opens a file as a document would, then moves it with `rename` the way Files does.
fn moved(dir: &Dir, name: &str, into: &str, text: &str) -> (PathBuf, PathBuf) {
    let from = dir.join(name);
    fs::write(&from, text).unwrap();
    let folder = dir.join(into);
    fs::create_dir_all(&folder).unwrap();
    let to = folder.join(name);
    fs::rename(&from, &to).unwrap();
    (from, to)
}

#[test]
fn a_moved_file_is_found_by_its_name_and_its_bytes_together() {
    let dir = Dir::new("together");
    let text = "# Pricing\n\nrates\n";
    let (from, to) = moved(&dir, "northwind-pricing.md", "pricing/2026", text);
    assert_eq!(moved_to(&from, text, read).as_deref(), Some(to.as_path()));

    // The same name with different bytes is a different document.
    fs::write(dir.join("northwind-pricing.md"), "# Pricing\n\nsomething else\n").unwrap();
    assert_eq!(moved_to(&from, "# Pricing\n\nnot this\n", read), None);

    // The same bytes under a different name is a copy, not this document.
    fs::write(dir.join("pricing/copy.md"), text).unwrap();
    fs::remove_file(&to).unwrap();
    assert_eq!(moved_to(&from, text, read), None);
}

#[test]
fn the_search_for_a_moved_file_does_not_find_the_file_it_started_from() {
    let dir = Dir::new("itself");
    let path = dir.join("still-here.md");
    fs::write(&path, "body\n").unwrap();
    assert_eq!(moved_to(&path, "body\n", read), None);
}

#[test]
fn the_search_stays_out_of_hidden_folders() {
    let dir = Dir::new("hidden");
    let text = "body\n";
    let path = dir.join("notes.md");
    fs::write(&path, text).unwrap();
    fs::create_dir_all(dir.join(".cache/deep")).unwrap();
    fs::write(dir.join(".cache/deep/notes.md"), text).unwrap();
    fs::remove_file(&path).unwrap();
    // A copy under `.cache` is a cache, not where a person moved their document.
    assert_eq!(moved_to(&path, text, read), None);
}

#[test]
fn a_document_follows_its_own_rename_and_the_rename_of_a_folder_above_it() {
    let renamed = follow_rename(
        Path::new("/home/p/Documents/notes.md"),
        Path::new("/home/p/Documents/notes.md"),
        Path::new("/home/p/Documents/pricing.md"),
    );
    assert_eq!(renamed.as_deref(), Some(Path::new("/home/p/Documents/pricing.md")));

    let carried = follow_rename(
        Path::new("/home/p/Documents/notes.md"),
        Path::new("/home/p/Documents"),
        Path::new("/home/p/Archive"),
    );
    assert_eq!(carried.as_deref(), Some(Path::new("/home/p/Archive/notes.md")));
}

#[test]
fn a_rename_of_some_other_file_leaves_the_document_where_it_is() {
    assert_eq!(
        follow_rename(
            Path::new("/home/p/Documents/notes.md"),
            Path::new("/home/p/Documents/other.md"),
            Path::new("/home/p/Documents/renamed.md"),
        ),
        None
    );
    // A prefix that is not a path component is not an ancestor: `/home/p/Doc` is not the
    // folder `/home/p/Documents` is in, whatever the strings look like.
    assert_eq!(
        follow_rename(
            Path::new("/home/p/Documents/notes.md"),
            Path::new("/home/p/Doc"),
            Path::new("/home/p/Elsewhere"),
        ),
        None
    );
}
