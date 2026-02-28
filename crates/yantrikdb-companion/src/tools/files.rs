//! File tools — write_file, manage_files, search_files, file_info.

use std::io::Write;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path, glob_match, format_size};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(WriteFileTool));
    reg.register(Box::new(ManageFilesTool));
    reg.register(Box::new(SearchFilesTool));
    reg.register(Box::new(FileInfoTool));
}

// ── Write File ──

pub struct WriteFileTool;

impl Tool for WriteFileTool {
    fn name(&self) -> &'static str { "write_file" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "files" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write text to a file. Creates or overwrites.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path (e.g. ~/Documents/note.txt)"},
                        "content": {"type": "string", "description": "Text to write"},
                        "append": {"type": "boolean", "description": "Append instead of overwrite"}
                    },
                    "required": ["path", "content"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or_default();
        let append = args.get("append").and_then(|v| v.as_bool()).unwrap_or(false);

        if path.is_empty() || content.is_empty() {
            return "Error: path and content are required".to_string();
        }

        // Max 100KB
        if content.len() > 102_400 {
            return "Error: content exceeds 100KB limit".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Create parent dirs if needed
        if let Some(parent) = std::path::Path::new(&expanded).parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return format!("Error creating directory: {e}");
                }
            }
        }

        if append {
            let mut file = match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&expanded)
            {
                Ok(f) => f,
                Err(e) => return format!("Error opening file: {e}"),
            };
            match file.write_all(content.as_bytes()) {
                Ok(_) => format!("Appended {} bytes to {path}", content.len()),
                Err(e) => format!("Error writing: {e}"),
            }
        } else {
            match std::fs::write(&expanded, content) {
                Ok(_) => format!("Wrote {} bytes to {path}", content.len()),
                Err(e) => format!("Error writing file: {e}"),
            }
        }
    }
}

// ── Manage Files ──

pub struct ManageFilesTool;

impl Tool for ManageFilesTool {
    fn name(&self) -> &'static str { "manage_files" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Dangerous }
    fn category(&self) -> &'static str { "files" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "manage_files",
                "description": "Move, copy, or delete a file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["move", "copy", "delete"]},
                        "path": {"type": "string", "description": "Source file path"},
                        "destination": {"type": "string", "description": "Target path (for move/copy)"}
                    },
                    "required": ["action", "path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or_default();
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let dest = args.get("destination").and_then(|v| v.as_str()).unwrap_or_default();

        if action.is_empty() || path.is_empty() {
            return "Error: action and path are required".to_string();
        }

        let src = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        match action {
            "delete" => {
                let p = std::path::Path::new(&src);
                if p.is_dir() {
                    match std::fs::remove_dir_all(&src) {
                        Ok(_) => format!("Deleted directory: {path}"),
                        Err(e) => format!("Error deleting directory: {e}"),
                    }
                } else {
                    match std::fs::remove_file(&src) {
                        Ok(_) => format!("Deleted: {path}"),
                        Err(e) => format!("Error deleting: {e}"),
                    }
                }
            }
            "move" | "copy" => {
                if dest.is_empty() {
                    return "Error: destination is required for move/copy".to_string();
                }
                let dst = match validate_path(dest) {
                    Ok(p) => p,
                    Err(e) => return format!("Error (destination): {e}"),
                };

                // Create parent dirs for destination
                if let Some(parent) = std::path::Path::new(&dst).parent() {
                    if !parent.exists() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                }

                if action == "move" {
                    match std::fs::rename(&src, &dst) {
                        Ok(_) => format!("Moved {path} → {dest}"),
                        Err(_) => {
                            // rename fails across filesystems — fall back to copy+delete
                            match std::fs::copy(&src, &dst) {
                                Ok(_) => {
                                    let _ = std::fs::remove_file(&src);
                                    format!("Moved {path} → {dest}")
                                }
                                Err(e) => format!("Error moving: {e}"),
                            }
                        }
                    }
                } else {
                    match std::fs::copy(&src, &dst) {
                        Ok(bytes) => format!("Copied {path} → {dest} ({})", format_size(bytes)),
                        Err(e) => format!("Error copying: {e}"),
                    }
                }
            }
            _ => format!("Error: unknown action '{action}'. Use move, copy, or delete."),
        }
    }
}

// ── Search Files ──

pub struct SearchFilesTool;

impl Tool for SearchFilesTool {
    fn name(&self) -> &'static str { "search_files" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "files" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "search_files",
                "description": "Search for text in files within a directory.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Directory to search"},
                        "query": {"type": "string", "description": "Text to find (case-insensitive)"},
                        "pattern": {"type": "string", "description": "Glob filter (e.g. *.txt)"}
                    },
                    "required": ["path", "query"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");

        if path.is_empty() || query.is_empty() {
            return "Error: path and query are required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();

        search_dir(&expanded, &query_lower, pattern, 0, 3, &mut matches);

        if matches.is_empty() {
            format!("No matches for '{query}' in {path}")
        } else {
            let count = matches.len();
            matches.truncate(20);
            let mut result = format!("Found {} match(es) for '{query}':\n", count);
            for (file, line_num, line) in &matches {
                result.push_str(&format!("  {}:{}: {}\n", file, line_num, line));
            }
            if count > 20 {
                result.push_str(&format!("  ... and {} more\n", count - 20));
            }
            result
        }
    }
}

/// Recursively search files for text. Max depth and max 1MB file size.
fn search_dir(
    dir: &str,
    query: &str,
    pattern: &str,
    depth: usize,
    max_depth: usize,
    matches: &mut Vec<(String, usize, String)>,
) {
    if depth > max_depth || matches.len() >= 100 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            // Skip hidden dirs
            if !name.starts_with('.') {
                search_dir(&path.to_string_lossy(), query, pattern, depth + 1, max_depth, matches);
            }
        } else if pattern == "*" || glob_match(pattern, &name) {
            // Skip large files (>1MB) and binary-looking files
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            if size > 1_048_576 || size == 0 {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(&path) {
                for (i, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(query) {
                        let display_path = path.to_string_lossy().to_string();
                        let trimmed = if line.len() > 120 { &line[..120] } else { line };
                        matches.push((display_path, i + 1, trimmed.to_string()));
                        if matches.len() >= 100 {
                            return;
                        }
                    }
                }
            }
        }
    }
}

// ── File Info ──

pub struct FileInfoTool;

impl Tool for FileInfoTool {
    fn name(&self) -> &'static str { "file_info" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "files" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "file_info",
                "description": "Get file metadata: size, modified time, type, permissions.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path"}
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

        let meta = match std::fs::metadata(&expanded) {
            Ok(m) => m,
            Err(e) => return format!("Error: {e}"),
        };

        let file_type = if meta.is_dir() {
            "directory"
        } else if meta.is_symlink() {
            "symlink"
        } else {
            "file"
        };

        let modified = meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                let dt = chrono::DateTime::from_timestamp(d.as_secs() as i64, 0);
                dt.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default()
            })
            .unwrap_or_else(|| "unknown".to_string());

        // Guess MIME type from extension
        let ext = std::path::Path::new(&expanded)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let mime = match ext {
            "txt" => "text/plain",
            "md" => "text/markdown",
            "rs" => "text/x-rust",
            "py" => "text/x-python",
            "js" => "text/javascript",
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "json" => "application/json",
            "yaml" | "yml" => "text/yaml",
            "toml" => "text/toml",
            "xml" => "text/xml",
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            "zip" => "application/zip",
            "tar" => "application/x-tar",
            "gz" => "application/gzip",
            _ => "application/octet-stream",
        };

        #[cfg(unix)]
        let perms = {
            use std::os::unix::fs::PermissionsExt;
            format!("{:o}", meta.permissions().mode() & 0o7777)
        };
        #[cfg(not(unix))]
        let perms = if meta.permissions().readonly() { "readonly" } else { "read-write" }.to_string();

        format!(
            "Path: {path}\nType: {file_type}\nSize: {}\nModified: {modified}\nMIME: {mime}\nPermissions: {perms}",
            format_size(meta.len())
        )
    }
}
