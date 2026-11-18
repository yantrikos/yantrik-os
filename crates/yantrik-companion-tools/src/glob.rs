//! glob — recursive file pattern matching.
//!
//! Inspired by Claude Code's Glob tool: find files by patterns
//! like "**/*.rs", "src/**/*.ts", etc.

use serde_json::Value;
use super::{Tool, ToolContext, PermissionLevel, validate_path};

pub fn register(reg: &mut super::ToolRegistry) {
    reg.register(Box::new(GlobTool));
}

pub(crate) struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &'static str { "glob" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "files" }

    fn definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "glob",
                "description": "Find files by filename pattern recursively",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern (e.g. '**/*.rs', 'src/**/*.py', '*.toml', 'Cargo.*')"
                        },
                        "path": {
                            "type": "string",
                            "description": "Base directory to search from (default: current working directory or ~)"
                        }
                    },
                    "required": ["pattern"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or_default();
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("~");

        if pattern.is_empty() {
            return "Error: pattern is required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Parse the glob pattern
        let (dir_prefix, file_pattern, recursive) = parse_glob_pattern(pattern);

        // Construct search directory
        let search_dir = if dir_prefix.is_empty() {
            expanded.clone()
        } else {
            format!("{}/{}", expanded, dir_prefix)
        };

        let search_path = std::path::Path::new(&search_dir);
        if !search_path.is_dir() {
            return format!("Error: directory '{}' does not exist", search_dir);
        }

        let mut results: Vec<(String, std::time::SystemTime)> = Vec::new();
        collect_matching_files(
            &search_dir, &file_pattern, recursive, &mut results, 0,
        );

        if results.is_empty() {
            return format!("No files matching '{}' in {}", pattern, path);
        }

        // Sort by modification time (newest first)
        results.sort_by(|a, b| b.1.cmp(&a.1));

        let total = results.len();
        results.truncate(50);

        let mut output = String::new();
        for (file_path, _) in &results {
            output.push_str(file_path);
            output.push('\n');
        }
        if total > 50 {
            output.push_str(&format!("... and {} more files\n", total - 50));
        }
        output.push_str(&format!("\n{} file(s) found", total));
        output
    }
}

/// Parse a glob pattern into (directory_prefix, file_pattern, recursive).
///
/// Examples:
/// - `**/*.rs` → ("", "*.rs", true)
/// - `src/**/*.py` → ("src", "*.py", true)
/// - `*.toml` → ("", "*.toml", false)
/// - `Cargo.*` → ("", "Cargo.*", false)
fn parse_glob_pattern(pattern: &str) -> (String, String, bool) {
    let pattern = pattern.replace('\\', "/");

    // Check for **/ recursive marker
    if let Some(pos) = pattern.find("**/") {
        let dir_prefix = if pos > 0 {
            pattern[..pos].trim_end_matches('/').to_string()
        } else {
            String::new()
        };
        let file_pattern = pattern[pos + 3..].to_string();
        return (dir_prefix, file_pattern, true);
    }

    // Check for directory component
    if let Some(pos) = pattern.rfind('/') {
        let dir_prefix = pattern[..pos].to_string();
        let file_pattern = pattern[pos + 1..].to_string();
        return (dir_prefix, file_pattern, false);
    }

    // Simple pattern in current directory
    (String::new(), pattern.to_string(), false)
}

/// Match a filename against a simple glob pattern.
fn matches_pattern(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    // Handle *.ext
    if let Some(ext) = pattern.strip_prefix("*.") {
        return name.ends_with(&format!(".{}", ext));
    }

    // Handle prefix.*
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return name.starts_with(&format!("{}.", prefix));
    }

    // Handle prefix*
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }

    // Handle *suffix
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }

    // Exact match
    pattern == name
}

fn collect_matching_files(
    dir: &str,
    file_pattern: &str,
    recursive: bool,
    results: &mut Vec<(String, std::time::SystemTime)>,
    depth: usize,
) {
    if depth > 10 || results.len() >= 500 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common noise, and the protected places below the folder asked
        // for, which validate_path checked only at the top. A link is not followed, so the
        // names in a protected tree are not listed through one (security review of #942).
        if name.starts_with('.') || name == "node_modules" || name == "target"
            || name == "__pycache__"
            || entry.file_type().map_or(true, |kind| kind.is_symlink())
            || yantrik_ipc_contracts::home_paths::is_protected(&path)
        {
            continue;
        }

        if path.is_dir() {
            if recursive {
                collect_matching_files(
                    &path.to_string_lossy(), file_pattern, recursive,
                    results, depth + 1,
                );
            }
        } else if matches_pattern(file_pattern, &name) {
            let mtime = entry.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            results.push((path.to_string_lossy().to_string(), mtime));
        }
    }
}

#[cfg(all(test, unix))]
mod link_tests {
    /// A link met while recursing is not followed, so the names in a protected tree are not
    /// listed through one (security review of #942, finding 2).
    #[test]
    fn glob_does_not_list_through_a_link_found_below_the_folder() {
        let root = std::env::temp_dir().join(format!("yantrik-glob-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tree")).unwrap();
        std::fs::create_dir_all(root.join("parent")).unwrap();
        std::fs::write(root.join("tree/snippets.json"), "{}").unwrap();
        std::fs::write(root.join("parent/own.json"), "{}").unwrap();
        std::os::unix::fs::symlink(root.join("tree"), root.join("parent/lnk")).unwrap();
        let mut found = Vec::new();
        super::collect_matching_files(&root.join("parent").to_string_lossy(), "*.json", true, &mut found, 0);
        let _ = std::fs::remove_dir_all(&root);
        let names: Vec<_> = found.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].ends_with("own.json"), "{names:?}");
    }
}
