//! Window management tools — list_windows, focus_window, close_window.
//! Uses wlrctl for wlroots-based compositors (labwc).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ListWindowsTool));
    reg.register(Box::new(FocusWindowTool));
    reg.register(Box::new(CloseWindowTool));
    reg.register(Box::new(FocusContextTool));
}

/// Run wlrctl and return output.
fn wlrctl(args: &[&str]) -> Result<String, String> {
    match std::process::Command::new("wlrctl").args(args).output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).to_string()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let out = String::from_utf8_lossy(&o.stdout);
            Err(format!("{} {}", out.trim(), err.trim()))
        }
        Err(e) => Err(format!("wlrctl not available: {e}")),
    }
}

// ── List Windows ──

pub struct ListWindowsTool;

impl Tool for ListWindowsTool {
    fn name(&self) -> &'static str { "list_windows" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "window" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_windows",
                "description": "List desktop app windows; not browser tabs",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        // Try wlrctl first
        match wlrctl(&["toplevel", "list"]) {
            Ok(output) => {
                if output.trim().is_empty() {
                    "No open windows.".to_string()
                } else {
                    let lines: Vec<&str> = output.lines().take(30).collect();
                    format!("Open windows ({}):\n{}", lines.len(), lines.join("\n"))
                }
            }
            Err(_) => {
                // Fallback: use wmctrl (X11 compat)
                match std::process::Command::new("wmctrl").arg("-l").output() {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout);
                        if text.trim().is_empty() {
                            "No open windows.".to_string()
                        } else {
                            text.to_string()
                        }
                    }
                    _ => "Error: window listing requires wlrctl or wmctrl".to_string(),
                }
            }
        }
    }
}

// ── Focus Window ──

pub struct FocusWindowTool;

impl Tool for FocusWindowTool {
    fn name(&self) -> &'static str { "focus_window" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "window" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "focus_window",
                "description": "Focus (bring to front) a window by title or app name",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "Window title or app name to focus"}
                    },
                    "required": ["title"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        if title.is_empty() {
            return "Error: title is required".to_string();
        }

        // Validate no metacharacters
        if title.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
            return "Error: title contains invalid characters".to_string();
        }

        match wlrctl(&["toplevel", "focus", title]) {
            Ok(_) => format!("Focused window: {title}"),
            Err(e) => format!("Failed to focus '{title}': {e}"),
        }
    }
}

// ── Close Window ──

pub struct CloseWindowTool;

impl Tool for CloseWindowTool {
    fn name(&self) -> &'static str { "close_window" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "window" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "close_window",
                "description": "Close a desktop window; not browser tabs",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "Window title or app name to close"}
                    },
                    "required": ["title"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        if title.is_empty() {
            return "Error: title is required".to_string();
        }

        if title.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&') {
            return "Error: title contains invalid characters".to_string();
        }

        // Don't close yantrik itself
        if title.to_lowercase().contains("yantrik") {
            return "Error: cannot close Yantrik shell".to_string();
        }

        match wlrctl(&["toplevel", "close", title]) {
            Ok(_) => format!("Closed window: {title}"),
            Err(e) => format!("Failed to close '{title}': {e}"),
        }
    }
}

// ── Focus Context ──

pub struct FocusContextTool;

impl Tool for FocusContextTool {
    fn name(&self) -> &'static str { "focus_context" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "window" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "focus_context",
                "description": "Organize windows for a task context",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "context": {
                            "type": "string",
                            "description": "Task context: coding, browsing, writing, communication, media, research"
                        }
                    },
                    "required": ["context"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("general");

        let output = match wlrctl(&["toplevel", "list"]) {
            Ok(o) => o,
            Err(e) => return format!("Error listing windows: {e}"),
        };

        if output.trim().is_empty() {
            return "No open windows to organize.".to_string();
        }

        let windows: Vec<&str> = output.lines()
            .filter(|l| !l.trim().is_empty())
            .collect();

        // Classify each window by relevance to context
        let relevant_keywords: &[&str] = match context {
            "coding" => &["terminal", "foot", "vim", "nvim", "code", "editor", "git", "cargo"],
            "browsing" => &["firefox", "chromium", "browser", "chrome"],
            "writing" => &["editor", "note", "text", "libreoffice", "writer", "gedit"],
            "communication" => &["telegram", "chat", "slack", "discord", "mail", "email"],
            "media" => &["mpv", "player", "music", "video", "spotify"],
            "research" => &["firefox", "chromium", "browser", "terminal", "foot", "pdf"],
            _ => &[],
        };

        let mut relevant = Vec::new();
        let mut irrelevant = Vec::new();

        for win in &windows {
            let lower = win.to_lowercase();
            // Skip yantrik itself
            if lower.contains("yantrik") {
                continue;
            }
            let is_relevant = relevant_keywords.iter().any(|kw| lower.contains(kw));
            if is_relevant {
                relevant.push(*win);
            } else {
                irrelevant.push(*win);
            }
        }

        // Focus the first relevant window
        if let Some(best) = relevant.first() {
            let _ = wlrctl(&["toplevel", "focus", best]);
        }

        let mut report = format!("Context: {}\n\nRelevant windows ({}):\n", context, relevant.len());
        for w in &relevant {
            report.push_str(&format!("  [KEEP] {}\n", w));
        }
        if !irrelevant.is_empty() {
            report.push_str(&format!("\nDistractors ({}):\n", irrelevant.len()));
            for w in &irrelevant {
                report.push_str(&format!("  [DISTRACTOR] {}\n", w));
            }
            report.push_str("\nAsk the user if they want to close the distractors.");
        } else {
            report.push_str("\nNo distractors found — all windows are relevant.");
        }

        if let Some(best) = relevant.first() {
            report.push_str(&format!("\nFocused: {}", best));
        }

        report
    }
}
