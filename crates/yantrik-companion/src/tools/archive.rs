//! Archive tools — archive_create, archive_extract.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ArchiveCreateTool));
    reg.register(Box::new(ArchiveExtractTool));
}

// ── Archive Create ──

pub struct ArchiveCreateTool;

impl Tool for ArchiveCreateTool {
    fn name(&self) -> &'static str { "archive_create" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "archive" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "archive_create",
                "description": "Create a tar",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "output_path": {"type": "string", "description": "Archive output path (e.g. ~/backup.tar.gz)"},
                        "source_paths": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Files/directories to include"
                        }
                    },
                    "required": ["output_path", "source_paths"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let output_path = args.get("output_path").and_then(|v| v.as_str()).unwrap_or_default();
        let source_paths = args
            .get("source_paths")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if output_path.is_empty() || source_paths.is_empty() {
            return "Error: output_path and source_paths are required".to_string();
        }

        if source_paths.len() > 20 {
            return "Error: too many source paths (max 20)".to_string();
        }

        let out_expanded = match validate_path(output_path) {
            Ok(p) => p,
            Err(e) => return format!("Error (output): {e}"),
        };

        // Validate all source paths
        let mut validated_sources = Vec::new();
        for src in &source_paths {
            match validate_path(src) {
                Ok(p) => validated_sources.push(p),
                Err(e) => return format!("Error (source '{src}'): {e}"),
            }
        }

        // Create parent dirs for output
        if let Some(parent) = std::path::Path::new(&out_expanded).parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        let mut cmd = std::process::Command::new("tar");
        cmd.arg("czf").arg(&out_expanded);
        for src in &validated_sources {
            cmd.arg(src);
        }

        match cmd.output() {
            Ok(o) if o.status.success() => {
                let size = std::fs::metadata(&out_expanded)
                    .map(|m| super::format_size(m.len()))
                    .unwrap_or_else(|_| "unknown size".to_string());
                format!(
                    "Archive created: {output_path} ({size}, {} source(s))",
                    source_paths.len()
                )
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("tar failed: {err}")
            }
            Err(e) => format!("Error (tar not available?): {e}"),
        }
    }
}

// ── Archive Extract ──

pub struct ArchiveExtractTool;

impl Tool for ArchiveExtractTool {
    fn name(&self) -> &'static str { "archive_extract" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "archive" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "archive_extract",
                "description": "Extract a tar",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "archive_path": {"type": "string", "description": "Path to .tar.gz file"},
                        "extract_to": {"type": "string", "description": "Directory to extract into"}
                    },
                    "required": ["archive_path", "extract_to"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let archive_path = args.get("archive_path").and_then(|v| v.as_str()).unwrap_or_default();
        let extract_to = args.get("extract_to").and_then(|v| v.as_str()).unwrap_or_default();

        if archive_path.is_empty() || extract_to.is_empty() {
            return "Error: archive_path and extract_to are required".to_string();
        }

        let archive_expanded = match validate_path(archive_path) {
            Ok(p) => p,
            Err(e) => return format!("Error (archive): {e}"),
        };

        let dest_expanded = match validate_path(extract_to) {
            Ok(p) => p,
            Err(e) => return format!("Error (destination): {e}"),
        };

        // Create destination directory
        if !std::path::Path::new(&dest_expanded).exists() {
            if let Err(e) = std::fs::create_dir_all(&dest_expanded) {
                return format!("Error creating directory: {e}");
            }
        }

        match std::process::Command::new("tar")
            .args(["xzf", &archive_expanded, "-C", &dest_expanded])
            .output()
        {
            Ok(o) if o.status.success() => {
                format!("Extracted {archive_path} to {extract_to}")
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("Extraction failed: {err}")
            }
            Err(e) => format!("Error (tar not available?): {e}"),
        }
    }
}
