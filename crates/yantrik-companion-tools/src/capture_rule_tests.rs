//! The person's screenshots and recordings are no tool's to read (security review of #828,
//! finding 1): `~/Pictures/Screenshots` and `~/Videos/Recordings` are on the desktop's shared
//! protected list, and a tool that walks or packs a folder above them leaves them out or refuses,
//! as a tool given one of them by name does.
//!
//! `validate_path` only works inside the real home, so the tree is made there, under a name of
//! this test run's own, and removed however the test ends.

use super::{Tool, ToolContext, PermissionLevel};

struct Gone(std::path::PathBuf);
impl Drop for Gone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(tool: &dyn Tool, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext {
        db: &db,
        max_permission: PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    tool.execute(&ctx, &args)
}

#[test]
fn no_tool_reads_walks_or_packs_the_persons_captures() {
    let home = std::path::PathBuf::from(std::env::var("HOME").expect("HOME"));
    let base = home.join(format!("yantrik-capture-rule-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let _gone = Gone(base.clone());
    let shots = base.join("Pictures/Screenshots");
    std::fs::create_dir_all(&shots).unwrap();
    std::fs::create_dir_all(base.join("Videos/Recordings")).unwrap();
    // Text, so a regression shows up in grep's and search_files' answers rather than hiding
    // behind a PNG that is not UTF-8.
    let shot = shots.join("Screenshot 2026-10-15 at 14.32.07.png");
    std::fs::write(&shot, "needle on the desktop").unwrap();
    std::fs::write(base.join("Videos/Recordings/Recording 2026-10-15 at 14.32.07.mp4"), "needle").unwrap();
    std::fs::write(base.join("Pictures/holiday.png"), "needle at the beach").unwrap();
    let at = |p: &std::path::Path| p.to_string_lossy().into_owned();

    let said = run(&crate::files::FileInfoTool, serde_json::json!({ "path": at(&shot) }));
    assert!(said.starts_with("Error") && said.contains("protected"), "file_info: {said}");

    let said = run(&crate::grep::GrepTool, serde_json::json!({ "pattern": "needle", "path": at(&base) }));
    assert!(said.contains("holiday.png"), "grep still searches the person's other files: {said}");
    assert!(!said.contains("Screenshot") && !said.contains("Recording"), "grep: {said}");

    let said = run(&crate::glob::GlobTool, serde_json::json!({ "pattern": "**/*.png", "path": at(&base) }));
    assert!(said.contains("holiday.png"), "glob: {said}");
    assert!(!said.contains("Screenshot") && !said.contains("Recording"), "glob: {said}");

    let said = run(&crate::files::SearchFilesTool, serde_json::json!({ "query": "needle", "path": at(&base) }));
    assert!(said.contains("holiday.png"), "search_files: {said}");
    assert!(!said.contains("Screenshot") && !said.contains("Recording"), "search_files: {said}");

    let out = base.join("out.tar.gz");
    let said = run(
        &crate::archive::ArchiveCreateTool,
        serde_json::json!({ "output_path": at(&out), "source_paths": [at(&base.join("Pictures"))] }),
    );
    assert!(said.starts_with("Error") && said.contains("protected"), "archive_create: {said}");
    assert!(!out.exists(), "nothing was packed");

    assert!(crate::safe_command::protected(&at(&shot)), "run_command's cat, head, file …");
    assert!(!crate::safe_command::protected(&at(&base.join("Pictures/holiday.png"))));
}
