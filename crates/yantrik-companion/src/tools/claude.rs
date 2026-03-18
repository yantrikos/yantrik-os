//! Claude Code integration — delegates complex reasoning to Claude via CLI.
//!
//! Uses the `claude` CLI (Claude Code) in print mode for single-shot responses.
//! The local Qwen model can call this tool when it needs deeper reasoning,
//! better planning, or more reliable tool-call generation.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ClaudeThinkTool));
    reg.register(Box::new(ClaudeCodeTool));
}

// ── claude_think ──
// For reasoning, analysis, planning — Claude responds with text.

struct ClaudeThinkTool;

impl Tool for ClaudeThinkTool {
    fn name(&self) -> &'static str { "claude_think" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "ai" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "claude_think",
                "description": "Delegate a complex reasoning task to Claude (a more capable AI). \
                    Use this when you need: deep analysis, complex planning, code generation, \
                    multi-step reasoning, or when you're unsure about something. \
                    Claude will return a text response — it cannot call tools directly.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The question or task for Claude. Be specific about what you need."
                        },
                        "context": {
                            "type": "string",
                            "description": "Optional context to include (file contents, previous results, etc.)"
                        }
                    },
                    "required": ["prompt"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: prompt".to_string(),
        };

        let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");

        let full_prompt = if context.is_empty() {
            prompt.to_string()
        } else {
            format!("{}\n\nContext:\n{}", prompt, context)
        };

        run_claude(&full_prompt, false)
    }
}

// ── claude_code ──
// For tasks that benefit from Claude's coding/system capabilities.
// Runs with --allowedTools to let Claude execute commands if needed.

struct ClaudeCodeTool;

impl Tool for ClaudeCodeTool {
    fn name(&self) -> &'static str { "claude_code" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "ai" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "claude_code",
                "description": "Ask Claude Code to perform a task with full tool access (can read/write files, \
                    run commands, search code). Use for: writing code, debugging, file operations, \
                    system administration tasks that need intelligence. Claude will execute the task \
                    and return the result.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "What Claude should do. Be specific."
                        },
                        "working_directory": {
                            "type": "string",
                            "description": "Optional working directory for Claude. Default: /home/yantrik"
                        }
                    },
                    "required": ["task"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: task".to_string(),
        };

        let cwd = args.get("working_directory")
            .and_then(|v| v.as_str())
            .unwrap_or("/home/yantrik");

        run_claude_code(task, cwd)
    }
}

/// Run claude CLI in print mode (text response only, no tools).
fn run_claude(prompt: &str, _verbose: bool) -> String {
    let result = std::process::Command::new("claude")
        .arg("-p")
        .arg(prompt)
        .arg("--no-input")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if stdout.trim().is_empty() {
                    "Claude returned empty response".to_string()
                } else {
                    // Truncate very long responses
                    if stdout.len() > 8000 {
                        format!("{}...\n[truncated, {} total chars]", &stdout[..stdout.floor_char_boundary(8000)], stdout.len())
                    } else {
                        stdout
                    }
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                format!("Claude CLI error (exit {}): {}", output.status, stderr.trim())
            }
        }
        Err(e) => {
            format!("Failed to run claude CLI: {}. Is it installed? (npm install -g @anthropic-ai/claude-code)", e)
        }
    }
}

/// Run claude CLI with tool access (can execute commands, read/write files).
fn run_claude_code(task: &str, cwd: &str) -> String {
    let result = std::process::Command::new("claude")
        .arg("-p")
        .arg(task)
        .arg("--allowedTools")
        .arg("Bash,Read,Write,Glob,Grep")
        .arg("--no-input")
        .current_dir(cwd)
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if stdout.trim().is_empty() {
                    "Claude completed task (no output)".to_string()
                } else {
                    if stdout.len() > 8000 {
                        format!("{}...\n[truncated, {} total chars]", &stdout[..stdout.floor_char_boundary(8000)], stdout.len())
                    } else {
                        stdout
                    }
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                format!("Claude Code error (exit {}): {}", output.status, stderr.trim())
            }
        }
        Err(e) => {
            format!("Failed to run claude CLI: {}", e)
        }
    }
}
