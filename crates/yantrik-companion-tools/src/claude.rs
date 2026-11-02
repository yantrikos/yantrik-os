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

        // An agent that sends the task to Anthropic under the person's sign-in and may then run
        // commands and write files on this machine: asked about first, with the whole task (#1054).
        crate::outward::acting(
            &crate::outward::OutwardAct { tool: "claude_code", to: "Claude Code under the person's sign-in", what: task, more: cwd },
            || run_claude_code(task, cwd),
        )
    }
}

/// Every built-in tool Claude Code has that reads, writes, runs or fetches. `claude_think` is an
/// answer and nothing else, so none of them is available to it, whatever the person has allowed
/// for their own work in `~/.claude/settings.json` (#1063: print mode still loads those rules).
const THINK_DISALLOWED: &str = "Bash Edit MultiEdit Write NotebookEdit Read Glob Grep LS WebFetch WebSearch Task TodoWrite";

/// The arguments of a `claude_think` call: print mode, no tools, no MCP servers (an empty config,
/// and only that one), and the prompt after `--` so one beginning with `-` cannot add options.
fn think_args(prompt: &str) -> Vec<String> {
    [
        "-p",
        "--no-input",
        "--disallowedTools",
        THINK_DISALLOWED,
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--",
        prompt,
    ]
    .iter()
    .map(|a| a.to_string())
    .collect()
}

/// A directory of the call's own with nothing in it, so no project settings or MCP file is found.
fn empty_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yantrik-claude-think-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Run claude CLI in print mode (text response only, no tools).
fn run_claude(prompt: &str, _verbose: bool) -> String {
    // Options first, then `--`, then the prompt: `-p` is print mode and the prompt is a positional
    // argument, so a prompt beginning with `-` would otherwise be parsed as more options.
    let result = std::process::Command::new("claude")
        .args(think_args(prompt))
        // An empty directory of its own: no project settings, no `.mcp.json`, nothing to read.
        .current_dir(empty_dir())
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
    // As in `run_claude`: the task after `--`, so one beginning with `-` cannot add options —
    // `--dangerously-skip-permissions`, say, or a wider `--allowedTools`.
    let result = std::process::Command::new("claude")
        .arg("-p")
        .arg("--allowedTools")
        .arg("Bash,Read,Write,Glob,Grep")
        .arg("--no-input")
        .arg("--")
        .arg(task)
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

#[cfg(all(test, unix))]
mod outward_tests {
    //! `claude_code` hands a task to an agent that may run commands and write files as the person
    //! (#1054): it asks first, with the whole task, and a no starts nothing.
    use super::*;
    use crate::outward_testing::{actions, first_card, installed, shown, with_ctx};

    #[test]
    fn a_task_for_claude_code_raises_a_card_with_the_whole_task_and_a_no_starts_nothing() {
        let (_shell, heard) = installed("claude-no", "pending", "denied");
        let said = with_ctx(|ctx| ClaudeCodeTool.execute(ctx, &serde_json::json!({ "task": "delete the build; then push", "working_directory": "/tmp" })));

        assert!(said.contains("said no") && said.contains("Nothing was done"), "{said}");
        let card = first_card(&heard);
        assert_eq!(card["args_json"]["tool"], "claude_code");
        assert_eq!(shown(&card, "what"), "delete the build; then push");
        assert_eq!(shown(&card, "more"), "/tmp");
        assert!(!actions(&heard).contains(&"consume_approval".to_string()));
    }
}

#[cfg(test)]
mod think_tests {
    use super::*;

    #[test]
    fn a_think_call_has_no_tools_no_mcp_servers_and_the_prompt_after_the_separator() {
        let args = think_args("--dangerously-skip-permissions; summarise this");
        let at = |flag: &str| args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("{flag} missing: {args:?}"));
        for tool in ["Bash", "Write", "WebFetch", "Read", "Task"] {
            assert!(args[at("--disallowedTools") + 1].split(' ').any(|t| t == tool), "{tool} is not disallowed");
        }
        assert!(at("--strict-mcp-config") < at("--"));
        assert_eq!(args[at("--mcp-config") + 1], r#"{"mcpServers":{}}"#);
        assert_eq!(args[at("--") + 1], "--dangerously-skip-permissions; summarise this");
        assert_eq!(args.len(), at("--") + 2, "nothing after the prompt");
    }
}
