//! Git tools — git_status, git_log, git_diff, git_clone.
//! Read-heavy: most operations are Safe. Only clone writes to disk.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(GitStatusTool));
    reg.register(Box::new(GitLogTool));
    reg.register(Box::new(GitDiffTool));
    reg.register(Box::new(GitCloneTool));
    reg.register(Box::new(GitBranchTool));
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
                format!("{}...\n(truncated, {} chars)", &out[..3000], out.len())
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
                "description": "Show the working tree status of a git repository.",
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
                "description": "Show recent git commit history.",
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
                "description": "Show uncommitted changes in a git repository.",
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
                "description": "Clone a git repository to a local directory.",
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

        let expanded = match validate_path(dest) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        let mut cmd = std::process::Command::new("git");
        cmd.arg("clone");
        if shallow {
            cmd.args(["--depth", "1"]);
        }
        cmd.arg(url).arg(&expanded);

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
                "description": "List branches in a git repository.",
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
