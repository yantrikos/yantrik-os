//! Docker tools — container management via the Docker CLI.
//!
//! Tools: docker_ps, docker_images, docker_logs, docker_start, docker_stop, docker_exec.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DockerPsTool));
    reg.register(Box::new(DockerImagesTool));
    reg.register(Box::new(DockerLogsTool));
    reg.register(Box::new(DockerStartTool));
    reg.register(Box::new(DockerStopTool));
    reg.register(Box::new(DockerExecTool));
}

// ── Helpers ──

/// Validate a container name — only alphanumeric, dash, underscore, dot allowed.
fn sanitize_container_name(name: &str) -> Result<&str, String> {
    if name.is_empty() {
        return Err("container name is required".to_string());
    }
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        Ok(name)
    } else {
        Err(format!("Invalid container name '{}': only a-z, A-Z, 0-9, '-', '_', '.' are allowed", name))
    }
}

/// Run a docker command and return its output.
fn run_docker(args: &[&str]) -> String {
    match std::process::Command::new("docker")
        .args(args)
        .output()
    {
        Ok(o) if o.status.success() => {
            let out = String::from_utf8_lossy(&o.stdout);
            if out.trim().is_empty() {
                "(no output)".to_string()
            } else {
                out.to_string()
            }
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            format!("{} {}", out.trim(), err.trim())
        }
        Err(e) => format!("Error (docker not available?): {e}"),
    }
}

/// Truncate a string to max_len chars, appending a notice.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...\n(truncated, {} chars)", &s[..max_len], s.len())
    } else {
        s.to_string()
    }
}

// ── Docker PS ──

pub struct DockerPsTool;

impl Tool for DockerPsTool {
    fn name(&self) -> &'static str { "docker_ps" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "docker" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "docker_ps",
                "description": "List running Docker containers.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let out = run_docker(&[
            "ps", "--format",
            "table {{.Names}}\t{{.Status}}\t{{.Ports}}\t{{.Image}}"
        ]);
        truncate(&out, 2000)
    }
}

// ── Docker Images ──

pub struct DockerImagesTool;

impl Tool for DockerImagesTool {
    fn name(&self) -> &'static str { "docker_images" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "docker" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "docker_images",
                "description": "List Docker images.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let out = run_docker(&[
            "images", "--format",
            "table {{.Repository}}\t{{.Tag}}\t{{.Size}}"
        ]);
        truncate(&out, 2000)
    }
}

// ── Docker Logs ──

pub struct DockerLogsTool;

impl Tool for DockerLogsTool {
    fn name(&self) -> &'static str { "docker_logs" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "docker" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "docker_logs",
                "description": "View logs from a Docker container.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "container": {"type": "string", "description": "Container name or ID"},
                        "lines": {"type": "integer", "description": "Number of lines to show (default: 50)"}
                    },
                    "required": ["container"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let container = args.get("container").and_then(|v| v.as_str()).unwrap_or_default();
        let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(50);

        let container = match sanitize_container_name(container) {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };

        let tail_arg = format!("{}", lines);
        let out = run_docker(&["logs", "--tail", &tail_arg, container]);
        truncate(&out, 3000)
    }
}

// ── Docker Start ──

pub struct DockerStartTool;

impl Tool for DockerStartTool {
    fn name(&self) -> &'static str { "docker_start" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "docker" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "docker_start",
                "description": "Start a stopped Docker container.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "container": {"type": "string", "description": "Container name or ID"}
                    },
                    "required": ["container"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let container = args.get("container").and_then(|v| v.as_str()).unwrap_or_default();

        let container = match sanitize_container_name(container) {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };

        run_docker(&["start", container])
    }
}

// ── Docker Stop ──

pub struct DockerStopTool;

impl Tool for DockerStopTool {
    fn name(&self) -> &'static str { "docker_stop" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "docker" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "docker_stop",
                "description": "Stop a running Docker container.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "container": {"type": "string", "description": "Container name or ID"}
                    },
                    "required": ["container"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let container = args.get("container").and_then(|v| v.as_str()).unwrap_or_default();

        let container = match sanitize_container_name(container) {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };

        run_docker(&["stop", container])
    }
}

// ── Docker Exec ──

pub struct DockerExecTool;

impl Tool for DockerExecTool {
    fn name(&self) -> &'static str { "docker_exec" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Dangerous }
    fn category(&self) -> &'static str { "docker" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "docker_exec",
                "description": "Execute a command inside a Docker container.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "container": {"type": "string", "description": "Container name or ID"},
                        "command": {"type": "string", "description": "Command to execute"}
                    },
                    "required": ["container", "command"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let container = args.get("container").and_then(|v| v.as_str()).unwrap_or_default();
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or_default();

        let container = match sanitize_container_name(container) {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };

        if command.is_empty() {
            return "Error: command is required".to_string();
        }

        // Block harmful commands even inside containers
        if let Some(reason) = crate::sanitize::detect_harmful_command(command) {
            return format!("Error: blocked — {reason}");
        }

        let out = run_docker(&["exec", container, "sh", "-c", command]);
        truncate(&out, 3000)
    }
}
