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
                "description": "Save current workspace state",
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
                "description": "Recall last saved workspace snapshot",
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

/// What an entry of a template may be: a plain app name, `^[a-z0-9][a-z0-9.+_-]*$`. Never a path
/// or a word with a `/`, so a script a Mind wrote cannot be a template's entry (#1124, round 9).
pub fn is_plain_app_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '+' | '_' | '-'))
}

/// `Ok` when every entry is a plain app name; otherwise the sentence that names the refused ones.
fn vet_entries(entries: &[String]) -> Result<(), String> {
    let refused: Vec<String> = entries
        .iter()
        .filter(|e| !is_plain_app_name(e))
        .map(|e| format!("`{}`", e.chars().take(80).collect::<String>()))
        .collect();
    if refused.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Refused: {} {} not a plain app name (lowercase letters, digits and . + _ -, no path). \
         A workspace template opens apps by name through the shell; it never runs a program or a script.",
        refused.join(", "),
        if refused.len() == 1 { "is" } else { "are" }
    ))
}

/// The apps of the template `name`: from the first saved one among `saved` (the text of each
/// recalled memory) that has an `Apps:` line, else from the built-in defaults.
fn apps_of(name: &str, saved: &[String]) -> Vec<String> {
    let header = format!("workspace template: {}", name.to_lowercase());
    for text in saved.iter().filter(|t| t.to_lowercase().contains(&header)) {
        let found: Vec<String> = text
            .lines()
            .find_map(|l| l.strip_prefix("Apps:"))
            .map(|rest| rest.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();
        if !found.is_empty() {
            return found;
        }
    }
    DEFAULT_TEMPLATES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, apps)| apps.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// Apply the template `name`: each entry is one `open_app` request to the shell's door, which
/// decides what a Mind may open and opens it walled. `ask` sends one request (`open_app <name>`'s
/// name) and returns the shell's answer. This file starts no program itself.
fn apply_with(name: &str, saved: &[String], mut ask: impl FnMut(&str) -> Result<(), String>) -> String {
    let apps = apps_of(name, saved);
    if apps.is_empty() {
        return format!("No workspace template '{}' found. Available defaults: {}", name,
            DEFAULT_TEMPLATES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "));
    }
    // Refused whole, before anything is opened: a template that holds a path starts nothing.
    if let Err(why) = vet_entries(&apps) {
        return format!("Workspace template '{name}' not applied. {why}");
    }
    let mut launched = Vec::new();
    let mut failed = Vec::new();
    for app in &apps {
        match ask(app) {
            Ok(()) => launched.push(app.as_str()),
            Err(e) => failed.push(format!("{app} ({e})")),
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

/// The shell's `open_app name=<app>`, by the one road (`app_act::gated_act`): the browser rule,
/// Mind View, `may_launch` and the card watch apply to each app.
fn open_in_shell(ctx: &ToolContext, app: &str) -> Result<(), String> {
    crate::app_act::gated_act(ctx, "shell", "open_app", &serde_json::json!({ "name": app }), "").map(|_| ())
}

/// Built-in default workspace templates (fallback when no custom template found).
const DEFAULT_TEMPLATES: &[(&str, &[&str])] = &[
    ("coding", &["terminal", "browser"]),
    ("writing", &["terminal"]),
    ("browsing", &["browser"]),
    ("research", &["browser", "terminal"]),
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
                "description": "Save workspace as reusable template",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Template name (e.g. 'coding', 'writing', 'research')"},
                        "apps": {
                            "type": "array", "items": {"type": "string"},
                            "description": "Apps to launch (e.g. ['terminal', 'browser'])"
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
        if let Err(why) = vet_entries(&apps) {
            return format!("Workspace template '{name}' not saved. {why}");
        }

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
                "description": "List saved workspace templates",
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
                "description": "Apply a saved workspace template",
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

        let saved: Vec<String> = ctx
            .db
            .recall_text(&format!("workspace template: {}", name), 3)
            .map(|results| results.into_iter().map(|r| r.text).collect())
            .unwrap_or_default();
        apply_with(name, &saved, |app| open_in_shell(ctx, app))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "/home/yantrik/.config/yantrik/scripts/w.sh";

    fn saved_w() -> Vec<String> {
        vec![format!("Workspace template: w\nApps: {SCRIPT}\n")]
    }

    #[test]
    fn plain_names_are_what_the_rule_says() {
        for ok in ["foot", "chromium", "gnome-terminal", "a.b+c_d", "0ad"] {
            assert!(is_plain_app_name(ok), "{ok}");
        }
        for bad in ["", SCRIPT, "./w.sh", "a/b", "Foot", "-x", ".x", "+x", "_x", "a b", "a;b", "a\nb", "a,b"] {
            assert!(!is_plain_app_name(bad), "{bad:?}");
        }
    }

    /// Round 9, finding 1: a template `w` whose entry is the script a Mind wrote is refused at
    /// save and again at apply, and nothing is asked of the shell.
    #[test]
    fn a_template_with_a_script_is_refused_at_save_and_again_at_apply() {
        let saving = vet_entries(&[SCRIPT.to_string()]).expect_err("a path is not an app name");
        assert!(saving.contains(SCRIPT) && saving.contains("Refused"), "{saving}");

        let mut asked: Vec<String> = Vec::new();
        let answer = apply_with("w", &saved_w(), |app| {
            asked.push(format!("open_app {app}"));
            Ok(())
        });
        assert!(asked.is_empty(), "{asked:?}");
        assert!(answer.contains("not applied") && answer.contains(SCRIPT), "{answer}");
    }

    /// One entry that is a path refuses the whole template: no other entry is opened either.
    #[test]
    fn one_bad_entry_opens_nothing() {
        let saved = vec![format!("Workspace template: mixed\nApps: foot, {SCRIPT}\n")];
        let mut asked = 0;
        let answer = apply_with("mixed", &saved, |_| {
            asked += 1;
            Ok(())
        });
        assert_eq!(asked, 0);
        assert!(answer.contains("not applied"), "{answer}");
    }

    /// `browsing` is exactly one `open_app browser` request: the shell's own name for its browser.
    #[test]
    fn browsing_is_one_open_app_browser_request() {
        let mut asked: Vec<String> = Vec::new();
        let answer = apply_with("browsing", &[], |app| {
            asked.push(format!("open_app {app}"));
            Ok(())
        });
        assert_eq!(asked, ["open_app browser"]);
        assert!(answer.contains("Launched: browser"), "{answer}");
    }

    #[test]
    fn a_saved_template_of_plain_names_asks_the_shell_for_each_and_names_a_failure() {
        let saved = vec!["Workspace template: dev\nApps: foot, chromium\n".to_string()];
        let answer = apply_with("dev", &saved, |app| if app == "chromium" { Err("not installed".to_string()) } else { Ok(()) });
        assert!(answer.contains("Launched: foot") && answer.contains("chromium (not installed)"), "{answer}");
    }

    /// The decision's text check: this file starts no program and names no `swaymsg` outside tests.
    #[test]
    fn this_file_starts_no_program() {
        let src = include_str!("workspace.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!code.contains("Command::new") && !code.contains("swaymsg"));
    }
}
