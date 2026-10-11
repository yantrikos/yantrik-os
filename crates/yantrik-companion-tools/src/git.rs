//! Git tools — git_status, git_log, git_diff, git_clone, git_branch,
//! git_commit, git_show, git_stash, git_diff_file.
//! Read-heavy: most operations are Safe. Clone/commit/stash write to disk.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path, validate_write_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(GitStatusTool));
    reg.register(Box::new(GitLogTool));
    reg.register(Box::new(GitDiffTool));
    reg.register(Box::new(GitCloneTool));
    reg.register(Box::new(GitBranchTool));
    reg.register(Box::new(GitCommitTool));
    reg.register(Box::new(GitShowTool));
    reg.register(Box::new(GitStashTool));
    reg.register(Box::new(GitDiffFileTool));
}

/// Run a git command in a validated directory.
fn run_git(dir: &str, git_args: &[&str]) -> String {
    match std::process::Command::new("git")
        .current_dir(dir)
        .args(git_args)
        .output()
    {
        Ok(o) if o.status.success() => {
            let out = String::from_utf8_lossy(&o.stdout);
            if out.trim().is_empty() {
                "(no output)".to_string()
            } else if out.len() > 3000 {
                format!("{}...\n(truncated, {} chars)", &out[..out.floor_char_boundary(3000)], out.len())
            } else {
                out.to_string()
            }
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            format!("{} {}", out.trim(), err.trim())
        }
        Err(e) => format!("Error (git not available?): {e}"),
    }
}

// ── Git Status ──

pub struct GitStatusTool;

impl Tool for GitStatusTool {
    fn name(&self) -> &'static str { "git_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_status",
                "description": "Show the working tree status of a git repository",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path (default: ~/*)"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        if path.is_empty() {
            return "Error: path is required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };
        run_git(&expanded, &["status", "--short", "--branch"])
    }
}

// ── Git Log ──

pub struct GitLogTool;

impl Tool for GitLogTool {
    fn name(&self) -> &'static str { "git_log" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_log",
                "description": "Show recent git commit history",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path"},
                        "count": {"type": "integer", "description": "Number of commits (default: 10, max: 50)"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10).min(50);

        if path.is_empty() {
            return "Error: path is required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        let n = format!("-{}", count);
        run_git(&expanded, &["log", "--oneline", "--graph", &n])
    }
}

// ── Git Diff ──

pub struct GitDiffTool;

impl Tool for GitDiffTool {
    fn name(&self) -> &'static str { "git_diff" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_diff",
                "description": "Show uncommitted changes in a git repository",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path"},
                        "staged": {"type": "boolean", "description": "Show staged changes only"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let staged = args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);

        if path.is_empty() {
            return "Error: path is required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        if staged {
            run_git(&expanded, &["diff", "--cached", "--stat"])
        } else {
            run_git(&expanded, &["diff", "--stat"])
        }
    }
}

// ── Git Clone ──

/// Build the `git clone` command for a clone the model asked for.
///
/// A repository can carry symlinks pointing anywhere the person can write (e.g.
/// `~/.config/autostart/x.desktop`), and any later writer that follows links under the clone
/// would be steered through them past the home_paths rule (#664). `core.symlinks=false` makes
/// git materialise each link as a small plain file holding the target text instead, closing the
/// planting at its source; the `clone --config` form also writes it into the new repo's
/// `.git/config`, so later checkouts/pulls in that repo stay link-free too.
fn clone_command(url: &str, dest: &str, shallow: bool) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("clone").arg("--config").arg("core.symlinks=false");
    if shallow {
        cmd.args(["--depth", "1"]);
    }
    cmd.arg("--").arg(url).arg(dest);
    cmd
}

pub struct GitCloneTool;

impl Tool for GitCloneTool {
    fn name(&self) -> &'static str { "git_clone" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_clone",
                "description": "Clone a git repository to a local directory",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "Repository URL (https://)"},
                        "destination": {"type": "string", "description": "Local path (e.g. ~/Projects/repo)"},
                        "shallow": {"type": "boolean", "description": "Shallow clone (--depth 1)"}
                    },
                    "required": ["url", "destination"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let dest = args.get("destination").and_then(|v| v.as_str()).unwrap_or_default();
        let shallow = args.get("shallow").and_then(|v| v.as_bool()).unwrap_or(false);

        if url.is_empty() || dest.is_empty() {
            return "Error: url and destination are required".to_string();
        }

        if !url.starts_with("https://") && !url.starts_with("git@") {
            return "Error: URL must start with https:// or git@".to_string();
        }

        // Block metacharacters in URL
        if url.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
            return "Error: URL contains invalid characters".to_string();
        }

        // A clone writes a whole tree, .git/config (which git runs from) included.
        let expanded = match validate_write_path(dest) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        let mut cmd = clone_command(url, &expanded, shallow);

        match cmd.output() {
            Ok(o) if o.status.success() => {
                format!("Cloned {url} → {dest}")
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("Clone failed: {}", err.trim())
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Git Branch ──

pub struct GitBranchTool;

impl Tool for GitBranchTool {
    fn name(&self) -> &'static str { "git_branch" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_branch",
                "description": "List branches in a git repository",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path"},
                        "all": {"type": "boolean", "description": "Show remote branches too"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        if path.is_empty() {
            return "Error: path is required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        if all {
            run_git(&expanded, &["branch", "-a", "-v"])
        } else {
            run_git(&expanded, &["branch", "-v"])
        }
    }
}

// ── Git Commit ──

pub struct GitCommitTool;

impl Tool for GitCommitTool {
    fn name(&self) -> &'static str { "git_commit" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_commit",
                "description": "Commit changes in a git repository",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path"},
                        "message": {"type": "string", "description": "Commit message"},
                        "stage_all": {"type": "boolean", "description": "Stage all changes before committing (git add -A)"}
                    },
                    "required": ["path", "message"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let message = args.get("message").and_then(|v| v.as_str()).unwrap_or_default();
        let stage_all = args.get("stage_all").and_then(|v| v.as_bool()).unwrap_or(false);

        if path.is_empty() || message.is_empty() {
            return "Error: path and message are required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Stage all if requested
        if stage_all {
            let stage_result = run_git(&expanded, &["add", "-A"]);
            if stage_result.contains("Error") {
                return format!("Failed to stage: {}", stage_result);
            }
        }

        run_git(&expanded, &["commit", "-m", message])
    }
}

// ── Git Show ──

pub struct GitShowTool;

impl Tool for GitShowTool {
    fn name(&self) -> &'static str { "git_show" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_show",
                "description": "Show details of a specific git commit",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path"},
                        "commit": {"type": "string", "description": "Commit ref (hash, HEAD, tag, etc.)"}
                    },
                    "required": ["path", "commit"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let commit = args.get("commit").and_then(|v| v.as_str()).unwrap_or_default();

        if path.is_empty() || commit.is_empty() {
            return "Error: path and commit are required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Validate commit ref: only alphanumeric + ^~/ allowed
        if !commit.chars().all(|c| c.is_ascii_alphanumeric() || c == '^' || c == '~') {
            return "Error: invalid commit ref (only alphanumeric, ^, ~ allowed)".to_string();
        }

        let out = run_git(&expanded, &["show", "--stat", commit]);
        if out.len() > 3000 {
            format!("{}...\n(truncated, {} chars)", &out[..out.floor_char_boundary(3000)], out.len())
        } else {
            out
        }
    }
}

// ── Git Stash ──

pub struct GitStashTool;

impl Tool for GitStashTool {
    fn name(&self) -> &'static str { "git_stash" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_stash",
                "description": "Manage git stash (push, pop, or list)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path"},
                        "action": {
                            "type": "string",
                            "description": "Stash action: push, pop, or list",
                            "enum": ["push", "pop", "list"]
                        }
                    },
                    "required": ["path", "action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or_default();

        if path.is_empty() || action.is_empty() {
            return "Error: path and action are required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        match action {
            "push" | "pop" | "list" => run_git(&expanded, &["stash", action]),
            _ => format!("Error: invalid stash action '{}' (use push, pop, or list)", action),
        }
    }
}

// ── Git Diff File ──

pub struct GitDiffFileTool;

impl Tool for GitDiffFileTool {
    fn name(&self) -> &'static str { "git_diff_file" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "git" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_diff_file",
                "description": "Show diff for a specific file in a git repository",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Repository path"},
                        "file": {"type": "string", "description": "File path relative to the repository root"}
                    },
                    "required": ["path", "file"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let file = args.get("file").and_then(|v| v.as_str()).unwrap_or_default();

        if path.is_empty() || file.is_empty() {
            return "Error: path and file are required".to_string();
        }
        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        let out = run_git(&expanded, &["diff", "--", file]);
        if out.len() > 3000 {
            format!("{}...\n(truncated, {} chars)", &out[..out.floor_char_boundary(3000)], out.len())
        } else {
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_does_not_plant_symlinks() {
        // Skip only when git is not installed; the test itself needs no symlink support.
        if std::process::Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipping: git is not on PATH");
            return;
        }

        let dir = std::env::temp_dir().join(format!("yantrik-git-clone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();

        let src = dir.join("src");
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .current_dir(&src)
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} succeeded");
        };
        run(&["init"]);
        std::fs::write(src.join("normal.txt"), "hello").unwrap();
        let mut hash = std::process::Command::new("git")
            .current_dir(&src)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        hash.stdin.take().unwrap().write_all(b"/tmp/somewhere-outside").unwrap();
        let blob = hash.wait_with_output().unwrap();
        assert!(blob.status.success());
        let blob = String::from_utf8(blob.stdout).unwrap();
        let blob = blob.trim();
        // A symlink entry (mode 120000) committed without needing symlink support on the host.
        run(&["update-index", "--add", "--cacheinfo", &format!("120000,{blob},evil_link")]);
        run(&["add", "normal.txt"]);
        run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "x"]);

        let dest = dir.join("dest");
        let mut cmd = clone_command(src.to_str().unwrap(), dest.to_str().unwrap(), false);
        // The command must carry the config that turns links into plain files.
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.iter().any(|a| a == "core.symlinks=false"), "args: {args:?}");

        let out = cmd.output().unwrap();
        assert!(out.status.success(), "clone failed: {}", String::from_utf8_lossy(&out.stderr));

        assert!(dest.join("normal.txt").is_file());
        let meta = std::fs::symlink_metadata(dest.join("evil_link")).unwrap();
        assert!(!meta.file_type().is_symlink(), "evil_link must not be a symlink");
        assert!(meta.file_type().is_file(), "evil_link must be a plain file");
        // The unfixed command (no core.symlinks=false) would leave evil_link a symlink.

        let _ = std::fs::remove_dir_all(&dir);
    }
}
