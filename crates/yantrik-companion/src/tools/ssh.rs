//! SSH tools — host discovery and remote command execution.
//!
//! NOTE: validate_path() blocks `.ssh` directory. SSH tools bypass it
//! and hardcode `~/.ssh/config` via `$HOME` directly.
//!
//! Tools: ssh_list_hosts, ssh_check_host, ssh_run.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(SshListHostsTool));
    reg.register(Box::new(SshCheckHostTool));
    reg.register(Box::new(SshRunTool));
}

/// Validate an SSH host string — only alphanumeric, dots, dashes, colons.
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    if host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':') {
        Ok(())
    } else {
        Err(format!("Invalid host '{}': only alphanumeric, '.', '-', ':' are allowed", host))
    }
}

/// Get the SSH config path directly from $HOME (bypassing validate_path).
fn ssh_config_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{}/.ssh/config", home)
}

/// Get the known_hosts path directly from $HOME.
fn known_hosts_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{}/.ssh/known_hosts", home)
}

// ── SSH List Hosts ──

pub struct SshListHostsTool;

impl Tool for SshListHostsTool {
    fn name(&self) -> &'static str { "ssh_list_hosts" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "ssh" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "ssh_list_hosts",
                "description": "List SSH hosts from your SSH config and known_hosts.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut hosts = Vec::new();

        // Parse ~/.ssh/config
        if let Ok(content) = std::fs::read_to_string(ssh_config_path()) {
            let mut current_host: Option<String> = None;
            let mut hostname = String::new();
            let mut user = String::new();
            let mut port = String::new();

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }

                let lower = trimmed.to_lowercase();
                if lower.starts_with("host ") {
                    // Flush previous host
                    if let Some(ref h) = current_host {
                        hosts.push(format!(
                            "  {:<25} hostname: {:<20} user: {:<15} port: {}",
                            h,
                            if hostname.is_empty() { "-" } else { &hostname },
                            if user.is_empty() { "-" } else { &user },
                            if port.is_empty() { "22" } else { &port },
                        ));
                    }

                    let host_value = trimmed[5..].trim().to_string();
                    // Skip wildcard Host *
                    if host_value == "*" {
                        current_host = None;
                    } else {
                        current_host = Some(host_value);
                    }
                    hostname.clear();
                    user.clear();
                    port.clear();
                } else if current_host.is_some() {
                    if lower.starts_with("hostname ") {
                        hostname = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
                    } else if lower.starts_with("user ") {
                        user = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
                    } else if lower.starts_with("port ") {
                        port = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
                    }
                }
            }

            // Flush last host
            if let Some(ref h) = current_host {
                hosts.push(format!(
                    "  {:<25} hostname: {:<20} user: {:<15} port: {}",
                    h,
                    if hostname.is_empty() { "-" } else { &hostname },
                    if user.is_empty() { "-" } else { &user },
                    if port.is_empty() { "22" } else { &port },
                ));
            }
        }

        // Also check known_hosts for additional hostnames
        let mut known = Vec::new();
        if let Ok(content) = std::fs::read_to_string(known_hosts_path()) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                // known_hosts format: hostname[,hostname] key-type key
                if let Some(host_part) = trimmed.split_whitespace().next() {
                    // Skip hashed entries (start with |)
                    if !host_part.starts_with('|') {
                        for h in host_part.split(',') {
                            // Strip [host]:port format
                            let clean = h.trim_start_matches('[')
                                .split(']')
                                .next()
                                .unwrap_or(h);
                            if !known.contains(&clean.to_string()) {
                                known.push(clean.to_string());
                            }
                        }
                    }
                }
            }
        }

        let mut out = String::new();

        if hosts.is_empty() && known.is_empty() {
            return "No SSH hosts found in config or known_hosts.".to_string();
        }

        if !hosts.is_empty() {
            out.push_str(&format!("SSH config hosts ({}):\n", hosts.len()));
            for h in &hosts {
                out.push_str(&format!("{}\n", h));
            }
        }

        if !known.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("Known hosts ({}):\n", known.len()));
            for (i, h) in known.iter().enumerate() {
                out.push_str(&format!("  {}", h));
                if (i + 1) % 5 == 0 {
                    out.push('\n');
                } else {
                    out.push_str("  ");
                }
            }
            out.push('\n');
        }

        out
    }
}

// ── SSH Check Host ──

pub struct SshCheckHostTool;

impl Tool for SshCheckHostTool {
    fn name(&self) -> &'static str { "ssh_check_host" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "ssh" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "ssh_check_host",
                "description": "Check if an SSH host is reachable.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": {"type": "string", "description": "SSH host to check"}
                    },
                    "required": ["host"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();

        if let Err(e) = validate_host(host) {
            return format!("Error: {e}");
        }

        match std::process::Command::new("ssh")
            .args([
                "-o", "ConnectTimeout=3",
                "-o", "BatchMode=yes",
                "-o", "StrictHostKeyChecking=no",
                host,
                "exit",
            ])
            .output()
        {
            Ok(o) => {
                let code = o.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&o.stderr);
                match code {
                    0 => format!("{}: reachable (SSH connection succeeded)", host),
                    255 => format!("{}: unreachable or auth failed (exit 255). {}", host, stderr.trim()),
                    _ => format!("{}: exit code {}. {}", host, code, stderr.trim()),
                }
            }
            Err(e) => format!("Error running ssh: {e}"),
        }
    }
}

// ── SSH Run ──

pub struct SshRunTool;

impl Tool for SshRunTool {
    fn name(&self) -> &'static str { "ssh_run" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Dangerous }
    fn category(&self) -> &'static str { "ssh" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "ssh_run",
                "description": "Run a command on a remote host via SSH.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": {"type": "string", "description": "SSH host to connect to"},
                        "command": {"type": "string", "description": "Command to execute remotely"}
                    },
                    "required": ["host", "command"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or_default();

        if let Err(e) = validate_host(host) {
            return format!("Error: {e}");
        }
        if command.is_empty() {
            return "Error: command is required".to_string();
        }

        // Block harmful commands on remote hosts
        if let Some(reason) = crate::sanitize::detect_harmful_command(command) {
            return format!("Error: blocked — {reason}");
        }

        match std::process::Command::new("ssh")
            .args([
                "-o", "BatchMode=yes",
                "-o", "ConnectTimeout=5",
                host,
                command,
            ])
            .output()
        {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let stderr = String::from_utf8_lossy(&o.stderr);
                let code = o.status.code().unwrap_or(-1);

                let mut out = String::new();
                if !stdout.trim().is_empty() {
                    out.push_str(&stdout);
                }
                if !stderr.trim().is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&format!("(stderr) {}", stderr.trim()));
                }
                if out.trim().is_empty() {
                    out = format!("(exit code: {}, no output)", code);
                }

                // Truncate
                if out.len() > 3000 {
                    format!("{}...\n(truncated, {} chars)", &out[..3000], out.len())
                } else {
                    out
                }
            }
            Err(e) => format!("Error running ssh: {e}"),
        }
    }
}
