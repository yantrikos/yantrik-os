//! Disk tools — disk_usage, mount_info.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DiskUsageTool));
    reg.register(Box::new(MountInfoTool));
    reg.register(Box::new(DirSizeTool));
}

// ── Disk Usage ──

pub struct DiskUsageTool;

impl Tool for DiskUsageTool {
    fn name(&self) -> &'static str { "disk_usage" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "disk" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "disk_usage",
                "description": "Show disk space usage for all mounted partitions.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match std::process::Command::new("df")
            .args(["-h", "--output=target,size,used,avail,pcent,fstype"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                // Filter out tmpfs/devtmpfs for cleaner output
                let lines: Vec<&str> = text
                    .lines()
                    .filter(|l| {
                        let l_lower = l.to_lowercase();
                        !l_lower.contains("tmpfs") && !l_lower.contains("devtmpfs")
                            && !l_lower.contains("squashfs") || l_lower.starts_with("mounted")
                            || l_lower.starts_with("target")
                    })
                    .collect();
                if lines.is_empty() {
                    text.to_string()
                } else {
                    lines.join("\n")
                }
            }
            Ok(_) => {
                // Fallback to simpler df
                match std::process::Command::new("df").arg("-h").output() {
                    Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                    Err(e) => format!("Error: {e}"),
                }
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Mount Info ──

pub struct MountInfoTool;

impl Tool for MountInfoTool {
    fn name(&self) -> &'static str { "mount_info" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "disk" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "mount_info",
                "description": "Show mounted filesystems, their types, and mount options.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match std::process::Command::new("mount").output() {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                // Show only real filesystems (ext4, xfs, btrfs, vfat, etc)
                let real_types = ["ext4", "ext3", "btrfs", "xfs", "vfat", "ntfs", "f2fs", "zfs", "nfs"];
                let lines: Vec<&str> = text
                    .lines()
                    .filter(|l| real_types.iter().any(|t| l.contains(t)))
                    .collect();
                if lines.is_empty() {
                    // Show all (truncated)
                    let trunc = if text.len() > 2000 { &text[..2000] } else { &text };
                    trunc.to_string()
                } else {
                    let mut result = format!("Mounted filesystems ({}):\n", lines.len());
                    for l in &lines {
                        result.push_str(&format!("  {l}\n"));
                    }
                    result
                }
            }
            Ok(o) => format!("mount failed: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Directory Size ──

pub struct DirSizeTool;

impl Tool for DirSizeTool {
    fn name(&self) -> &'static str { "dir_size" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "disk" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "dir_size",
                "description": "Calculate the size of a directory and its largest subdirectories.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Directory path (e.g. ~/Downloads)"},
                        "depth": {"type": "integer", "description": "Depth of subdirectory listing (default: 1)"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1).min(3);

        if path.is_empty() {
            return "Error: path is required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Total size
        match std::process::Command::new("du")
            .args(["-h", "--max-depth", &depth.to_string(), &expanded])
            .output()
        {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                // Sort by size (du output is already structured)
                let mut lines: Vec<&str> = text.lines().collect();
                lines.truncate(30);
                if lines.is_empty() {
                    format!("Could not calculate size of {path}")
                } else {
                    let total_line = lines.last().copied().unwrap_or("unknown");
                    format!("Directory size ({path}):\n{}\nTotal: {}", lines.join("\n"), total_line)
                }
            }
            Ok(o) => format!("du failed: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error: {e}"),
        }
    }
}
