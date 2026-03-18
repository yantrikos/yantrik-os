//! Service management tools — service_list, service_control, service_status.
//! Targets Alpine Linux (OpenRC: rc-service, rc-update).
//! Falls back to systemctl for systemd distros.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ServiceListTool));
    reg.register(Box::new(ServiceControlTool));
    reg.register(Box::new(ServiceStatusTool));
}

/// Detect init system.
fn init_system() -> &'static str {
    if std::path::Path::new("/sbin/rc-service").exists() {
        "openrc"
    } else if std::path::Path::new("/usr/bin/systemctl").exists()
        || std::path::Path::new("/bin/systemctl").exists()
    {
        "systemd"
    } else {
        "unknown"
    }
}

/// Services that should never be stopped/restarted by the AI.
const PROTECTED_SERVICES: &[&str] = &[
    "dbus", "seatd", "udev", "eudev", "networking", "labwc",
    "yantrik", "pipewire", "wireplumber", "sshd", "cron",
];

// ── Service List ──

pub struct ServiceListTool;

impl Tool for ServiceListTool {
    fn name(&self) -> &'static str { "service_list" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "service" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "service_list",
                "description": "List system services",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "filter": {"type": "string", "description": "Filter services by name"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");

        match init_system() {
            "openrc" => {
                match std::process::Command::new("rc-status").arg("-a").output() {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        if filter.is_empty() {
                            let trunc = if text.len() > 2000 { &text[..text.floor_char_boundary(2000)] } else { &text };
                            trunc.to_string()
                        } else {
                            let f = filter.to_lowercase();
                            let matched: Vec<&str> = text
                                .lines()
                                .filter(|l| l.to_lowercase().contains(&f))
                                .take(30)
                                .collect();
                            if matched.is_empty() {
                                format!("No services matching '{filter}'")
                            } else {
                                matched.join("\n")
                            }
                        }
                    }
                    Ok(o) => format!("rc-status failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            "systemd" => {
                let mut cmd = std::process::Command::new("systemctl");
                cmd.args(["list-units", "--type=service", "--no-pager"]);
                match cmd.output() {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        if filter.is_empty() {
                            let trunc = if text.len() > 2000 { &text[..text.floor_char_boundary(2000)] } else { &text };
                            trunc.to_string()
                        } else {
                            let f = filter.to_lowercase();
                            let matched: Vec<&str> = text
                                .lines()
                                .filter(|l| l.to_lowercase().contains(&f))
                                .take(30)
                                .collect();
                            if matched.is_empty() {
                                format!("No services matching '{filter}'")
                            } else {
                                matched.join("\n")
                            }
                        }
                    }
                    Ok(o) => format!("systemctl failed: {}", String::from_utf8_lossy(&o.stderr)),
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => "Error: no supported init system found".to_string(),
        }
    }
}

// ── Service Control ──

pub struct ServiceControlTool;

impl Tool for ServiceControlTool {
    fn name(&self) -> &'static str { "service_control" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "service" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "service_control",
                "description": "Start, stop, or restart a system service",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {"type": "string", "description": "Service name"},
                        "action": {
                            "type": "string",
                            "enum": ["start", "stop", "restart"],
                            "description": "Action to perform"
                        }
                    },
                    "required": ["service", "action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str()).unwrap_or_default();
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or_default();

        if service.is_empty() || action.is_empty() {
            return "Error: service and action are required".to_string();
        }

        // Validate service name
        if service.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.') {
            return "Error: invalid service name".to_string();
        }

        // Check protected list
        let svc_lower = service.to_lowercase();
        for protected in PROTECTED_SERVICES {
            if svc_lower == *protected {
                return format!("Error: '{service}' is a protected service");
            }
        }

        match init_system() {
            "openrc" => {
                match std::process::Command::new("rc-service")
                    .args([service, action])
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        format!("Service '{service}': {action} OK")
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        format!("Failed: {}", err.trim())
                    }
                    Err(e) => format!("Error: {e}"),
                }
            }
            "systemd" => {
                match std::process::Command::new("systemctl")
                    .args([action, service])
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        format!("Service '{service}': {action} OK")
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        format!("Failed: {}", err.trim())
                    }
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => "Error: no supported init system".to_string(),
        }
    }
}

// ── Service Status ──

pub struct ServiceStatusTool;

impl Tool for ServiceStatusTool {
    fn name(&self) -> &'static str { "service_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "service" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "service_status",
                "description": "Show status of a system service",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {"type": "string", "description": "Service name"}
                    },
                    "required": ["service"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = args.get("service").and_then(|v| v.as_str()).unwrap_or_default();
        if service.is_empty() {
            return "Error: service is required".to_string();
        }

        if service.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.') {
            return "Error: invalid service name".to_string();
        }

        match init_system() {
            "openrc" => {
                match std::process::Command::new("rc-service")
                    .args([service, "status"])
                    .output()
                {
                    Ok(o) => {
                        let out = String::from_utf8_lossy(&o.stdout);
                        let err = String::from_utf8_lossy(&o.stderr);
                        format!("{} {}", out.trim(), err.trim()).trim().to_string()
                    }
                    Err(e) => format!("Error: {e}"),
                }
            }
            "systemd" => {
                match std::process::Command::new("systemctl")
                    .args(["status", service, "--no-pager", "-l"])
                    .output()
                {
                    Ok(o) => {
                        let out = String::from_utf8_lossy(&o.stdout);
                        let trunc = if out.len() > 2000 { &out[..out.floor_char_boundary(2000)] } else { &out };
                        trunc.to_string()
                    }
                    Err(e) => format!("Error: {e}"),
                }
            }
            _ => "Error: no supported init system".to_string(),
        }
    }
}
