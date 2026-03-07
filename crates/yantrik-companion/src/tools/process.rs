//! Process tools — list_processes, system_info, diagnose_process.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ListProcessesTool));
    reg.register(Box::new(SystemInfoTool));
    reg.register(Box::new(DiagnoseProcessTool));
}

// ── List Processes ──

pub struct ListProcessesTool;

impl Tool for ListProcessesTool {
    fn name(&self) -> &'static str { "list_processes" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "process" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_processes",
                "description": "List running processes sorted by CPU or memory usage.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sort_by": {
                            "type": "string",
                            "enum": ["cpu", "memory", "name"],
                            "description": "Sort order (default: cpu)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max processes to show (default: 15)"
                        },
                        "filter": {
                            "type": "string",
                            "description": "Filter by process name (substring match)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let sort_by = args.get("sort_by").and_then(|v| v.as_str()).unwrap_or("cpu");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(15) as usize;
        let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");

        let sort_key = match sort_by {
            "memory" => "-%mem",
            "name" => "comm",
            _ => "-%cpu",
        };

        let ps_args = format!(
            "ps aux --sort={} | head -n {}",
            sort_key,
            limit + 1 // +1 for header
        );

        match std::process::Command::new("sh")
            .arg("-c")
            .arg(&ps_args)
            .output()
        {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                if filter.is_empty() {
                    text.to_string()
                } else {
                    // Keep header + matching lines
                    let filter_lower = filter.to_lowercase();
                    let mut lines: Vec<&str> = text.lines().collect();
                    if lines.is_empty() {
                        return "No processes found.".to_string();
                    }
                    let header = lines.remove(0);
                    let matched: Vec<&str> = lines
                        .into_iter()
                        .filter(|l| l.to_lowercase().contains(&filter_lower))
                        .take(limit)
                        .collect();
                    if matched.is_empty() {
                        format!("No processes matching '{filter}'")
                    } else {
                        format!("{}\n{}", header, matched.join("\n"))
                    }
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                format!("ps failed: {err}")
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

// ── System Info ──

pub struct SystemInfoTool;

impl Tool for SystemInfoTool {
    fn name(&self) -> &'static str { "system_info" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "process" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "system_info",
                "description": "Get system information: CPU, RAM, uptime, load average, hostname, kernel.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = Vec::new();

        // Hostname
        if let Ok(output) = std::process::Command::new("hostname").output() {
            if output.status.success() {
                let h = String::from_utf8_lossy(&output.stdout);
                info.push(format!("Hostname: {}", h.trim()));
            }
        }

        // Kernel
        if let Ok(output) = std::process::Command::new("uname").arg("-r").output() {
            if output.status.success() {
                let k = String::from_utf8_lossy(&output.stdout);
                info.push(format!("Kernel: {}", k.trim()));
            }
        }

        // Uptime + load
        if let Ok(output) = std::process::Command::new("uptime").output() {
            if output.status.success() {
                let u = String::from_utf8_lossy(&output.stdout);
                info.push(format!("Uptime: {}", u.trim()));
            }
        }

        // CPU info (model name + core count)
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            let model = content
                .lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let cores = content
                .lines()
                .filter(|l| l.starts_with("processor"))
                .count();
            info.push(format!("CPU: {} ({} cores)", model, cores));
        }

        // Memory (from /proc/meminfo)
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            let get_kb = |key: &str| -> u64 {
                content
                    .lines()
                    .find(|l| l.starts_with(key))
                    .and_then(|l| {
                        l.split_whitespace().nth(1).and_then(|v| v.parse().ok())
                    })
                    .unwrap_or(0)
            };
            let total = get_kb("MemTotal:");
            let available = get_kb("MemAvailable:");
            let used = total.saturating_sub(available);
            info.push(format!(
                "RAM: {} used / {} total ({:.0}%)",
                super::format_size(used * 1024),
                super::format_size(total * 1024),
                if total > 0 { used as f64 / total as f64 * 100.0 } else { 0.0 }
            ));
        }

        // Disk (root partition)
        if let Ok(output) = std::process::Command::new("df")
            .args(["-h", "/"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = text.lines().nth(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 5 {
                        info.push(format!(
                            "Disk (/): {} used / {} total ({})",
                            parts[2], parts[1], parts[4]
                        ));
                    }
                }
            }
        }

        if info.is_empty() {
            "Could not gather system information.".to_string()
        } else {
            info.join("\n")
        }
    }
}

// ── Diagnose Process ──

pub struct DiagnoseProcessTool;

impl Tool for DiagnoseProcessTool {
    fn name(&self) -> &'static str { "diagnose_process" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "process" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "diagnose_process",
                "description": "Deep-diagnose a running process: memory (RSS), CPU%, threads, children, open files. Use when you notice a process consuming resources.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Process name (e.g. firefox, chromium)"},
                        "pid": {"type": "integer", "description": "Specific PID (optional — finds by name if omitted)"}
                    },
                    "required": ["name"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let explicit_pid = args.get("pid").and_then(|v| v.as_u64()).map(|p| p as u32);

        if name.is_empty() {
            return "Error: name is required".to_string();
        }

        // Validate name
        if name.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.') {
            return "Error: invalid process name".to_string();
        }

        // Find PID
        let pid: u32 = if let Some(p) = explicit_pid {
            p
        } else {
            match std::process::Command::new("pgrep")
                .args(["-n", name]) // newest matching process
                .output()
            {
                Ok(o) if o.status.success() => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    match text.trim().parse() {
                        Ok(p) => p,
                        Err(_) => return format!("No process '{}' found.", name),
                    }
                }
                _ => return format!("No process '{}' found.", name),
            }
        };

        let mut info = Vec::new();

        // VmRSS and Threads from /proc/{pid}/status
        if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
            for line in status.lines() {
                if line.starts_with("VmRSS:") {
                    let kb: u64 = line.split_whitespace().nth(1)
                        .and_then(|v| v.parse().ok()).unwrap_or(0);
                    if kb > 1_048_576 {
                        info.push(format!("{:.1} GB RSS", kb as f64 / 1_048_576.0));
                    } else {
                        info.push(format!("{} MB RSS", kb / 1024));
                    }
                }
                if line.starts_with("Threads:") {
                    if let Some(n) = line.split_whitespace().nth(1) {
                        info.push(format!("{} threads", n));
                    }
                }
            }
        }

        // CPU% from ps
        if let Ok(o) = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "%cpu", "--no-headers"])
            .output()
        {
            if o.status.success() {
                let cpu = String::from_utf8_lossy(&o.stdout).trim().to_string();
                info.push(format!("{}% CPU", cpu));
            }
        }

        // Open file descriptors
        let fd_path = format!("/proc/{pid}/fd");
        if let Ok(entries) = std::fs::read_dir(&fd_path) {
            let count = entries.count();
            info.push(format!("{} open files", count));
        }

        // Child process count
        if let Ok(o) = std::process::Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
        {
            if o.status.success() {
                let children = String::from_utf8_lossy(&o.stdout)
                    .lines().count();
                if children > 0 {
                    info.push(format!("{} children", children));
                }
            }
        }

        if info.is_empty() {
            format!("{} (PID {}): process exists but no details readable.", name, pid)
        } else {
            format!("{} (PID {}): {}.", name, pid, info.join(", "))
        }
    }
}
