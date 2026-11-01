//! The sentences the documents layer says, pinned.
//!
//! Twice a Rust line continuation (`\` at the end of a line inside a string) was lost in an edit, and
//! every editor said "written and              your draft". The editors' tests only look for a word
//! of the sentence, so nothing noticed. The moved/deleted default sentences are asserted byte for
//! byte here; the run of spaces a lost continuation leaves in a string is what
//! `cargo run -q -p xtask -- check-sentences` is for, over the whole tree (tools/xtask/src/sentences.rs).

use std::path::{Path, PathBuf};

use super::files::read::inode;
use super::files::save::{stranded, SaveJob};
use super::files::FileRules;

/// The default words, as every editor but ySheets has them.
struct Defaults;

impl FileRules for Defaults {
    fn max_bytes(&self) -> usize {
        1 << 20
    }
    fn validate(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn reader(&self) -> fn(&Path) -> Result<String, String> {
        |p| std::fs::read_to_string(p).map_err(|e| e.to_string())
    }
    fn cannot_read(&self, _: &Path, e: &std::io::Error) -> String {
        e.to_string()
    }
    fn not_regular(&self) -> String {
        "not regular".into()
    }
    fn too_big(&self, _: &Path, _: u64) -> String {
        "too big".into()
    }
    fn not_utf8(&self) -> String {
        "not utf8".into()
    }
    fn cannot_open(&self, _: &Path, e: &std::io::Error) -> String {
        e.to_string()
    }
    fn relative(&self, _: &Path) -> String {
        "relative".into()
    }
    fn cannot_folder(&self, e: &std::io::Error) -> String {
        e.to_string()
    }
    fn open_elsewhere(&self, _: &str) -> String {
        "elsewhere".into()
    }
    fn already_there(&self, _: &Path) -> String {
        "there".into()
    }
    fn changed_on_disk(&self, _: &Path) -> String {
        "changed".into()
    }
    fn linked(&self, _: &Path) -> String {
        "linked".into()
    }
    fn read_only(&self, _: &Path) -> String {
        "read only".into()
    }
    fn save_failed(&self, e: &dyn std::fmt::Display) -> String {
        e.to_string()
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yantrik-sentences-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn job(target: &Path, on_disk: &str, inode: Option<super::files::read::Inode>) -> SaveJob {
    SaveJob {
        text: "new".into(),
        target: target.to_path_buf(),
        own: Some(target.to_path_buf()),
        on_disk: Some(on_disk.into()),
        unsure: None,
        inode,
        overwrite: false,
        agent: false,
        others: Vec::new(),
        elsewhere: PathBuf::new(),
    }
}

#[test]
fn the_default_moved_or_deleted_sentences_are_exactly_these_words() {
    let dir = scratch("stranded");
    let was = dir.join("was.txt");
    std::fs::write(&was, "hello").unwrap();
    let id = inode(&was);
    let now = dir.join("now.txt");
    std::fs::rename(&was, &now).unwrap();

    let found = stranded(&Defaults, &job(&was, "hello", id), &was);
    assert_eq!(
        found,
        format!(
            "{} is not there any more \u{2014} the same file looks to be at {} now. Nothing was written and your \
             draft is intact: Save As to {} to write your changes into it.",
            was.display(),
            now.display(),
            now.display()
        )
    );

    let lost = dir.join("lost.txt");
    let nowhere = stranded(&Defaults, &job(&lost, "something else entirely", None), &lost);
    assert_eq!(
        nowhere,
        format!(
            "{} was moved or deleted, so there is nothing there to write into. Nothing was written and your draft \
             is intact: Save As with overwrite=true to write it here, or Save As to wherever it went.",
            lost.display()
        )
    );
}
