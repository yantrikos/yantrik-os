//! Firewall tools — status, rules, port management.
//! Uses `nft` (nftables) on modern Alpine, falls back to `iptables`.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(FirewallStatusTool));
    reg.register(Box::new(FirewallListRulesTool));
    reg.register(Box::new(FirewallAllowPortTool));
    reg.register(Box::new(FirewallBlockPortTool));
    reg.register(Box::new(FirewallBlockIpTool));
    reg.register(Box::new(FirewallEnableTool));
    reg.register(Box::new(FirewallDisableTool));
}

/// Detect which firewall backend is available.
fn firewall_backend() -> &'static str {
    if std::process::Command::new("nft")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "nft"
    } else {
        "iptables"
    }
}

/// Run a command and return stdout or stderr as a string.
fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, String> {
    match std::process::Command::new(cmd).args(args).output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).to_string()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            Err(format!("{} {}", out.trim(), err.trim()))
        }
        Err(e) => Err(format!("{cmd} not available: {e}")),
    }
}

// ── Firewall Status ──

pub struct FirewallStatusTool;

impl Tool for FirewallStatusTool {
    fn name(&self) -> &'static str { "firewall_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_status",
                "description": "Check if the firewall is active and show basic info",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = Vec::new();
        let backend = firewall_backend();
        info.push(format!("Backend: {backend}"));

        match backend {
            "nft" => {
                // Check if nftables service is loaded
                match run_cmd("nft", &["list", "tables"]) {
                    Ok(tables) => {
                        let table_count = tables.lines().count();
                        info.push(format!("Status: active ({table_count} tables loaded)"));
                        // Count rules
                        if let Ok(ruleset) = run_cmd("nft", &["list", "ruleset"]) {
                            let rule_count = ruleset.lines().filter(|l| l.trim().starts_with("meta") || l.trim().starts_with("tcp") || l.trim().starts_with("udp") || l.trim().starts_with("ip") || l.trim().starts_with("ct") || l.trim().starts_with("iif")).count();
                            info.push(format!("Rules: ~{rule_count}"));
                        }
                    }
                    Err(e) => {
                        info.push(format!("Status: inactive or not configured ({e})"));
                    }
                }
            }
            _ => {
                // iptables
                match run_cmd("iptables", &["-L", "-n", "--line-numbers"]) {
                    Ok(output) => {
                        let rule_count = output.lines().filter(|l| l.starts_with(|c: char| c.is_ascii_digit())).count();
                        info.push(format!("Status: active ({rule_count} rules)"));
                        // Extract default policies
                        for line in output.lines() {
                            if line.starts_with("Chain") {
                                info.push(format!("  {line}"));
                            }
                        }
                    }
                    Err(e) => {
                        info.push(format!("Status: unavailable ({e})"));
                    }
                }
            }
        }

        info.join("\n")
    }
}

// ── List Rules ──

pub struct FirewallListRulesTool;

impl Tool for FirewallListRulesTool {
    fn name(&self) -> &'static str { "firewall_list_rules" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_list_rules",
                "description": "List all current firewall rules",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "table": {"type": "string", "description": "Filter by table name (nftables) or chain (iptables). Optional."}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let table = args.get("table").and_then(|v| v.as_str()).unwrap_or("");

        match firewall_backend() {
            "nft" => {
                let result = if table.is_empty() {
                    run_cmd("nft", &["list", "ruleset"])
                } else {
                    // Validate table name (alphanumeric + underscore only)
                    if !table.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        return "Error: invalid table name".to_string();
                    }
                    run_cmd("nft", &["list", "table", "inet", table])
                };
                match result {
                    Ok(output) => {
                        if output.trim().is_empty() {
                            "No firewall rules configured.".to_string()
                        } else if output.len() > 3000 {
                            format!("{}\n... (truncated)", &output[..output.floor_char_boundary(3000)])
                        } else {
                            output
                        }
                    }
                    Err(e) => format!("Error listing rules: {e}"),
                }
            }
            _ => {
                let chain = if table.is_empty() { "" } else { table };
                let mut cmd_args = vec!["-L", "-n", "--line-numbers", "-v"];
                if !chain.is_empty() {
                    if !chain.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        return "Error: invalid chain name".to_string();
                    }
                    cmd_args.push(chain);
                }
                match run_cmd("iptables", &cmd_args) {
                    Ok(output) => {
                        if output.trim().is_empty() {
                            "No firewall rules.".to_string()
                        } else if output.len() > 3000 {
                            format!("{}\n... (truncated)", &output[..output.floor_char_boundary(3000)])
                        } else {
                            output
                        }
                    }
                    Err(e) => format!("Error: {e}"),
                }
            }
        }
    }
}

// ── Allow Port ──

pub struct FirewallAllowPortTool;

impl Tool for FirewallAllowPortTool {
    fn name(&self) -> &'static str { "firewall_allow_port" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_allow_port",
                "description": "Allow incoming traffic on a port (open a port in the",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "port": {"type": "integer", "description": "Port number (1-65535)"},
                        "protocol": {"type": "string", "enum": ["tcp", "udp", "both"], "description": "Protocol (default: tcp)"}
                    },
                    "required": ["port"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let port = args.get("port").and_then(|v| v.as_i64()).unwrap_or(0);
        let protocol = args.get("protocol").and_then(|v| v.as_str()).unwrap_or("tcp");

        if port < 1 || port > 65535 {
            return "Error: port must be 1-65535".to_string();
        }

        let protocols: Vec<&str> = if protocol == "both" { vec!["tcp", "udp"] } else { vec![protocol] };
        let mut results = Vec::new();

        for proto in &protocols {
            let result = match firewall_backend() {
                "nft" => run_cmd("nft", &[
                    "add", "rule", "inet", "filter", "input",
                    proto, "dport", &port.to_string(), "accept",
                ]),
                _ => run_cmd("iptables", &[
                    "-A", "INPUT", "-p", proto,
                    "--dport", &port.to_string(), "-j", "ACCEPT",
                ]),
            };
            match result {
                Ok(_) => results.push(format!("Allowed {proto}/{port}")),
                Err(e) => results.push(format!("Failed {proto}/{port}: {e}")),
            }
        }

        results.join("; ")
    }
}

// ── Block Port ──

pub struct FirewallBlockPortTool;

impl Tool for FirewallBlockPortTool {
    fn name(&self) -> &'static str { "firewall_block_port" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_block_port",
                "description": "Block incoming traffic on a port (close a port in the",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "port": {"type": "integer", "description": "Port number (1-65535)"},
                        "protocol": {"type": "string", "enum": ["tcp", "udp", "both"], "description": "Protocol (default: tcp)"}
                    },
                    "required": ["port"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let port = args.get("port").and_then(|v| v.as_i64()).unwrap_or(0);
        let protocol = args.get("protocol").and_then(|v| v.as_str()).unwrap_or("tcp");

        if port < 1 || port > 65535 {
            return "Error: port must be 1-65535".to_string();
        }

        let protocols: Vec<&str> = if protocol == "both" { vec!["tcp", "udp"] } else { vec![protocol] };
        let mut results = Vec::new();

        for proto in &protocols {
            let result = match firewall_backend() {
                "nft" => run_cmd("nft", &[
                    "add", "rule", "inet", "filter", "input",
                    proto, "dport", &port.to_string(), "drop",
                ]),
                _ => run_cmd("iptables", &[
                    "-A", "INPUT", "-p", proto,
                    "--dport", &port.to_string(), "-j", "DROP",
                ]),
            };
            match result {
                Ok(_) => results.push(format!("Blocked {proto}/{port}")),
                Err(e) => results.push(format!("Failed {proto}/{port}: {e}")),
            }
        }

        results.join("; ")
    }
}

// ── Block IP ──

pub struct FirewallBlockIpTool;

impl Tool for FirewallBlockIpTool {
    fn name(&self) -> &'static str { "firewall_block_ip" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_block_ip",
                "description": "Block all traffic from a specific IP address",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ip": {"type": "string", "description": "IPv4 or IPv6 address to block"}
                    },
                    "required": ["ip"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let ip = args.get("ip").and_then(|v| v.as_str()).unwrap_or_default();

        if ip.is_empty() {
            return "Error: ip is required".to_string();
        }

        // Validate IP format (basic check — hex digits, dots, colons for IPv6)
        if !ip.chars().all(|c: char| c.is_ascii_hexdigit() || c == '.' || c == ':') {
            return "Error: invalid IP address format".to_string();
        }

        match firewall_backend() {
            "nft" => {
                match run_cmd("nft", &[
                    "add", "rule", "inet", "filter", "input",
                    "ip", "saddr", ip, "drop",
                ]) {
                    Ok(_) => format!("Blocked all traffic from {ip}"),
                    Err(e) => format!("Failed to block {ip}: {e}"),
                }
            }
            _ => {
                match run_cmd("iptables", &[
                    "-A", "INPUT", "-s", ip, "-j", "DROP",
                ]) {
                    Ok(_) => format!("Blocked all traffic from {ip}"),
                    Err(e) => format!("Failed to block {ip}: {e}"),
                }
            }
        }
    }
}

// ── Enable Firewall ──

pub struct FirewallEnableTool;

impl Tool for FirewallEnableTool {
    fn name(&self) -> &'static str { "firewall_enable" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Dangerous }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_enable",
                "description": "Enable the firewall with a default drop policy",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match firewall_backend() {
            "nft" => {
                // Create a basic nftables ruleset with sane defaults
                let ruleset = r#"
table inet filter {
    chain input {
        type filter hook input priority 0; policy drop;
        ct state established,related accept
        iif lo accept
        icmp type echo-request accept
        tcp dport 22 accept
    }
    chain forward {
        type filter hook forward priority 0; policy drop;
    }
    chain output {
        type filter hook output priority 0; policy accept;
    }
}
"#;
                // Flush existing and load
                let _ = run_cmd("nft", &["flush", "ruleset"]);
                match std::process::Command::new("nft")
                    .arg("-f")
                    .arg("-")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(ref mut stdin) = child.stdin {
                            stdin.write_all(ruleset.as_bytes())?;
                        }
                        child.wait()
                    })
                {
                    Ok(status) if status.success() => {
                        "Firewall enabled. Default policy: drop incoming, accept outgoing. SSH (22) allowed.".to_string()
                    }
                    Ok(_) => "Failed to load nftables ruleset.".to_string(),
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => {
                // iptables: set default policies
                let steps = [
                    run_cmd("iptables", &["-P", "INPUT", "DROP"]),
                    run_cmd("iptables", &["-P", "FORWARD", "DROP"]),
                    run_cmd("iptables", &["-P", "OUTPUT", "ACCEPT"]),
                    run_cmd("iptables", &["-A", "INPUT", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-j", "ACCEPT"]),
                    run_cmd("iptables", &["-A", "INPUT", "-i", "lo", "-j", "ACCEPT"]),
                    run_cmd("iptables", &["-A", "INPUT", "-p", "icmp", "-j", "ACCEPT"]),
                    run_cmd("iptables", &["-A", "INPUT", "-p", "tcp", "--dport", "22", "-j", "ACCEPT"]),
                ];
                let failures: Vec<_> = steps.iter().filter(|r| r.is_err()).collect();
                if failures.is_empty() {
                    "Firewall enabled via iptables. Default: drop incoming, accept outgoing. SSH (22) allowed.".to_string()
                } else {
                    format!("Partially enabled with {} errors.", failures.len())
                }
            }
        }
    }
}

// ── Disable Firewall ──

pub struct FirewallDisableTool;

impl Tool for FirewallDisableTool {
    fn name(&self) -> &'static str { "firewall_disable" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Dangerous }
    fn category(&self) -> &'static str { "firewall" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "firewall_disable",
                "description": "Disable the firewall (accept all traffic)",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match firewall_backend() {
            "nft" => {
                match run_cmd("nft", &["flush", "ruleset"]) {
                    Ok(_) => "Firewall disabled — all rules flushed. System is unprotected.".to_string(),
                    Err(e) => format!("Failed to disable: {e}"),
                }
            }
            _ => {
                let _ = run_cmd("iptables", &["-P", "INPUT", "ACCEPT"]);
                let _ = run_cmd("iptables", &["-P", "FORWARD", "ACCEPT"]);
                let _ = run_cmd("iptables", &["-P", "OUTPUT", "ACCEPT"]);
                let _ = run_cmd("iptables", &["-F"]);
                "Firewall disabled — all rules flushed, policies set to ACCEPT.".to_string()
            }
        }
    }
}
