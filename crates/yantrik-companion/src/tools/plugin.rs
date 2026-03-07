//! YAML Plugin system — load user-defined tools from ~/.config/yantrik/plugins/*.yaml.
//!
//! Plugin YAML format:
//! ```yaml
//! name: "my-tools"
//! version: "1.0"
//! tools:
//!   - name: "check_vpn"
//!     description: "Check if VPN is connected"
//!     permission: "safe"
//!     category: "network"
//!     parameters: {}
//!     command: "mullvad status"
//!   - name: "deploy_staging"
//!     description: "Deploy branch to staging"
//!     permission: "sensitive"
//!     category: "devops"
//!     parameters:
//!       branch: { type: "string", description: "Branch name", required: true }
//!     command: "cd ~/projects && ./deploy.sh {branch}"
//! ```
//!
//! Tools use `Box::leak()` for dynamic `&'static str` names — loaded once at startup,
//! lives for program lifetime.

use std::collections::HashMap;

use serde::Deserialize;

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, parse_permission, expand_home};

/// A single tool definition from a plugin YAML.
#[derive(Deserialize)]
struct PluginToolDef {
    name: String,
    description: String,
    #[serde(default = "default_safe")]
    permission: String,
    #[serde(default = "default_custom")]
    category: String,
    #[serde(default)]
    parameters: HashMap<String, ParamDef>,
    command: String,
}

fn default_safe() -> String { "safe".to_string() }
fn default_custom() -> String { "custom".to_string() }

#[derive(Deserialize)]
struct ParamDef {
    #[serde(default = "default_string_type")]
    r#type: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    required: bool,
}

fn default_string_type() -> String { "string".to_string() }

/// Plugin manifest file.
#[derive(Deserialize)]
struct PluginManifest {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    #[serde(default)]
    version: String,
    #[serde(default)]
    tools: Vec<PluginToolDef>,
}

/// A tool created from a YAML plugin definition.
struct PluginTool {
    tool_name: &'static str,
    tool_description: &'static str,
    tool_permission: PermissionLevel,
    tool_category: &'static str,
    tool_params: Vec<(&'static str, &'static str, &'static str, bool)>, // (name, type, desc, required)
    command_template: String,
}

impl Tool for PluginTool {
    fn name(&self) -> &'static str { self.tool_name }
    fn permission(&self) -> PermissionLevel { self.tool_permission }
    fn category(&self) -> &'static str { self.tool_category }

    fn definition(&self) -> serde_json::Value {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();

        for (name, ty, desc, req) in &self.tool_params {
            props.insert(
                name.to_string(),
                serde_json::json!({"type": ty, "description": desc}),
            );
            if *req {
                required.push(serde_json::Value::String(name.to_string()));
            }
        }

        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.tool_name,
                "description": self.tool_description,
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": required,
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        // Substitute {param_name} in command template
        let mut cmd = self.command_template.clone();

        if let Some(obj) = args.as_object() {
            for (key, val) in obj {
                let val_str = match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };

                // Sanitize parameter values — reject shell metacharacters
                if val_str.chars().any(|c| matches!(c, ';' | '&' | '|' | '`' | '$' | '>' | '<')) {
                    return format!("Error: parameter '{}' contains forbidden characters", key);
                }

                cmd = cmd.replace(&format!("{{{}}}", key), &val_str);
            }
        }

        // Expand ~ in command
        let cmd = if cmd.contains("~/") {
            cmd.replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()))
        } else {
            cmd
        };

        match std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&cmd)
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let mut result = String::new();

                if !stdout.is_empty() {
                    result.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str("stderr: ");
                    result.push_str(&stderr);
                }

                if result.is_empty() {
                    if output.status.success() {
                        "Command completed successfully (no output).".to_string()
                    } else {
                        format!("Command failed with exit code: {}", output.status)
                    }
                } else if result.len() > 3000 {
                    format!("{}\n[Truncated — {} bytes]", &result[..3000], result.len())
                } else {
                    result
                }
            }
            Err(e) => format!("Error executing plugin command: {e}"),
        }
    }
}

/// Load all plugins from ~/.config/yantrik/plugins/ and register their tools.
pub fn load_plugins(reg: &mut ToolRegistry) {
    let plugins_dir = expand_home("~/.config/yantrik/plugins");
    let entries = match std::fs::read_dir(&plugins_dir) {
        Ok(e) => e,
        Err(_) => return, // No plugins directory — that's fine
    };

    let mut loaded_count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if !name.ends_with(".yaml") && !name.ends_with(".yml") {
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_yaml::from_str::<PluginManifest>(&content) {
                Ok(manifest) => {
                    let plugin_name = manifest.name.clone();
                    let tool_count = manifest.tools.len();

                    for def in manifest.tools {
                        let params: Vec<_> = def
                            .parameters
                            .iter()
                            .map(|(k, v)| {
                                (
                                    leak_str(&k),
                                    leak_str(&v.r#type),
                                    leak_str(&v.description),
                                    v.required,
                                )
                            })
                            .collect();

                        let tool = PluginTool {
                            tool_name: leak_str(&def.name),
                            tool_description: leak_str(&def.description),
                            tool_permission: parse_permission(&def.permission),
                            tool_category: leak_str(&def.category),
                            tool_params: params,
                            command_template: def.command,
                        };
                        reg.register(Box::new(tool));
                    }

                    tracing::info!(
                        plugin = plugin_name,
                        tools = tool_count,
                        "Loaded plugin"
                    );
                    loaded_count += tool_count;
                }
                Err(e) => {
                    tracing::warn!(file = %name, error = %e, "Failed to parse plugin YAML");
                }
            },
            Err(e) => {
                tracing::warn!(file = %name, error = %e, "Failed to read plugin file");
            }
        }
    }

    if loaded_count > 0 {
        tracing::info!(total = loaded_count, "Plugin tools loaded");
    }
}

/// Leak a String into a &'static str. Acceptable for startup-loaded names
/// that live for the program's lifetime.
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
