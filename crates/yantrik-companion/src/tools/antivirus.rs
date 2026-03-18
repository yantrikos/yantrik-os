//! Antivirus tools — scan, status, update, quarantine.
//! Uses ClamAV (`clamscan`/`clamdscan` + `freshclam`).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(AntivirusScanTool));
    reg.register(Box::new(AntivirusStatusTool));
    reg.register(Box::new(AntivirusUpdateTool));
    reg.register(Box::new(AntivirusQuarantineTool));
}

/// Check if ClamAV daemon is running (faster scans).
fn has_clamd() -> bool {
    std::process::Command::new("clamdscan")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Scan ──

pub struct AntivirusScanTool;

impl Tool for AntivirusScanTool {
    fn name(&self) -> &'static str { "antivirus_scan" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "antivirus" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "antivirus_scan",
                "description": "Scan a file or directory for malware using ClamAV",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File or directory to scan (e.g. ~/Downloads)"},
                        "recursive": {"type": "boolean", "description": "Scan subdirectories (default: true)"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(true);

        if path.is_empty() {
            return "Error: path is required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        if !std::path::Path::new(&expanded).exists() {
            return format!("Error: '{}' does not exist", path);
        }

        // Ensure quarantine dir exists
        let quarantine_dir = "/tmp/quarantine";
        let _ = std::fs::create_dir_all(quarantine_dir);

        // Prefer clamdscan (daemon, faster) over clamscan
        let scanner = if has_clamd() { "clamdscan" } else { "clamscan" };

        let mut cmd = std::process::Command::new(scanner);
        cmd.arg("--infected")
           .arg("--no-summary");  // We'll build our own summary

        if scanner == "clamscan" {
            // Move infected files to quarantine
            cmd.arg("--move").arg(quarantine_dir);
        }

        if recursive {
            cmd.arg("-r");
        }

        cmd.arg(&expanded);

        match cmd.output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Exit codes: 0 = clean, 1 = found virus, 2 = error
                match output.status.code() {
                    Some(0) => {
                        format!("Scan complete: {} is clean. No threats found.", path)
                    }
                    Some(1) => {
                        let threats: Vec<&str> = stdout
                            .lines()
                            .filter(|l| l.contains("FOUND"))
                            .collect();
                        let count = threats.len();
                        let details = if threats.len() > 20 {
                            format!("{}\n... and {} more", threats[..20].join("\n"), threats.len() - 20)
                        } else {
                            threats.join("\n")
                        };
                        format!(
                            "⚠ THREATS FOUND: {count} infected file(s) in {path}\n{details}\nInfected files moved to {quarantine_dir}/"
                        )
                    }
                    Some(2) => {
                        format!("Scan error: {}", stderr.trim())
                    }
                    _ => {
                        if stderr.contains("command not found") || stderr.contains("No such file") {
                            "ClamAV is not installed. Install with: apk add clamav".to_string()
                        } else {
                            format!("Scan returned unexpected status.\nstdout: {}\nstderr: {}", stdout.trim(), stderr.trim())
                        }
                    }
                }
            }
            Err(e) => {
                format!("ClamAV not available (install with `apk add clamav`): {e}")
            }
        }
    }
}

// ── Status ──

pub struct AntivirusStatusTool;

impl Tool for AntivirusStatusTool {
    fn name(&self) -> &'static str { "antivirus_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "antivirus" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "antivirus_status",
                "description": "Check ClamAV installation status, virus database version",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = Vec::new();

        // Check clamscan version
        match std::process::Command::new("clamscan").arg("--version").output() {
            Ok(o) if o.status.success() => {
                let version = String::from_utf8_lossy(&o.stdout);
                info.push(format!("ClamAV: {}", version.trim()));
            }
            _ => {
                return "ClamAV is not installed. Install with: apk add clamav clamav-libunrar".to_string();
            }
        }

        // Check daemon status
        if has_clamd() {
            info.push("Daemon: running (clamdscan available — faster scans)".to_string());
        } else {
            info.push("Daemon: not running (using clamscan — slower but works)".to_string());
        }

        // Check database freshness
        let db_paths = [
            "/var/lib/clamav/daily.cvd",
            "/var/lib/clamav/daily.cld",
            "/var/lib/clamav/main.cvd",
            "/var/lib/clamav/main.cld",
        ];
        let mut found_db = false;
        for db in &db_paths {
            if let Ok(meta) = std::fs::metadata(db) {
                found_db = true;
                if let Ok(modified) = meta.modified() {
                    let age = std::time::SystemTime::now()
                        .duration_since(modified)
                        .unwrap_or_default();
                    let hours = age.as_secs() / 3600;
                    let days = hours / 24;
                    if days > 0 {
                        info.push(format!("Database: {} ({} days old)", db, days));
                    } else {
                        info.push(format!("Database: {} ({} hours old)", db, hours));
                    }
                    if days > 7 {
                        info.push("⚠ Database is outdated! Run antivirus_update to refresh.".to_string());
                    }
                }
                break;
            }
        }
        if !found_db {
            info.push("Database: NOT FOUND — run antivirus_update first!".to_string());
        }

        // Check quarantine
        if let Ok(entries) = std::fs::read_dir("/tmp/quarantine") {
            let count = entries.count();
            if count > 0 {
                info.push(format!("Quarantine: {count} file(s) in /tmp/quarantine/"));
            }
        }

        info.join("\n")
    }
}

// ── Update ──

pub struct AntivirusUpdateTool;

impl Tool for AntivirusUpdateTool {
    fn name(&self) -> &'static str { "antivirus_update" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "antivirus" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "antivirus_update",
                "description": "Update ClamAV virus definitions using freshclam",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match std::process::Command::new("freshclam").output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.success() {
                    // Extract useful lines
                    let updates: Vec<&str> = stdout
                        .lines()
                        .filter(|l| l.contains("updated") || l.contains("already") || l.contains("Database"))
                        .collect();
                    if updates.is_empty() {
                        format!("Update complete.\n{}", stdout.trim())
                    } else {
                        format!("Update complete:\n{}", updates.join("\n"))
                    }
                } else {
                    format!("Update failed: {}\n{}", stdout.trim(), stderr.trim())
                }
            }
            Err(e) => {
                format!("freshclam not available (install clamav): {e}")
            }
        }
    }
}

// ── Quarantine List ──

pub struct AntivirusQuarantineTool;

impl Tool for AntivirusQuarantineTool {
    fn name(&self) -> &'static str { "antivirus_quarantine" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "antivirus" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "antivirus_quarantine",
                "description": "List files currently in the quarantine directory",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let quarantine_dir = "/tmp/quarantine";

        match std::fs::read_dir(quarantine_dir) {
            Ok(entries) => {
                let mut files = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let size = entry.metadata()
                        .map(|m| super::format_size(m.len()))
                        .unwrap_or_else(|_| "?".to_string());
                    files.push(format!("  {} ({})", name, size));
                }
                if files.is_empty() {
                    "Quarantine is empty — no infected files.".to_string()
                } else {
                    format!("Quarantined files ({}):\n{}", files.len(), files.join("\n"))
                }
            }
            Err(_) => "No quarantine directory exists (no files have been quarantined).".to_string(),
        }
    }
}
