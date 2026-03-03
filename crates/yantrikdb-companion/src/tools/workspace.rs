//! Workspace tools — save and recall session state.
//!
//! Captures terminal CWDs, git branches, recent commands, and open windows.
//! Stores as episodic memories in YantrikDB for cross-session continuity.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(SaveWorkspaceTool));
    reg.register(Box::new(RecallWorkspaceTool));
    reg.register(Box::new(SaveWorkspaceTemplateTool));
    reg.register(Box::new(ListWorkspaceTemplatesTool));
    reg.register(Box::new(ApplyWorkspaceTemplateTool));
}

// ── Save Workspace ──

pub struct SaveWorkspaceTool;

impl Tool for SaveWorkspaceTool {
    fn name(&self) -> &'static str { "save_workspace" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "workspace" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "save_workspace",
                "description": "Save a snapshot of the current workspace state — terminal directories, git branches, recent commands. Call this when the user is logging out, shutting down, or switching contexts.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "note": {
                            "type": "string",
                            "description": "Optional note about what the user was working on"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let note = args.get("note").and_then(|v| v.as_str()).unwrap_or("");
        let mut parts = Vec::new();

        // Collect terminal CWDs from /proc
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.chars().all(|c| c.is_ascii_digit()) {
                    let comm_path = format!("/proc/{}/comm", name);
                    let cwd_path = format!("/proc/{}/cwd", name);
                    if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                        let comm = comm.trim();
                        if matches!(comm, "ash" | "bash" | "zsh" | "fish" | "sh") {
                            if let Ok(cwd) = std::fs::read_link(&cwd_path) {
                                parts.push(format!("terminal:{} in {}", comm, cwd.display()));
                            }
                        }
                    }
                }
            }
        }

        // Collect git info from common project dirs
        let home = std::env::var("HOME").unwrap_or_default();
        for dir in &["projects", "code", "src", "repos", "dev"] {
            let base = format!("{}/{}", home, dir);
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten().take(10) {
                    let git_head = entry.path().join(".git/HEAD");
                    if git_head.exists() {
                        if let Ok(head) = std::fs::read_to_string(&git_head) {
                            let branch = head
                                .strip_prefix("ref: refs/heads/")
                                .unwrap_or(&head)
                                .trim();
                            parts.push(format!(
                                "git:{} on branch {}",
                                entry.file_name().to_string_lossy(),
                                branch
                            ));
                        }
                    }
                }
            }
        }

        // Recent shell commands
        for hist_file in &[".ash_history", ".bash_history", ".zsh_history"] {
            let path = format!("{}/{}", home, hist_file);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let recent: Vec<&str> = content.lines().rev().take(10).collect();
                if !recent.is_empty() {
                    let recent: Vec<&str> = recent.into_iter().rev().collect();
                    parts.push(format!("recent_commands:{}", recent.join(" | ")));
                    break;
                }
            }
        }

        // Build the snapshot text
        let snapshot = if parts.is_empty() && note.is_empty() {
            "Workspace snapshot: no active terminals or projects detected.".to_string()
        } else {
            let mut text = String::from("Workspace snapshot:\n");
            for p in &parts {
                text.push_str(&format!("  {}\n", p));
            }
            if !note.is_empty() {
                text.push_str(&format!("  note: {}\n", note));
            }
            text
        };

        // Store in memory as episodic event with high importance
        match ctx.db.record_text(
            &snapshot,
            "episodic",
            0.8, // high importance for session state
            0.0,
            604800.0, // 1 week TTL
            &serde_json::json!({"type": "workspace_snapshot"}),
            "default",
            0.9,
            "work",
            "system",
            None,
        ) {
            Ok(rid) => format!("Workspace saved (memory #{rid}). {}", snapshot),
            Err(e) => format!("Failed to save workspace: {e}"),
        }
    }
}

// ── Recall Workspace ──

pub struct RecallWorkspaceTool;

impl Tool for RecallWorkspaceTool {
    fn name(&self) -> &'static str { "recall_workspace" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "workspace" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "recall_workspace",
                "description": "Recall the last saved workspace snapshot — what the user was working on, which directories, git branches, and recent commands. Use this when resuming a session or when the user asks 'where was I?' or 'what was I doing?'.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match ctx.db.recall_text("workspace snapshot", 3) {
            Ok(results) if !results.is_empty() => {
                let mut text = String::from("Recent workspace snapshots:\n");
                for (i, r) in results.iter().enumerate() {
                    text.push_str(&format!("\n--- Session {} ---\n{}\n", i + 1, r.text));
                }
                text
            }
            Ok(_) => "No workspace snapshots found. This might be your first session.".to_string(),
            Err(e) => format!("Failed to recall workspace: {e}"),
        }
    }
}

// ── Workspace Templates ──

/// Built-in default workspace templates (fallback when no custom template found).
const DEFAULT_TEMPLATES: &[(&str, &[&str])] = &[
    ("coding", &["foot", "chromium"]),
    ("writing", &["foot"]),
    ("browsing", &["chromium"]),
    ("research", &["chromium", "foot"]),
];

pub struct SaveWorkspaceTemplateTool;

impl Tool for SaveWorkspaceTemplateTool {
    fn name(&self) -> &'static str { "save_workspace_template" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "workspace" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "save_workspace_template",
                "description": "Save a named workspace template. Captures which apps to launch for a specific activity. Use when the user says 'save this as my coding workspace' or 'remember this setup'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Template name (e.g. 'coding', 'writing', 'research')"},
                        "apps": {
                            "type": "array", "items": {"type": "string"},
                            "description": "Apps to launch (e.g. ['foot', 'chromium'])"
                        },
                        "description": {"type": "string", "description": "What this workspace is for"}
                    },
                    "required": ["name"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("default");
        let apps: Vec<String> = args.get("apps")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("");

        let mut text = format!("Workspace template: {}\n", name);
        if !desc.is_empty() {
            text.push_str(&format!("Description: {}\n", desc));
        }
        if !apps.is_empty() {
            text.push_str(&format!("Apps: {}\n", apps.join(", ")));
        }

        match ctx.db.record_text(
            &text,
            "semantic",
            0.9,
            0.0,
            0.0, // permanent (no decay)
            &serde_json::json!({"type": "workspace_template", "template_name": name}),
            "default",
            1.0,
            "workspace/templates",
            "user",
            None,
        ) {
            Ok(rid) => format!("Workspace template '{}' saved (#{}).", name, rid),
            Err(e) => format!("Failed to save template: {}", e),
        }
    }
}

pub struct ListWorkspaceTemplatesTool;

impl Tool for ListWorkspaceTemplatesTool {
    fn name(&self) -> &'static str { "list_workspace_templates" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "workspace" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_workspace_templates",
                "description": "List all saved workspace templates.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match ctx.db.recall_text("workspace template", 10) {
            Ok(results) if !results.is_empty() => {
                let mut text = String::from("Saved workspace templates:\n");
                for r in &results {
                    if r.text.starts_with("Workspace template:") {
                        text.push_str(&format!("  - {}\n", r.text.lines().next().unwrap_or("")));
                    }
                }
                // Also list built-in defaults
                text.push_str("\nBuilt-in defaults:\n");
                for (name, apps) in DEFAULT_TEMPLATES {
                    text.push_str(&format!("  - {} ({})\n", name, apps.join(", ")));
                }
                text
            }
            Ok(_) => {
                let mut text = String::from("No custom workspace templates saved.\n\nBuilt-in defaults:\n");
                for (name, apps) in DEFAULT_TEMPLATES {
                    text.push_str(&format!("  - {} ({})\n", name, apps.join(", ")));
                }
                text
            }
            Err(e) => format!("Failed to list templates: {e}"),
        }
    }
}

pub struct ApplyWorkspaceTemplateTool;

impl Tool for ApplyWorkspaceTemplateTool {
    fn name(&self) -> &'static str { "apply_workspace_template" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "workspace" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "apply_workspace_template",
                "description": "Apply a workspace template — launch the apps associated with a named template. Looks up saved templates first, falls back to built-in defaults (coding, writing, browsing, research).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Template name to apply (e.g. 'coding', 'writing')"}
                    },
                    "required": ["name"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("default");

        // Try to find a saved template
        let apps = match ctx.db.recall_text(&format!("workspace template: {}", name), 3) {
            Ok(results) => {
                // Look for a matching template
                let mut found_apps = Vec::new();
                for r in &results {
                    if r.text.to_lowercase().contains(&format!("workspace template: {}", name.to_lowercase())) {
                        // Parse apps from "Apps: foot, chromium" line
                        for line in r.text.lines() {
                            if line.starts_with("Apps:") {
                                found_apps = line[5..].split(',').map(|s| s.trim().to_string()).collect();
                                break;
                            }
                        }
                        if !found_apps.is_empty() {
                            break;
                        }
                    }
                }
                if found_apps.is_empty() {
                    // Fallback to defaults
                    DEFAULT_TEMPLATES.iter()
                        .find(|(n, _)| *n == name)
                        .map(|(_, apps)| apps.iter().map(|s| s.to_string()).collect())
                        .unwrap_or_default()
                } else {
                    found_apps
                }
            }
            Err(_) => {
                DEFAULT_TEMPLATES.iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, apps)| apps.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default()
            }
        };

        if apps.is_empty() {
            return format!("No workspace template '{}' found. Available defaults: {}", name,
                DEFAULT_TEMPLATES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "));
        }

        let mut launched = Vec::new();
        let mut failed = Vec::new();

        for app in &apps {
            match std::process::Command::new(app).spawn() {
                Ok(_) => launched.push(app.as_str()),
                Err(e) => {
                    // Try via swaymsg exec (for Wayland apps)
                    match std::process::Command::new("swaymsg")
                        .args(["exec", app])
                        .spawn()
                    {
                        Ok(_) => launched.push(app.as_str()),
                        Err(_) => failed.push(format!("{} ({})", app, e)),
                    }
                }
            }
        }

        let mut report = format!("Applied workspace template '{}'.\n", name);
        if !launched.is_empty() {
            report.push_str(&format!("Launched: {}\n", launched.join(", ")));
        }
        if !failed.is_empty() {
            report.push_str(&format!("Failed to launch: {}\n", failed.join(", ")));
        }
        report
    }
}
