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
    // The tool under test runs `git commit` itself, so the identity lives in the repository's own
    // configuration: a CI runner has none of its own.
    git(&dir, &["config", "user.name", "t"]);
    git(&dir, &["config", "user.email", "t@t"]);
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

/// Review N2: git quotes a path with a space, a `"`, a `\` or a non-ASCII character, and the
/// quoted form is no secret name. Staged: two secrets by name and four by where they are.
fn quoting_repo(tag: &str) -> Option<Repo> {
    let r = repo(tag)?;
    for rel in ["Local State", "tokens (1).json", "a\"b/tokens.json", "back\\slash/tokens.json", "é/tokens.json", "my dir/id_rsa", "clé.pem"] {
        let at = r.0.join(rel);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, format!("{FAKE}\n")).unwrap();
    }
    std::fs::write(r.0.join("ok name.md"), "fine\n").unwrap();
    git(&r.0, &["add", "-A"]);
    Some(r)
}

const QUOTED_SECRETS: [&str; 7] = ["Local State", "tokens (1)", "tokens.json", "id_rsa", "clé.pem", "cl\\303\\251", "\\303\\251"];

fn assert_no_secret_named(tool: &str, out: &str) {
    assert!(out.contains("ok name.md") || tool == "stash push", "{tool}: {out}");
    assert!(!out.contains(FAKE), "{tool}: {out}");
    for name in QUOTED_SECRETS {
        assert!(!out.contains(name), "{tool} names {name}: {out}");
    }
    assert!(out.contains("entries are not shown"), "{tool}: {out}");
}

#[test]
fn a_quoted_secret_name_is_not_printed_by_status_stat_show_stash_or_commit() {
    let Some(r) = quoting_repo("quoted") else { return };
    let at = r.0.to_string_lossy().into_owned();
    let path = serde_json::json!({ "path": at });
    assert_no_secret_named("git_status", &call(&crate::git::GitStatusTool, path.clone()));
    assert_no_secret_named("git_diff --stat", &call(&crate::git::GitDiffTool, serde_json::json!({ "path": at, "staged": true })));
    let stash = call(&crate::git::GitStashTool, serde_json::json!({ "path": at, "action": "push" }));
    assert!(!stash.contains(FAKE), "{stash}");
    assert_no_secret_named("git_stash pop", &call(&crate::git::GitStashTool, serde_json::json!({ "path": at, "action": "pop" })));
    git(&r.0, &["add", "-A"]);
    let commit = call(&crate::git::GitCommitTool, serde_json::json!({ "path": at, "message": "m" }));
    assert_no_secret_named("git_commit", &commit);
    assert_no_secret_named("git_show --stat", &call(&crate::git::GitShowTool, serde_json::json!({ "path": at, "commit": "HEAD" })));
}

/// Review N6: when git cannot say what a pathspec selects, nothing is diffed.
#[test]
fn a_pathspec_git_cannot_list_is_not_diffed() {
    let refused = check_pathspec("/nonexistent-yantrik-1247-repo", "notes.md").unwrap_err();
    assert!(refused.contains("could not list") && !refused.contains(FAKE), "{refused}");
}

const GOOGLE_CLIENT: &str = "client_secret_123456789012-abcdefghijklmnopqrstuvwxyz012345.apps.googleusercontent.com.json";

/// Review R3-3: git cuts a long name from the left at its default width, and the tail
/// (`...012-abc….json`) is no secret name. No part of Google's OAuth client file name is shown.
#[test]
fn a_long_secret_name_is_not_shortened_into_view_by_stat() {
    let Some(r) = repo("stat-width") else { return };
    let at = r.0.to_string_lossy().into_owned();
    std::fs::write(r.0.join(GOOGLE_CLIENT), format!("{FAKE}\n")).unwrap();
    git(&r.0, &["add", "-A"]);
    let staged = call(&crate::git::GitDiffTool, serde_json::json!({ "path": at, "staged": true }));
    assert!(!staged.contains("googleusercontent") && !staged.contains("012345") && !staged.contains(FAKE), "{staged}");
    assert!(staged.contains("entries are not shown"), "{staged}");
    git(&r.0, &["commit", "-qm", "oauth"]);
    let shown = call(&crate::git::GitShowTool, serde_json::json!({ "path": at, "commit": "HEAD" }));
    assert!(!shown.contains("googleusercontent") && !shown.contains("012345") && !shown.contains(FAKE), "{shown}");
}

/// Review R3-3, second line: a name git did cut is judged by the changed path it ends.
#[test]
fn a_name_cut_from_the_left_is_judged_by_the_path_it_ends() {
    let cut = format!(" ...{} | 1 +\n a-readable-file.md | 1 +", &GOOGLE_CLIENT[30..]);
    let shown = without_protected_cut("/home/ann/proj", &cut, &[GOOGLE_CLIENT.to_string(), "a-readable-file.md".to_string()]);
    assert!(!shown.contains("googleusercontent") && shown.contains("a-readable-file.md"), "{shown}");
    let unrelated = without_protected_cut("/home/ann/proj", &cut, &["docs/other.md".to_string()]);
    assert!(unrelated.contains("googleusercontent"), "a tail that ends no protected path is shown: {unrelated}");
}

/// Review R3-4: the answer is cut after the names are filtered. 72 lines of 40 bytes, one of 30 and
/// one of 17 put the start of the secret's line at byte 2990 of what git prints, so a cut at 3000 lands inside `A  zz/tokens.json`.
#[test]
fn a_secret_name_at_the_cut_is_hidden_whole() {
    let Some(r) = repo("cut") else { return };
    let at = r.0.to_string_lossy().into_owned();
    std::fs::create_dir_all(r.0.join("many")).unwrap();
    std::fs::create_dir_all(r.0.join("zz")).unwrap();
    for i in 0..72 {
        std::fs::write(r.0.join(format!("many/{i:027}.txt")), "x\n").unwrap();
    }
    for filler in [format!("many/{:017}.txt", 0), "many/0000.txt".to_string()] {
        std::fs::write(r.0.join(filler), "x\n").unwrap();
    }
    std::fs::write(r.0.join("zz/tokens.json"), format!("{FAKE}\n")).unwrap();
    git(&r.0, &["add", "-A"]);
    let shown = call(&crate::git::GitStatusTool, serde_json::json!({ "path": at }));
    assert!(!shown.contains("zz/") && !shown.contains(FAKE), "{shown}");
    assert!(shown.contains("entries are not shown"), "{shown}");
}

/// Review M2: `git show` of a blob id (whole or short) or a tree prints the object's contents, which
/// no name rule sees. Only a commit is shown, and the refusal prints nothing of the object.
#[test]
fn git_show_refuses_a_blob_and_a_tree() {
    let Some(r) = repo("show-blob") else { return };
    let at = r.0.to_string_lossy().into_owned();
    git(&r.0, &["add", "-A"]);
    git(&r.0, &["commit", "-qm", "edited"]);
    let id = |rev: &str| String::from_utf8(Command::new("git").current_dir(&r.0).args(["rev-parse", rev]).output().unwrap().stdout).unwrap().trim().to_string();
    let blob = id("HEAD:credentials.json");
    for rev in [blob.clone(), blob[..7].to_string(), id("HEAD^{tree}"), "HEAD~0^{tree}".to_string(), "notacommit".to_string()] {
        let out = call(&crate::git::GitShowTool, serde_json::json!({ "path": at, "commit": rev }));
        assert!(out.starts_with("Error:") && !out.contains(FAKE) && !out.contains("old"), "{rev}: {out}");
    }
    for rev in ["HEAD", "HEAD~1", "HEAD^"] {
        let out = call(&crate::git::GitShowTool, serde_json::json!({ "path": at, "commit": rev }));
        assert!(out.contains("notes.md") && !out.starts_with("Error:"), "{rev}: {out}");
    }
}

/// Review M3: a commit's summary and a stat end a rename in ` (100%)`, so no word of the line is the
/// secret name it renames to. Through `git_commit`, for the plain rename and the `{...}` one.
#[test]
fn a_rename_onto_a_secret_name_is_not_printed_by_commit() {
    for (tag, from, to) in [("rename-plain", "notes.md", "tokens.json"), ("rename-braced", "deploy/notes.md", "deploy/tokens.json")] {
        let Some(r) = repo(tag) else { return };
        let at = r.0.to_string_lossy().into_owned();
        std::fs::write(r.0.join("deploy/notes.md"), "old\n").unwrap();
        git(&r.0, &["add", "-A"]);
        git(&r.0, &["commit", "-qm", "more"]);
        std::fs::write(r.0.join("notes.md"), "old\n").unwrap();
        git(&r.0, &["checkout", "--", "."]);
        git(&r.0, &["mv", from, to]);
        let out = call(&crate::git::GitCommitTool, serde_json::json!({ "path": at, "message": "m" }));
        assert!(out.contains("rename") || out.contains("entries are not shown"), "{tag}: {out}");
        assert!(!out.contains("tokens") && !out.contains(FAKE), "{tag}: {out}");
        assert!(out.contains("entries are not shown"), "{tag}: {out}");
    }
}

/// Review M3, the rule itself: ` (100%)` is taken off before a rename is read, and only that.
#[test]
fn a_similarity_figure_is_peeled_before_a_rename_is_read() {
    for line in [" rename notes.md => tokens.json (100%)", " rename a/{notes.md => tokens.json} (87%)", " R  notes.md -> tokens.json (100%)"] {
        let shown = without_protected("/home/ann/proj", line);
        assert!(!shown.contains("tokens"), "{line}: {shown}");
    }
    assert_eq!(without_protected("/home/ann/proj", " rename a.md => b.md (100%)"), " rename a.md => b.md (100%)");
    assert_eq!(without_similarity("done (100%)"), "done");
    assert_eq!(without_similarity("done (x%)"), "done (x%)");
}
