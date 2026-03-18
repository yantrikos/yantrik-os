//! Project tools — detect git repos, set active project, get project context.
//!
//! Tools: detect_projects, set_active_project, get_project_context.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path, expand_home};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DetectProjectsTool));
    reg.register(Box::new(SetActiveProjectTool));
    reg.register(Box::new(GetProjectContextTool));
}

/// Read the current branch from a `.git/HEAD` file.
fn read_branch(git_head_path: &str) -> String {
    match std::fs::read_to_string(git_head_path) {
        Ok(head) => head
            .strip_prefix("ref: refs/heads/")
            .unwrap_or(&head)
            .trim()
            .to_string(),
        Err(_) => "(unknown)".to_string(),
    }
}

// ── Detect Projects ──

pub struct DetectProjectsTool;

impl Tool for DetectProjectsTool {
    fn name(&self) -> &'static str { "detect_projects" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "project" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "detect_projects",
                "description": "Find git repositories under a directory",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "base_dir": {"type": "string", "description": "Directory to scan (default: ~/)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let base_dir = args.get("base_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("~/");
        let expanded = expand_home(base_dir);

        let start = std::time::Instant::now();
        let mut repos = Vec::new();

        // Walk up to 2 levels deep
        let level0 = match std::fs::read_dir(&expanded) {
            Ok(entries) => entries,
            Err(e) => return format!("Error reading {}: {}", expanded, e),
        };

        'outer: for entry in level0.flatten() {
            if start.elapsed().as_secs() >= 2 || repos.len() >= 20 {
                break;
            }

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Check level 1
            let git_dir = path.join(".git");
            if git_dir.is_dir() {
                let branch = read_branch(&git_dir.join("HEAD").to_string_lossy());
                let dirty = count_dirty(&path.to_string_lossy());
                repos.push(format!(
                    "  {:<40} branch: {:<20} dirty: {}",
                    path.display(), branch, dirty
                ));
                continue;
            }

            // Check level 2
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                    if start.elapsed().as_secs() >= 2 || repos.len() >= 20 {
                        break 'outer;
                    }

                    let sub_path = sub_entry.path();
                    if !sub_path.is_dir() {
                        continue;
                    }

                    let git_dir = sub_path.join(".git");
                    if git_dir.is_dir() {
                        let branch = read_branch(&git_dir.join("HEAD").to_string_lossy());
                        let dirty = count_dirty(&sub_path.to_string_lossy());
                        repos.push(format!(
                            "  {:<40} branch: {:<20} dirty: {}",
                            sub_path.display(), branch, dirty
                        ));
                    }
                }
            }
        }

        if repos.is_empty() {
            format!("No git repositories found under {}", expanded)
        } else {
            let mut out = format!("Git repositories under {} ({} found):\n", expanded, repos.len());
            for r in &repos {
                out.push_str(&format!("{}\n", r));
            }
            out
        }
    }
}

/// Count dirty files in a git repo using `git status --short`.
fn count_dirty(path: &str) -> String {
    match std::process::Command::new("git")
        .current_dir(path)
        .args(["status", "--short"])
        .output()
    {
        Ok(o) if o.status.success() => {
            let out = String::from_utf8_lossy(&o.stdout);
            let count = out.lines().count();
            format!("{}", count)
        }
        _ => "?".to_string(),
    }
}

// ── Set Active Project ──

pub struct SetActiveProjectTool;

impl Tool for SetActiveProjectTool {
    fn name(&self) -> &'static str { "set_active_project" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "project" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "set_active_project",
                "description": "Mark a project as your active working context",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the project directory"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        if path.is_empty() {
            return "Error: path is required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Check it's a real directory
        if !std::path::Path::new(&expanded).is_dir() {
            return format!("Error: '{}' is not a directory", expanded);
        }

        // Read branch if it's a git repo
        let branch = {
            let head_path = format!("{}/.git/HEAD", expanded);
            if std::path::Path::new(&head_path).exists() {
                read_branch(&head_path)
            } else {
                "(not a git repo)".to_string()
            }
        };

        let text = format!("Active project set to: {} (branch: {})", expanded, branch);

        match ctx.db.record_text(
            &text,
            "episodic",
            0.7,
            0.0,
            604800.0,
            &serde_json::json!({"type": "active_project", "path": expanded}),
            "default",
            0.9,
            "work/project",
            "companion",
            None,
        ) {
            Ok(rid) => format!("{} (memory #{rid})", text),
            Err(e) => format!("Failed to set active project: {e}"),
        }
    }
}

// ── Get Project Context ──

pub struct GetProjectContextTool;

impl Tool for GetProjectContextTool {
    fn name(&self) -> &'static str { "get_project_context" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "project" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "get_project_context",
                "description": "Get context about a project: branch, recent commits",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the project directory"}
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

        if !std::path::Path::new(&expanded).is_dir() {
            return format!("Error: '{}' is not a directory", expanded);
        }

        let mut out = format!("Project: {}\n", expanded);

        // Branch
        let head_path = format!("{}/.git/HEAD", expanded);
        if std::path::Path::new(&head_path).exists() {
            let branch = read_branch(&head_path);
            out.push_str(&format!("Branch: {}\n", branch));
        } else {
            out.push_str("Branch: (not a git repo)\n");
        }

        // Recent commits
        if let Ok(o) = std::process::Command::new("git")
            .current_dir(&expanded)
            .args(["log", "--oneline", "-5"])
            .output()
        {
            if o.status.success() {
                let log = String::from_utf8_lossy(&o.stdout);
                out.push_str(&format!("\nRecent commits:\n{}\n", log.trim()));
            }
        }

        // Detect language
        let mut languages = Vec::new();
        let markers = [
            ("Cargo.toml", "Rust"),
            ("package.json", "JavaScript/TypeScript"),
            ("requirements.txt", "Python"),
            ("go.mod", "Go"),
            ("pom.xml", "Java"),
        ];
        for (file, lang) in &markers {
            let check = format!("{}/{}", expanded, file);
            if std::path::Path::new(&check).exists() {
                languages.push(*lang);
            }
        }

        if !languages.is_empty() {
            out.push_str(&format!("Languages: {}\n", languages.join(", ")));
        } else {
            out.push_str("Languages: (not detected)\n");
        }

        out
    }
}
