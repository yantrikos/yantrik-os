//! Package manager tools — package_search, package_install, package_remove, package_info.
//! Targets Alpine Linux (apk) with fallback to apt/pacman.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(PackageSearchTool));
    reg.register(Box::new(PackageInstallTool));
    reg.register(Box::new(PackageRemoveTool));
    reg.register(Box::new(PackageInfoTool));
    reg.register(Box::new(PackageListTool));
}

/// Detect which package manager is available.
fn pkg_backend() -> &'static str {
    if std::path::Path::new("/sbin/apk").exists() || std::path::Path::new("/usr/sbin/apk").exists() {
        "apk"
    } else if std::path::Path::new("/usr/bin/apt").exists() {
        "apt"
    } else if std::path::Path::new("/usr/bin/pacman").exists() {
        "pacman"
    } else {
        "unknown"
    }
}

// ── Package Search ──

pub struct PackageSearchTool;

impl Tool for PackageSearchTool {
    fn name(&self) -> &'static str { "package_search" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "package" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "package_search",
                "description": "Search available packages by name or keyword",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Package name or keyword to search"}
                    },
                    "required": ["query"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        if query.is_empty() {
            return "Error: query is required".to_string();
        }

        // Validate no shell metacharacters
        if query.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.') {
            return "Error: invalid characters in query".to_string();
        }

        let (cmd, cmd_args): (&str, Vec<&str>) = match pkg_backend() {
            "apk" => ("apk", vec!["search", "-v", query]),
            "apt" => ("apt-cache", vec!["search", query]),
            "pacman" => ("pacman", vec!["-Ss", query]),
            _ => return "Error: no supported package manager found".to_string(),
        };

        match std::process::Command::new(cmd).args(&cmd_args).output() {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if text.trim().is_empty() {
                    format!("No packages found matching '{query}'")
                } else {
                    let lines: Vec<&str> = text.lines().take(30).collect();
                    let total = text.lines().count();
                    let mut result = format!("Packages matching '{query}':\n");
                    result.push_str(&lines.join("\n"));
                    if total > 30 {
                        result.push_str(&format!("\n... and {} more", total - 30));
                    }
                    result
                }
            }
            Ok(o) => format!("Search failed: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Package Install ──

pub struct PackageInstallTool;

impl Tool for PackageInstallTool {
    fn name(&self) -> &'static str { "package_install" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Dangerous }
    fn category(&self) -> &'static str { "package" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "package_install",
                "description": "Install a package from the system repository",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "package": {"type": "string", "description": "Package name to install"}
                    },
                    "required": ["package"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let package = args.get("package").and_then(|v| v.as_str()).unwrap_or_default();
        if package.is_empty() {
            return "Error: package name is required".to_string();
        }

        // Strict validation — package names are alphanumeric + hyphens
        if package.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.' && c != '+') {
            return "Error: invalid package name".to_string();
        }

        let (cmd, cmd_args): (&str, Vec<&str>) = match pkg_backend() {
            "apk" => ("apk", vec!["add", "--no-cache", package]),
            "apt" => ("apt-get", vec!["install", "-y", package]),
            "pacman" => ("pacman", vec!["-S", "--noconfirm", package]),
            _ => return "Error: no supported package manager found".to_string(),
        };

        match std::process::Command::new(cmd).args(&cmd_args).output() {
            Ok(o) if o.status.success() => {
                format!("Installed '{package}' successfully")
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                let out = String::from_utf8_lossy(&o.stdout);
                format!("Install failed: {} {}", out.trim(), err.trim())
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Package Remove ──

pub struct PackageRemoveTool;

impl Tool for PackageRemoveTool {
    fn name(&self) -> &'static str { "package_remove" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Dangerous }
    fn category(&self) -> &'static str { "package" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "package_remove",
                "description": "Remove an installed package",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "package": {"type": "string", "description": "Package name to remove"}
                    },
                    "required": ["package"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let package = args.get("package").and_then(|v| v.as_str()).unwrap_or_default();
        if package.is_empty() {
            return "Error: package name is required".to_string();
        }

        if package.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.' && c != '+') {
            return "Error: invalid package name".to_string();
        }

        // Block removal of critical packages
        const PROTECTED: &[&str] = &[
            "linux", "kernel", "glibc", "musl", "busybox", "openrc", "alpine-base",
            "labwc", "pipewire", "dbus", "seatd", "eudev",
        ];
        if PROTECTED.iter().any(|p| package.contains(p)) {
            return format!("Error: '{package}' is a critical system package and cannot be removed");
        }

        let (cmd, cmd_args): (&str, Vec<&str>) = match pkg_backend() {
            "apk" => ("apk", vec!["del", package]),
            "apt" => ("apt-get", vec!["remove", "-y", package]),
            "pacman" => ("pacman", vec!["-R", "--noconfirm", package]),
            _ => return "Error: no supported package manager found".to_string(),
        };

        match std::process::Command::new(cmd).args(&cmd_args).output() {
            Ok(o) if o.status.success() => {
                format!("Removed '{package}'")
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("Remove failed: {}", err.trim())
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Package Info ──

pub struct PackageInfoTool;

impl Tool for PackageInfoTool {
    fn name(&self) -> &'static str { "package_info" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "package" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "package_info",
                "description": "Get detailed info about a package (version, description",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "package": {"type": "string", "description": "Package name"}
                    },
                    "required": ["package"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let package = args.get("package").and_then(|v| v.as_str()).unwrap_or_default();
        if package.is_empty() {
            return "Error: package name is required".to_string();
        }

        if package.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.') {
            return "Error: invalid package name".to_string();
        }

        let (cmd, cmd_args): (&str, Vec<&str>) = match pkg_backend() {
            "apk" => ("apk", vec!["info", "-a", package]),
            "apt" => ("apt-cache", vec!["show", package]),
            "pacman" => ("pacman", vec!["-Si", package]),
            _ => return "Error: no supported package manager found".to_string(),
        };

        match std::process::Command::new(cmd).args(&cmd_args).output() {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if text.trim().is_empty() {
                    format!("Package '{package}' not found")
                } else {
                    let trunc = if text.len() > 2000 { &text[..text.floor_char_boundary(2000)] } else { &text };
                    trunc.to_string()
                }
            }
            Ok(o) => format!("Info failed: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── Package List (installed) ──

pub struct PackageListTool;

impl Tool for PackageListTool {
    fn name(&self) -> &'static str { "package_list" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "package" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "package_list",
                "description": "List installed packages, optionally filtered by name",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "filter": {"type": "string", "description": "Filter by name substring"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");

        let (cmd, cmd_args): (&str, Vec<&str>) = match pkg_backend() {
            "apk" => ("apk", vec!["list", "--installed"]),
            "apt" => ("dpkg", vec!["--list"]),
            "pacman" => ("pacman", vec!["-Q"]),
            _ => return "Error: no supported package manager found".to_string(),
        };

        match std::process::Command::new(cmd).args(&cmd_args).output() {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                let lines: Vec<&str> = if filter.is_empty() {
                    text.lines().take(50).collect()
                } else {
                    let f = filter.to_lowercase();
                    text.lines()
                        .filter(|l| l.to_lowercase().contains(&f))
                        .take(50)
                        .collect()
                };
                let total = if filter.is_empty() {
                    text.lines().count()
                } else {
                    lines.len()
                };

                if lines.is_empty() {
                    if filter.is_empty() {
                        "No packages installed.".to_string()
                    } else {
                        format!("No installed packages matching '{filter}'")
                    }
                } else {
                    let mut result = format!("Installed packages ({} shown):\n", lines.len());
                    result.push_str(&lines.join("\n"));
                    if total > 50 {
                        result.push_str(&format!("\n... {} total installed", total));
                    }
                    result
                }
            }
            Ok(o) => format!("Failed: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error: {e}"),
        }
    }
}
