//! Antivirus tools — scan, status, update, quarantine.
//! Uses ClamAV (`clamscan`/`clamdscan` + `freshclam`).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(AntivirusScanTool));
    reg.register(Box::new(AntivirusStatusTool));
    reg.register(Box::new(AntivirusUpdateTool));
    reg.register(Box::new(AntivirusQuarantineTool));
}

/// Where infected files are moved to: `$XDG_STATE_HOME/yantrik/quarantine` (or
/// `~/.local/state/yantrik/quarantine`), 0700 and checked to be ours.
///
/// It was `/tmp/quarantine` — the worst place for it. Any account could read the malware we had
/// just taken out of someone's Downloads, or create the directory first and have us move files
/// into a place they own; and a reboot emptied it, so there was nothing left to inspect or
/// restore. State, not scratch, so it lasts. Scan, status and listing all ask here.
fn quarantine_dir() -> Result<std::path::PathBuf, String> {
    yantrik_ml::private_dir::state_dir("quarantine").map_err(|e| format!("no private quarantine directory: {e}"))
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
        let quarantine_dir = match quarantine_dir() {
            Ok(dir) => dir,
            Err(e) => return format!("Error: {e}"),
        };

        // Prefer clamdscan (daemon, faster) over clamscan
        let scanner = if has_clamd() { "clamdscan" } else { "clamscan" };

        let mut cmd = std::process::Command::new(scanner);  // fixed-program: clamdscan|clamscan
        cmd.arg("--infected")
           .arg("--no-summary");  // We'll build our own summary

        if scanner == "clamscan" {
            // Move infected files to quarantine
            cmd.arg("--move").arg(&quarantine_dir);
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
                        threats_report(path, &stdout, &quarantine_dir)
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

/// What `antivirus_scan` says of a scanner's `stdout` that found something (one `path: signature FOUND`
/// line each). A file in a protected place, or with a secret's name, is counted and never named (a
/// name says where a secret is kept; #1247).
fn threats_report(path: &str, stdout: &str, quarantine_dir: &std::path::Path) -> String {
    let (kept, hidden): (Vec<&str>, Vec<&str>) = stdout.lines().filter(|l| l.contains("FOUND")).partition(|line| {
        let found = line.rsplit_once(": ").map_or(*line, |(file, _)| file);
        !yantrik_ipc_contracts::home_paths::is_protected(std::path::Path::new(found))
    });
    let count = kept.len() + hidden.len();
    let mut details = if kept.len() > 20 {
        format!("{}\n... and {} more", kept[..20].join("\n"), kept.len() - 20)
    } else {
        kept.join("\n")
    };
    if !hidden.is_empty() {
        details.push('\n');
        details.push_str(&crate::safe_command_listing::hidden_note(hidden.len()));
    }
    format!("⚠ THREATS FOUND: {count} infected file(s) in {path}\n{details}\nInfected files moved to {}/", quarantine_dir.display())
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
        if let Ok(dir) = quarantine_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let count = entries.count();
                if count > 0 {
                    info.push(format!("Quarantine: {count} file(s) in {}/", dir.display()));
                }
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
        let quarantine_dir = match quarantine_dir() {
            Ok(dir) => dir,
            Err(e) => return format!("Error: {e}"),
        };
        quarantine_report(&quarantine_dir)
    }
}

/// What `antivirus_quarantine` says of the files in `quarantine_dir`: a name that is a secret's is
/// counted, never listed (#1247).
fn quarantine_report(quarantine_dir: &std::path::Path) -> String {
    match std::fs::read_dir(&quarantine_dir) {
        Ok(entries) => {
            let mut files = Vec::new();
            let mut hidden = 0;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // A quarantined file keeps its name, and a secret's name is not shown (#1247).
                if yantrik_ipc_contracts::home_paths::is_secret_name(&name) {
                    hidden += 1;
                    continue;
                }
                let size = entry.metadata()
                    .map(|m| super::format_size(m.len()))
                    .unwrap_or_else(|_| "?".to_string());
                files.push(format!("  {} ({})", name, size));
            }
            if files.is_empty() && hidden == 0 {
                "Quarantine is empty — no infected files.".to_string()
            } else {
                let note = if hidden > 0 { format!("\n{}", crate::safe_command_listing::hidden_note(hidden)) } else { String::new() };
                format!("Quarantined files ({}):\n{}{note}", files.len(), files.join("\n"))
            }
        }
        Err(_) => "No quarantine directory exists (no files have been quarantined).".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_threat_in_a_protected_place_is_counted_and_never_named() {
        let stdout = "/home/ann/backup/tokens.json: Eicar-Signature FOUND\n/home/ann/Downloads/a.exe: Win.Test FOUND\n/home/ann/.ssh/x: Eicar FOUND\n/home/ann/backup/server.pem: Eicar FOUND\n";
        let out = threats_report("~", stdout, std::path::Path::new("/q"));
        assert!(out.contains("4 infected file(s)") && out.contains("a.exe") && out.contains("3 entries are not shown"), "{out}");
        for part in ["tokens", ".ssh", "server.pem"] {
            assert!(!out.contains(part), "{part}: {out}");
        }
    }

    #[test]
    fn the_quarantine_lists_no_secrets_name() {
        let Ok(scratch) = yantrik_ml::private_dir::scratch_dir() else { return };
        let dir = scratch.join(format!("yantrik-1247-quarantine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder in the scratch directory");
        for name in ["tokens.json", "id_rsa", "tokens (copy 1).json", "virus.exe"] {
            std::fs::write(dir.join(name), "FAKE-SECRET-DO-NOT-PRINT").expect("a file in the scratch directory");
        }
        let out = quarantine_report(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.contains("virus.exe") && out.contains("(1)") && out.contains("3 entries are not shown"), "{out}");
        for part in ["tokens", "id_rsa", "copy"] {
            assert!(!out.contains(part), "{part}: {out}");
        }
    }
}
