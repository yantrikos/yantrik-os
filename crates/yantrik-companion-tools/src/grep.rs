//! grep — regex-powered content search with context lines.
//!
//! Inspired by Claude Code's Grep tool (built on ripgrep concepts):
//! regex patterns, context lines, output modes, file type filtering.

use serde_json::Value;
use super::{Tool, ToolContext, PermissionLevel, validate_path, glob_match};

pub fn register(reg: &mut super::ToolRegistry) {
    reg.register(Box::new(GrepTool));
}

pub(crate) struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &'static str { "grep" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "files" }

    fn definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search file contents with regex",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to search for (e.g. 'fn\\s+\\w+', 'TODO|FIXME', 'error')"
                        },
                        "path": {
                            "type": "string",
                            "description": "File or directory to search in"
                        },
                        "glob": {
                            "type": "string",
                            "description": "Glob filter for file names (e.g. '*.rs', '*.py')"
                        },
                        "context": {
                            "type": "integer",
                            "description": "Number of context lines before and after each match (default: 0)"
                        },
                        "case_insensitive": {
                            "type": "boolean",
                            "description": "Case-insensitive search (default: false)"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of matches to return (default: 30)"
                        },
                        "files_only": {
                            "type": "boolean",
                            "description": "Only return file paths that contain matches, not the matching lines (default: false)"
                        }
                    },
                    "required": ["pattern", "path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or_default();
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let glob_filter = args.get("glob").and_then(|v| v.as_str()).unwrap_or("*");
        let context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let case_insensitive = args.get("case_insensitive").and_then(|v| v.as_bool()).unwrap_or(false);
        let max_results = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
        let files_only = args.get("files_only").and_then(|v| v.as_bool()).unwrap_or(false);

        if pattern.is_empty() || path.is_empty() {
            return "Error: pattern and path are required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Compile regex
        let regex_pattern = if case_insensitive {
            format!("(?i){}", pattern)
        } else {
            pattern.to_string()
        };
        let re = match regex::Regex::new(&regex_pattern) {
            Ok(r) => r,
            Err(e) => return format!("Error: invalid regex pattern: {e}"),
        };

        let exp_path = std::path::Path::new(&expanded);

        if exp_path.is_file() {
            // Search single file
            return search_file(&re, &expanded, context, max_results, files_only);
        }

        if !exp_path.is_dir() {
            return format!("Error: '{path}' is not a file or directory");
        }

        // Search directory recursively
        let mut all_matches = Vec::new();
        let mut matched_files = Vec::new();
        search_dir_recursive(
            &re, &expanded, glob_filter, context, max_results,
            files_only, &mut all_matches, &mut matched_files, 0,
        );

        if files_only {
            if matched_files.is_empty() {
                format!("No files matching pattern '{}' in {}", pattern, path)
            } else {
                let count = matched_files.len();
                matched_files.truncate(50);
                let mut result = format!("{} file(s) contain matches:\n", count);
                for f in &matched_files {
                    result.push_str(&format!("  {f}\n"));
                }
                result
            }
        } else {
            if all_matches.is_empty() {
                format!("No matches for '{}' in {}", pattern, path)
            } else {
                let total = all_matches.len();
                all_matches.truncate(max_results);
                let mut result = String::new();
                for m in &all_matches {
                    result.push_str(m);
                    result.push('\n');
                }
                if total > max_results {
                    result.push_str(&format!("... {} more matches (use max_results to see more)\n", total - max_results));
                }
                result
            }
        }
    }
}

fn search_file(
    re: &regex::Regex,
    file_path: &str,
    context: usize,
    max_results: usize,
    files_only: bool,
) -> String {
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => return format!("Error reading {file_path}: {e}"),
    };

    let lines: Vec<&str> = content.lines().collect();
    let mut matches = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            if files_only {
                return format!("{file_path}");
            }

            if context > 0 {
                let start = i.saturating_sub(context);
                let end = (i + context + 1).min(lines.len());
                for j in start..end {
                    let marker = if j == i { ">" } else { " " };
                    matches.push(format!("{file_path}:{}{} {}", j + 1, marker, lines[j]));
                }
                matches.push("--".to_string());
            } else {
                matches.push(format!("{file_path}:{}: {}", i + 1, line));
            }

            if matches.len() >= max_results {
                break;
            }
        }
    }

    if matches.is_empty() {
        format!("No matches in {file_path}")
    } else {
        matches.join("\n")
    }
}

fn search_dir_recursive(
    re: &regex::Regex,
    dir: &str,
    glob_filter: &str,
    context: usize,
    max_results: usize,
    files_only: bool,
    all_matches: &mut Vec<String>,
    matched_files: &mut Vec<String>,
    depth: usize,
) {
    if depth > 8 || all_matches.len() >= max_results {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common noise, and every protected place below the folder asked
        // for: validate_path checked only the top, and ~/Pictures held the person's screenshots
        // (security review of #828, finding 1). A link is not followed, as `grep -r` does not:
        // checked as written, a link in the home into a protected tree was read through
        // (security review of #942, finding 2).
        if name.starts_with('.') || name == "node_modules" || name == "target"
            || name == "__pycache__" || name == ".git"
            || entry.file_type().map_or(true, |kind| kind.is_symlink())
            || yantrik_ipc_contracts::home_paths::is_protected(&path)
        {
            continue;
        }

        if path.is_dir() {
            search_dir_recursive(
                re, &path.to_string_lossy(), glob_filter, context,
                max_results, files_only, all_matches, matched_files, depth + 1,
            );
        } else if glob_filter == "*" || glob_match(glob_filter, &name) {
            // Skip large files (>1MB) and binary files
            let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
            if size > 1_048_576 || size == 0 {
                continue;
            }

            let file_path = path.to_string_lossy().to_string();
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue, // Skip binary/unreadable files
            };

            let lines: Vec<&str> = content.lines().collect();
            let mut file_has_match = false;

            for (i, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    if !file_has_match {
                        file_has_match = true;
                        matched_files.push(file_path.clone());
                    }

                    if !files_only {
                        if context > 0 {
                            let start = i.saturating_sub(context);
                            let end = (i + context + 1).min(lines.len());
                            for j in start..end {
                                let marker = if j == i { ">" } else { " " };
                                all_matches.push(format!("{file_path}:{}{} {}", j + 1, marker, lines[j]));
                            }
                            all_matches.push("--".to_string());
                        } else {
                            all_matches.push(format!("{file_path}:{}: {}", i + 1, line));
                        }
                    }

                    if all_matches.len() >= max_results {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(all(test, unix))]
mod link_tests {
    /// A link met while recursing is not followed: one in the home into a protected tree was read
    /// through, since each entry was checked as written (security review of #942, finding 2).
    #[test]
    fn grep_does_not_read_through_a_link_found_below_the_folder() {
        let root = std::env::temp_dir().join(format!("yantrik-grep-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tree")).unwrap();
        std::fs::create_dir_all(root.join("parent")).unwrap();
        std::fs::write(root.join("tree/snippets.json"), "SENTINEL behind the link").unwrap();
        std::fs::write(root.join("parent/own.txt"), "SENTINEL of its own").unwrap();
        std::os::unix::fs::symlink(root.join("tree"), root.join("parent/lnk")).unwrap();
        std::os::unix::fs::symlink(root.join("tree/snippets.json"), root.join("parent/file-lnk")).unwrap();
        let re = regex::Regex::new("SENTINEL").unwrap();
        let (mut lines, mut files) = (Vec::new(), Vec::new());
        let parent = root.join("parent").to_string_lossy().to_string();
        super::search_dir_recursive(&re, &parent, "*", 0, 100, false, &mut lines, &mut files, 0);
        let _ = std::fs::remove_dir_all(&root);
        assert!(lines.iter().any(|l| l.contains("of its own")), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("behind the link")), "{lines:?}");
        assert_eq!(files.len(), 1, "{files:?}");
    }
}
