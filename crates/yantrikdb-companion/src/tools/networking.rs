//! General networking tools — interfaces, ping, traceroute, ports, DNS, VPN.
//! Complements `wifi.rs` (WiFi-specific) and `network.rs` (download/fetch).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(NetworkInterfacesTool));
    reg.register(Box::new(NetworkPingTool));
    reg.register(Box::new(NetworkTracerouteTool));
    reg.register(Box::new(NetworkPortsTool));
    reg.register(Box::new(NetworkDnsTool));
    reg.register(Box::new(NetworkDnsSetTool));
    reg.register(Box::new(NetworkVpnStatusTool));
}

/// Validate a hostname or IP (no shell metacharacters).
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    if host.len() > 253 {
        return Err("hostname too long".to_string());
    }
    if host.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&' || c == ' ' || c == '\'' || c == '"') {
        return Err("host contains invalid characters".to_string());
    }
    Ok(())
}

// ── Network Interfaces ──

pub struct NetworkInterfacesTool;

impl Tool for NetworkInterfacesTool {
    fn name(&self) -> &'static str { "network_interfaces" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_interfaces",
                "description": "List all network interfaces with their status, IP addresses, MAC addresses, and link state.",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        // `ip -brief addr` gives a concise view
        match std::process::Command::new("ip")
            .args(["-br", "addr"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if text.trim().is_empty() {
                    "No network interfaces found.".to_string()
                } else {
                    let mut result = String::from("Network interfaces:\n");
                    for line in text.lines() {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let iface = parts[0];
                            let state = parts[1];
                            let addrs = if parts.len() > 2 {
                                parts[2..].join(", ")
                            } else {
                                "no address".to_string()
                            };
                            result.push_str(&format!("  {} [{}] — {}\n", iface, state, addrs));
                        }
                    }

                    // Also get link-layer info (MAC, speed)
                    if let Ok(link_out) = std::process::Command::new("ip")
                        .args(["-br", "link"])
                        .output()
                    {
                        if link_out.status.success() {
                            let link_text = String::from_utf8_lossy(&link_out.stdout);
                            result.push_str("\nMAC addresses:\n");
                            for line in link_text.lines() {
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                if parts.len() >= 3 {
                                    result.push_str(&format!("  {} — {}\n", parts[0], parts[2]));
                                }
                            }
                        }
                    }

                    result
                }
            }
            Ok(o) => format!("Error: {}", String::from_utf8_lossy(&o.stderr)),
            Err(e) => format!("Error (ip not available): {e}"),
        }
    }
}

// ── Ping ──

pub struct NetworkPingTool;

impl Tool for NetworkPingTool {
    fn name(&self) -> &'static str { "network_ping" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_ping",
                "description": "Ping a host to check connectivity and measure latency. Sends 4 packets.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": {"type": "string", "description": "Hostname or IP address to ping"},
                        "count": {"type": "integer", "description": "Number of packets (default: 4, max: 10)"}
                    },
                    "required": ["host"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        let count = args.get("count").and_then(|v| v.as_i64()).unwrap_or(4).min(10).max(1);

        if let Err(e) = validate_host(host) {
            return format!("Error: {e}");
        }

        match std::process::Command::new("ping")
            .args(["-c", &count.to_string(), "-W", "5", host])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Extract the summary lines (last 2-3 lines)
                let lines: Vec<&str> = stdout.lines().collect();
                let mut result = Vec::new();

                // First line (PING header)
                if let Some(first) = lines.first() {
                    result.push(first.to_string());
                }

                // Stats lines (usually last 2 lines)
                for line in lines.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev() {
                    if line.contains("packets") || line.contains("rtt") || line.contains("round-trip") {
                        result.push(line.to_string());
                    }
                }

                if result.is_empty() {
                    if output.status.success() {
                        stdout.to_string()
                    } else {
                        format!("Host {} is unreachable.", host)
                    }
                } else {
                    result.join("\n")
                }
            }
            Err(e) => format!("Error (ping not available): {e}"),
        }
    }
}

// ── Traceroute ──

pub struct NetworkTracerouteTool;

impl Tool for NetworkTracerouteTool {
    fn name(&self) -> &'static str { "network_traceroute" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_traceroute",
                "description": "Trace the route to a host, showing each hop and latency.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": {"type": "string", "description": "Hostname or IP to trace route to"},
                        "max_hops": {"type": "integer", "description": "Maximum hops (default: 15, max: 30)"}
                    },
                    "required": ["host"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        let max_hops = args.get("max_hops").and_then(|v| v.as_i64()).unwrap_or(15).min(30).max(1);

        if let Err(e) = validate_host(host) {
            return format!("Error: {e}");
        }

        // Try traceroute, fallback to tracepath (common on minimal installs)
        let has_traceroute = std::process::Command::new("traceroute")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let cmd = if has_traceroute { "traceroute" } else { "tracepath" };
        let hops_str = max_hops.to_string();
        let cmd_args: Vec<&str> = if has_traceroute {
            vec!["-m", &hops_str, "-w", "3", host]
        } else {
            vec!["-m", &hops_str, host]
        };

        match std::process::Command::new(cmd)
            .args(&cmd_args)
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.trim().is_empty() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    format!("Traceroute failed: {}", stderr.trim())
                } else if stdout.len() > 3000 {
                    format!("{}\n... (truncated)", &stdout[..3000])
                } else {
                    stdout.to_string()
                }
            }
            Err(_) => "Neither traceroute nor tracepath is available. Install with: apk add traceroute".to_string(),
        }
    }
}

// ── Open Ports ──

pub struct NetworkPortsTool;

impl Tool for NetworkPortsTool {
    fn name(&self) -> &'static str { "network_ports" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_ports",
                "description": "List all open/listening network ports with the process using them.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "state": {"type": "string", "enum": ["listening", "established", "all"], "description": "Filter by connection state (default: listening)"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("listening");

        // Use `ss` (socket statistics) — available on all modern Linux
        let filter = match state {
            "listening" => vec!["-tlnp"],
            "established" => vec!["-tnp", "state", "established"],
            "all" => vec!["-tanp"],
            _ => vec!["-tlnp"],
        };

        match std::process::Command::new("ss").args(&filter).output() {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                if text.trim().is_empty() {
                    format!("No {} ports found.", state)
                } else if text.len() > 3000 {
                    format!("{}\n... (truncated)", &text[..3000])
                } else {
                    text.to_string()
                }
            }
            Ok(output) => {
                // Fallback to netstat
                let text = String::from_utf8_lossy(&output.stderr);
                match std::process::Command::new("netstat").args(["-tlnp"]).output() {
                    Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                    _ => format!("ss failed: {}", text.trim()),
                }
            }
            Err(e) => format!("Error (ss not available): {e}"),
        }
    }
}

// ── DNS Info ──

pub struct NetworkDnsTool;

impl Tool for NetworkDnsTool {
    fn name(&self) -> &'static str { "network_dns" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_dns",
                "description": "Show current DNS servers and resolve configuration.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "lookup": {"type": "string", "description": "Optional: resolve a hostname to see which DNS server answers"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let lookup = args.get("lookup").and_then(|v| v.as_str()).unwrap_or("");
        let mut info = Vec::new();

        // Read /etc/resolv.conf
        if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
            info.push("DNS configuration (/etc/resolv.conf):".to_string());
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    info.push(format!("  {trimmed}"));
                }
            }
        } else {
            info.push("Cannot read /etc/resolv.conf".to_string());
        }

        // Optional lookup
        if !lookup.is_empty() {
            if let Err(e) = validate_host(lookup) {
                return format!("Error: {e}");
            }
            info.push(format!("\nResolving '{lookup}':"));
            match std::process::Command::new("nslookup")
                .arg(lookup)
                .output()
            {
                Ok(o) if o.status.success() => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    for line in text.lines() {
                        info.push(format!("  {}", line.trim()));
                    }
                }
                _ => {
                    // Fallback to getent
                    match std::process::Command::new("getent")
                        .args(["hosts", lookup])
                        .output()
                    {
                        Ok(o) if o.status.success() => {
                            let text = String::from_utf8_lossy(&o.stdout);
                            info.push(format!("  {}", text.trim()));
                        }
                        _ => info.push("  Resolution failed.".to_string()),
                    }
                }
            }
        }

        info.join("\n")
    }
}

// ── Set DNS ──

pub struct NetworkDnsSetTool;

impl Tool for NetworkDnsSetTool {
    fn name(&self) -> &'static str { "network_dns_set" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_dns_set",
                "description": "Set DNS servers. Common options: Cloudflare (1.1.1.1), Google (8.8.8.8), Quad9 (9.9.9.9).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "primary": {"type": "string", "description": "Primary DNS server IP (e.g. 1.1.1.1)"},
                        "secondary": {"type": "string", "description": "Secondary DNS server IP (e.g. 1.0.0.1)"}
                    },
                    "required": ["primary"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let primary = args.get("primary").and_then(|v| v.as_str()).unwrap_or_default();
        let secondary = args.get("secondary").and_then(|v| v.as_str()).unwrap_or("");

        if primary.is_empty() {
            return "Error: primary DNS server is required".to_string();
        }

        // Validate IP format
        for ip in [primary, secondary] {
            if !ip.is_empty() && !ip.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ':') {
                return format!("Error: invalid DNS server IP: {ip}");
            }
        }

        // Build resolv.conf content
        let mut content = format!("nameserver {primary}\n");
        if !secondary.is_empty() {
            content.push_str(&format!("nameserver {secondary}\n"));
        }

        // Backup existing resolv.conf
        let _ = std::fs::copy("/etc/resolv.conf", "/etc/resolv.conf.bak");

        match std::fs::write("/etc/resolv.conf", &content) {
            Ok(()) => {
                let mut msg = format!("DNS set to {primary}");
                if !secondary.is_empty() {
                    msg.push_str(&format!(", {secondary}"));
                }
                msg.push_str(". Previous config backed up to /etc/resolv.conf.bak");
                msg
            }
            Err(e) => {
                format!("Failed to write /etc/resolv.conf: {e}. Try running as root.")
            }
        }
    }
}

// ── VPN Status ──

pub struct NetworkVpnStatusTool;

impl Tool for NetworkVpnStatusTool {
    fn name(&self) -> &'static str { "network_vpn_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "networking" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "network_vpn_status",
                "description": "Check VPN connection status. Detects WireGuard, OpenVPN, and nmcli VPN connections.",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let mut info = Vec::new();

        // Check WireGuard
        match std::process::Command::new("wg").arg("show").output() {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if text.trim().is_empty() {
                    info.push("WireGuard: no active tunnels".to_string());
                } else {
                    info.push("WireGuard: active".to_string());
                    for line in text.lines().take(10) {
                        info.push(format!("  {}", line.trim()));
                    }
                }
            }
            _ => {
                info.push("WireGuard: not installed".to_string());
            }
        }

        // Check OpenVPN
        match std::process::Command::new("pgrep")
            .args(["-a", "openvpn"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if !text.trim().is_empty() {
                    info.push("OpenVPN: running".to_string());
                    for line in text.lines().take(3) {
                        info.push(format!("  {}", line.trim()));
                    }
                } else {
                    info.push("OpenVPN: not running".to_string());
                }
            }
            _ => {
                info.push("OpenVPN: not detected".to_string());
            }
        }

        // Check nmcli VPN connections
        if let Ok(o) = std::process::Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"])
            .output()
        {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                for line in text.lines() {
                    if line.contains("vpn") || line.contains("wireguard") || line.contains("tun") {
                        info.push(format!("NM VPN: {}", line.replace(':', " | ")));
                    }
                }
            }
        }

        // Check tun/tap interfaces
        if let Ok(o) = std::process::Command::new("ip")
            .args(["-br", "link", "show", "type", "tun"])
            .output()
        {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout);
                if !text.trim().is_empty() {
                    info.push(format!("TUN interfaces: {}", text.trim()));
                }
            }
        }

        if info.is_empty() {
            "No VPN connections detected.".to_string()
        } else {
            info.join("\n")
        }
    }
}
