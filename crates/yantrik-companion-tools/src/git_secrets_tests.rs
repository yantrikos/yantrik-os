//! #1247 review H2: the git tools never print a protected file or name it. Fake contents only.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

const FAKE: &str = "FAKE-SECRET-DO-NOT-PRINT";

struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").current_dir(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).status();
    assert!(ok.is_ok_and(|s| s.success()), "git {args:?}");
}

/// A repository that tracks `credentials.json`, `deploy/server.pem` and `notes.md`, each with a
/// local edit. `None` when git is not installed or there is no scratch directory.
fn repo(tag: &str) -> Option<Repo> {
    Command::new("git").arg("--version").output().ok()?;
    // In the tools' scratch directory, where `validate_path` lets a tool look.
    let dir = yantrik_ml::private_dir::scratch_dir().ok()?.join(format!("yantrik-1247-git-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("deploy")).unwrap();
    let dir = dir.canonicalize().unwrap();
    git(&dir, &["init", "-q"]);
    for rel in ["credentials.json", "deploy/server.pem", "notes.md"] {
        std::fs::write(dir.join(rel), "old\n").unwrap();
    }
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "x"]);
    for rel in ["credentials.json", "deploy/server.pem", "notes.md"] {
        std::fs::write(dir.join(rel), format!("{FAKE}\n")).unwrap();
    }
    Some(Repo(dir))
}

#[test]
fn a_tracked_secret_is_not_diffed_by_name_folder_dot_or_glob() {
    let Some(r) = repo("diff") else { return };
    let at = r.0.to_string_lossy().into_owned();
    for file in ["credentials.json", "deploy/server.pem", "deploy", "*.json", "*", "deploy/*", ".", ":(glob)**", ":/", "-p", "--"] {
        let refused = check_pathspec(&at, file).unwrap_err();
        assert!(!refused.contains(FAKE), "{file}: {refused}");
    }
    assert_eq!(check_pathspec(&at, "notes.md"), Ok(()));
}

#[test]
fn a_link_to_a_secret_in_the_repository_is_not_diffed() {
    let Some(r) = repo("link") else { return };
    let at = r.0.to_string_lossy().into_owned();
    std::os::unix::fs::symlink(r.0.join("credentials.json"), r.0.join("shortcut.txt")).unwrap();
    assert!(check_pathspec(&at, "shortcut.txt").is_err());
}

#[test]
fn a_status_or_stat_does_not_name_a_protected_file() {
    let Some(r) = repo("names") else { return };
    let at = r.0.to_string_lossy().into_owned();
    let out = |args: &[&str]| String::from_utf8(Command::new("git").current_dir(&r.0).args(args).output().unwrap().stdout).unwrap();
    for raw in [out(&["status", "--short", "--branch"]), out(&["diff", "--stat"]), out(&["show", "--stat", "--format=", "HEAD"])] {
        assert!(raw.contains("credentials.json") && raw.contains("server.pem"), "the raw output holds them: {raw}");
        let shown = without_protected(&at, &raw);
        assert!(shown.contains("notes.md"), "{shown}");
        assert!(!shown.contains("credentials.json") && !shown.contains("server.pem"), "{shown}");
        assert!(shown.contains("2 entries are not shown"), "{shown}");
    }
}

#[test]
fn a_rename_that_ends_in_a_secret_name_is_hidden() {
    let shown = without_protected("/home/ann/proj", " R  a/x.txt -> a/tokens.json\n R  a/{tokens.json => x.txt} | 0\n M ok.txt");
    assert_eq!(shown, " M ok.txt\n(2 entries are not shown: each is, or is inside, a place or file the AI never reads)");
}

fn call(tool: &dyn crate::Tool, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = crate::ToolContext {
        db: &db,
        max_permission: crate::PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    tool.execute(&ctx, &args)
}

/// Through the tools a Mind calls, not only the rule: `git_diff_file` prints no secret line
/// however the file is asked for, and the decoy beside it is still diffed.
#[test]
fn git_diff_file_refuses_a_secret_and_still_diffs_a_decoy() {
    let Some(r) = repo("tool") else { return };
    let at = r.0.to_string_lossy().into_owned();
    for file in ["credentials.json", "deploy/server.pem", "deploy", ".", "*.json", ":(glob)**", "-p"] {
        let out = call(&crate::git::GitDiffFileTool, serde_json::json!({ "path": at, "file": file }));
        assert!(out.starts_with("Error:") && !out.contains(FAKE), "{file}: {out}");
    }
    let decoy = call(&crate::git::GitDiffFileTool, serde_json::json!({ "path": at, "file": "notes.md" }));
    assert!(decoy.contains(FAKE), "an ordinary file is diffed: {decoy}");
}

#[test]
fn git_status_and_diff_stat_name_no_secret() {
    let Some(r) = repo("tool-names") else { return };
    let at = r.0.to_string_lossy().into_owned();
    for (tool, args) in [
        (&crate::git::GitStatusTool as &dyn crate::Tool, serde_json::json!({ "path": at })),
        (&crate::git::GitDiffTool, serde_json::json!({ "path": at })),
        (&crate::git::GitShowTool, serde_json::json!({ "path": at, "commit": "HEAD" })),
    ] {
        let out = call(tool, args);
        assert!(out.contains("notes.md"), "{out}");
        assert!(!out.contains("credentials.json") && !out.contains("server.pem"), "{out}");
    }
}
