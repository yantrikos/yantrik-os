//! Disk tools — disk_usage, mount_info, dir_size, analyze_disk.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DiskUsageTool));
    reg.register(Box::new(MountInfoTool));
    reg.register(Box::new(DirSizeTool));
    reg.register(Box::new(AnalyzeDiskTool));
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
                "description": "Show disk space usage for all mounted partitions",
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
                "description": "Show mounted filesystems, their types, and mount options",
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
                    let trunc = if text.len() > 2000 { &text[..text.floor_char_boundary(2000)] } else { &text };
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
                "description": "Calculate the size of a directory and its largest",
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

// ── Analyze Disk ──

pub struct AnalyzeDiskTool;

impl Tool for AnalyzeDiskTool {
    fn name(&self) -> &'static str { "analyze_disk" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "disk" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "analyze_disk",
                "description": "Analyze a directory for disk space usage: subdirectory",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Directory to analyze (default: ~/)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("~/");

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };
        analyze(&expanded, path)
    }
}

/// The report for the folder `expanded`, named `path` in the text.
fn analyze(expanded: &str, path: &str) -> String {
    // Each program is started on its own, with the path as one argument: no shell line, so a `$(…)`
    // or a backtick in a folder name is only a name. The sorting and summing is done here.
    let mut report = Vec::new();

    // 1. Subdirectory sizes (top 10, by size). `du -sb -- <each visible entry>`.
    let entries: Vec<std::path::PathBuf> = std::fs::read_dir(expanded)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect())
        .unwrap_or_default();
    if !entries.is_empty() {
        if let Ok(o) = std::process::Command::new("du").arg("-sb").arg("--").args(&entries).stderr(std::process::Stdio::null()).output() {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut sizes: Vec<(u64, &str)> = text
                .lines()
                .filter_map(|l| l.split_once('\t'))
                .filter_map(|(n, p)| Some((n.trim().parse().ok()?, p)))
                .collect();
            sizes.sort_by(|a, b| b.0.cmp(&a.0));
            if !sizes.is_empty() {
                report.push(format!("Largest items in {}:", path));
                for (bytes, p) in sizes.into_iter().take(10) {
                    report.push(format!("  {}\t{}", super::format_size(bytes), p));
                }
            }
        }
    }

    // 2. Old files (>30 days, summarized)
    if let Ok(o) = std::process::Command::new("find")
        .arg(expanded)
        .args(["-maxdepth", "2", "-type", "f", "-mtime", "+30", "-printf", "%s\\n"])
        .stderr(std::process::Stdio::null())
        .output()
    {
        let text = String::from_utf8_lossy(&o.stdout);
        let sizes: Vec<u64> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
        if !sizes.is_empty() {
            report.push(format!("\nOlder than 30 days: {} files (~{}).", sizes.len(), super::format_size(sizes.iter().sum())));
        }
    }

    // 3. File type breakdown (top 8 extensions by count)
    if let Ok(o) = std::process::Command::new("find")
        .arg(expanded)
        .args(["-maxdepth", "2", "-type", "f", "-name", "*.*"])
        .stderr(std::process::Stdio::null())
        .output()
    {
        let text = String::from_utf8_lossy(&o.stdout);
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for line in text.lines() {
            if let Some((_, ext)) = line.rsplit_once('.') {
                *counts.entry(ext).or_default() += 1;
            }
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        if !counts.is_empty() {
            report.push("\nFile types:".to_string());
            for (ext, n) in counts.into_iter().take(8) {
                report.push(format!("  {:>6} {}", n, ext));
            }
        }
    }

    if report.is_empty() {
        format!("Could not analyze {}. Directory may be empty or inaccessible.", path)
    } else {
        report.join("\n")
    }
}

#[cfg(test)]
mod quote_tests {
    use super::*;

    #[test]
    fn a_command_substitution_in_a_folder_name_stays_a_name() {
        let dir = std::env::temp_dir().join(format!("disk-inj-{}-$(touch INJECTED)`touch INJECTED`'q", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), vec![0u8; 5000]).unwrap();
        let out = analyze(&dir.to_string_lossy(), "the folder");
        assert!(out.contains("sub"), "{out}");
        assert!(out.contains("txt"), "{out}");
        assert!(!dir.join("INJECTED").exists() && !std::path::Path::new("INJECTED").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
