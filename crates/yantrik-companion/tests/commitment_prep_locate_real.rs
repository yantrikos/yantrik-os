//! Finding a promised document with the real `glob` tool on a real folder (#1184). On VM 520 the
//! Documents folder held 292 `.md` files; `glob` lists the newest fifty of a pattern, the search
//! gave up once a listing was cut, and the one file that matched by name was never chosen.

use std::path::PathBuf;
use std::time::Duration;

use yantrik_companion::commitment_files::{NamedFile, Role};
use yantrik_companion::commitment_hands::Hands;
use yantrik_companion::commitment_locate::{locate, Located};
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

/// A folder of `count` notes, in the home (the tool only reads there), removed on drop.
struct Folder(PathBuf);

impl Folder {
    fn with_notes(count: usize, extra: &[&str]) -> Self {
        let home = PathBuf::from(std::env::var("HOME").expect("a home"));
        let dir = home.join(format!("yantrik-locate-{}-{count}-{}", std::process::id(), extra.len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..count {
            std::fs::write(dir.join(format!("note-{i}.md")), "x").unwrap();
        }
        for name in extra {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn find(folder: &Folder, label: &str) -> (Located, String) {
    let mut registry = ToolRegistry::new();
    yantrik_companion_tools::glob::register(&mut registry);
    let db = yantrikdb_core::YantrikDB::new(":memory:", 64).expect("a database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Standard, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    // What a plain listing of the kind says in a folder this size: cut, which is the fault.
    let root = folder.0.to_string_lossy().into_owned();
    let listing = registry.execute(&ctx, "glob", &serde_json::json!({ "pattern": "**/*.md", "path": root }));
    let file = NamedFile { role: Role::Attach, label: label.into() };
    let kind = file.kind().expect("a kind");
    let mut hands = Hands::new(&registry, &ctx, Duration::from_secs(30));
    (locate(&mut hands, &file, kind, &[root]), listing)
}

#[test]
fn one_file_named_for_the_label_is_found_among_three_hundred() {
    let folder = Folder::with_notes(300, &["gate8-alpha-report.md"]);
    let (found, listing) = find(&folder, "gate8 alpha report");
    assert!(listing.contains("... and 251 more files"), "the plain listing is cut at fifty: {listing}");
    assert_eq!(found, Located::Found(folder.0.join("gate8-alpha-report.md").to_string_lossy().into_owned()));
}

#[test]
fn two_files_that_match_by_name_choose_none_and_say_how_many() {
    let folder = Folder::with_notes(300, &["gate8-alpha-report.md", "gate8-alpha-report-old.md"]);
    let (found, _) = find(&folder, "gate8 alpha report");
    assert_eq!(found, Located::Ambiguous(2));
}

#[test]
fn nothing_named_like_it_is_missing_not_too_many() {
    let folder = Folder::with_notes(300, &[]);
    let (found, _) = find(&folder, "gate8 alpha report");
    assert_eq!(found, Located::Missing);
}
