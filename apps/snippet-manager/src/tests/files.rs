//! Files in and out: `import` of a source file or an export, a file on the command line or handed
//! over by a second launch, and Export.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::launch::Launch;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths};

use super::{open, start_at};
use crate::SnippetManager;

/// A folder of the test's own with one shell script in it.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let dir = std::env::temp_dir().join(format!("snippets-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("deploy.sh"), "#!/bin/sh\nset -eu\nrsync -a ./ vm:/opt/yantrik\n").unwrap();
        Folder(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn import_keeps_a_source_file_with_its_language_and_opens_it() {
    let folder = Folder::new("import");
    let f = open();
    let answer = f.act("import", json!({ "path": folder.path("deploy.sh").display().to_string() })).unwrap();
    assert_eq!(answer["imported"], json!(1));
    assert_eq!(answer["as"], json!("file"));
    assert_eq!(answer["titles"], json!(["deploy.sh"]));
    let id = answer["ids"][0].as_i64().unwrap() as i32;
    let kept = f.on_disk().get(id).cloned().unwrap();
    assert_eq!((kept.title.as_str(), kept.language.as_str()), ("deploy.sh", "Shell"));
    assert_eq!(f.state()["open"], json!("deploy.sh"));
}

/// A file that is not there is the Mind's refusal, and not a notice: nothing was decided.
#[test]
fn import_of_a_file_that_is_not_there_is_refused_and_not_said() {
    let f = open();
    let e = f.act("import", json!({ "path": "/nowhere/at/all.rs" })).unwrap_err();
    assert_eq!(e, "no file at /nowhere/at/all.rs");
    assert_eq!(f.notice(), "");
    assert!(f.act("import", json!({ "path": "  " })).unwrap_err().contains("`path` is empty"));
}

/// A file that is there but cannot be read as text is said, on screen and to the caller.
#[test]
fn import_of_a_file_that_is_not_text_is_said_twice() {
    let folder = Folder::new("binary");
    std::fs::write(folder.path("blob.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
    let f = open();
    let e = f.act("import", json!({ "path": folder.path("blob.bin").display().to_string() })).unwrap_err();
    assert!(e.contains("is not UTF-8 text"), "{e}");
    assert_eq!(f.notice(), e);
}

/// An export reads back in as a bundle, under fresh ids.
#[test]
fn an_export_is_written_said_and_reads_back_in() {
    let f = open();
    f.seeded();
    f.app.ui(|ui| ui.invoke_snip_export_all());
    f.quiet();
    let notice = f.notice();
    assert!(notice.starts_with("Exported 2 snippet(s) to "), "{notice}");
    let path = PathBuf::from(notice.trim_start_matches("Exported 2 snippet(s) to "));
    assert!(path.starts_with(f.paths.data_dir()), "beside the store: {}", path.display());
    let answer = f.act("import", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(answer["as"], json!("bundle"));
    assert_eq!(answer["imported"], json!(2));
    assert_eq!(answer["total"], json!(4));
    assert_eq!(f.on_disk().snippets().len(), 4);
}

/// A file on the command line is kept and open when the window shows it.
#[test]
fn a_file_on_the_command_line_is_kept_and_open() {
    let folder = Folder::new("launch");
    let f = start_at(Paths::temporary(SnippetManager::ID), FakeMind::new(), vec![folder.path("deploy.sh").into()]);
    assert_eq!(f.state()["open"], json!("deploy.sh"));
    assert_eq!(f.on_disk().snippets().len(), 1);
}

/// One that is not there is said on the notice line, where the person who named it looks.
#[test]
fn a_file_on_the_command_line_that_is_not_there_is_said() {
    let folder = Folder::new("missing");
    let f = start_at(Paths::temporary(SnippetManager::ID), FakeMind::new(), vec![folder.path("gone.rs").into()]);
    assert_eq!(f.notice(), format!("no file at {}", folder.path("gone.rs").display()));
    assert_eq!(f.state()["total"], json!(0));
}

/// A second launch with a file hands it to the window already open, which keeps it once however
/// often the hand-over is retried.
#[test]
fn a_second_launch_hands_its_file_over_once() {
    let folder = Folder::new("handover");
    let f = open();
    let mut launch = Launch::empty(f.paths.profile());
    launch.files = vec![folder.path("deploy.sh")];
    let args = json!({ "launch": launch });
    f.act("launch", args.clone()).unwrap();
    f.act("launch", args).unwrap();
    assert_eq!(f.on_disk().snippets().len(), 1, "a retried hand-over opens nothing twice");
    assert_eq!(f.state()["open"], json!("deploy.sh"));
}

/// What comes in is filed under the collection selected when it was asked for, wherever the
/// sidebar is by the time the file has been read.
#[test]
fn an_import_is_filed_where_it_was_asked_for() {
    let folder = Folder::new("filed");
    let f = open();
    f.app.ui(|ui| ui.invoke_collection_create("Scripts".into()));
    f.quiet();
    let scripts = f.app.model(|m| m.collection);
    let pending = f.app.act("import", json!({ "path": folder.path("deploy.sh").display().to_string() })).unwrap();
    f.app.ui(|ui| ui.invoke_collection_select(crate::store::ALL));
    f.quiet();
    let id = pending.settle().unwrap()["ids"][0].as_i64().unwrap() as i32;
    assert_eq!(f.on_disk().get(id).unwrap().collection, scripts);
}
