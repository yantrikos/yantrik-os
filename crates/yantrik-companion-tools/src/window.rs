//! Window management tools — list_windows, focus_window, close_window.
//! Listing reads the compositor with wlrctl (wlroots-based compositors, labwc). Moving a window
//! is asked of the shell over the socket bus: the shell owns the one resolver that turns a
//! caller's words into an exact window title, and the one set of command lines wlrctl accepts.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::app_ui::{act_asking, Desk};

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

// ── Moving windows: asked of the shell ──
//
// Focus and close used to run `wlrctl toplevel <verb> <whatever the caller said>` right here.
// A bare word is read by wlrctl as an app_id; our Slint windows declare none; and a match on
// nothing exits zero — so both tools answered "Focused window: Notes" over a desktop that had
// not moved (#83), and close could not match at all unless the caller's string happened to be
// a window's exact, case-sensitive title. The shell's own `focus_window` and `close_window`
// actions, published on the bus as `app-shell`, resolve the caller's words against the real
// window list — part of a title, any case — and ask wlrctl with `title:<exact>`, so those are
// what these tools call now. The refusals belong there too, where the window list is: the
// shell declines an ambiguous close rather than coin-tossing somebody's tab, and declines the
// desktop itself with a sentence that offers `lock`. The local guards went with the command
// lines: the metacharacter check refused real titles like "Ask | Hacker News - Chromium" to
// protect a shell invocation that no longer happens (the title travels as a JSON string), and
// "contains yantrik" caught any window whose title merely mentioned it — a terminal sitting at
// `yantrik@home: ~` among them.

/// Bring the window answering to `title` to the front. Returns the title the shell resolved it
/// to — the window's own exact title, not necessarily the caller's words.
///
/// `Err` is a sentence for the model to relay: the shell's refusal (which names what IS open),
/// or what became of an approval card if the shell's action is graded above the machine's mode.
fn focus_via(desk: &Desk, title: &str) -> Result<String, String> {
    let reply = act_asking(
        desk,
        "shell",
        "focus_window",
        &serde_json::json!({ "title": title }),
        "",
        "Bring an open window to the front",
    )?;
    Ok(reply["result"]["focused"].as_str().unwrap_or(title).to_string())
}

/// Ask the shell to close the window answering to `title`, the way pressing its × does.
///
/// Returns the title the shell resolved it to, and the shell's own caveat: a close is a
/// request, and an app holding unsaved work may answer with its own dialog and stay.
fn close_via(desk: &Desk, title: &str) -> Result<(String, String), String> {
    let reply = act_asking(
        desk,
        "shell",
        "close_window",
        &serde_json::json!({ "title": title }),
        "",
        "Ask an open window to close, as pressing its × does",
    )?;
    let closing = reply["result"]["closing"].as_str().unwrap_or(title).to_string();
    let note = reply["result"]["note"].as_str().unwrap_or("").to_string();
    Ok((closing, note))
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
                        "title": {"type": "string", "description": "Window title, or part of one"}
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

        match focus_via(&Desk::session(), title) {
            Ok(focused) => format!("Focused window: {focused}"),
            // Already a sentence for the model: the shell's own refusal names what IS open,
            // and a card's outcome says what the person decided.
            Err(why) => why,
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
                "description": "Ask a desktop window to close, as pressing its × does; an app \
                                with unsaved work may put up its own dialog and stay",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "Window title, or part of one"}
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

        match close_via(&Desk::session(), title) {
            Ok((closing, note)) => {
                let mut out = format!("Asked window to close: {closing}");
                if !note.is_empty() {
                    out.push('\n');
                    out.push_str(&note);
                }
                out
            }
            Err(why) => why,
        }
    }
}

// ── Focus Context ──

/// The title half of one `wlrctl toplevel list` line, which reads `app_id: title` — `: title`
/// for our own windows, which declare no app_id — or the whole line for a foreign window that
/// printed no separator at all.
fn line_title(line: &str) -> &str {
    match line.split_once(':') {
        Some((_, rest)) if !rest.trim().is_empty() => rest.trim(),
        _ => line.trim(),
    }
}

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

        // Focus the first relevant window — by its title, through the shell, like
        // focus_window. This used to hand wlrctl the whole list line, separator and all, as a
        // bare-word matchspec: that matched nothing and exited zero, and the report below
        // claimed "Focused" regardless.
        let mut focused = String::new();
        if let Some(best) = relevant.first() {
            let title = line_title(best);
            focused = match focus_via(&Desk::session(), title) {
                Ok(resolved) => format!("\nFocused: {resolved}"),
                Err(why) => format!("\nCould not focus {title}: {why}"),
            };
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

        report.push_str(&focused);

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch desk whose `app-shell` socket answers whatever `reply` makes of each request,
    /// through the same fake the `app_ui` tests drive `act_asking` with.
    #[cfg(unix)]
    fn desk_with_shell(
        tag: &str,
        reply: impl Fn(&serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    ) -> (Desk, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>) {
        let dir = std::env::temp_dir()
            .join(format!("yantrik-companion-window-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let heard = crate::app_ui::tests::fake(dir.join("app-shell.sock"), reply);
        let desk = Desk {
            dir: Some(dir),
            wait: std::time::Duration::from_millis(600),
            poll: std::time::Duration::from_millis(20),
        };
        (desk, heard)
    }

    /// The caller's words travel to the shell whole, and the shell's answer — which names the
    /// window ITS resolver found — comes back.
    #[cfg(unix)]
    #[test]
    fn focus_and_close_are_asked_of_the_shell_in_the_callers_own_words() {
        let (desk, heard) = desk_with_shell("asked", |params| match params["action"].as_str() {
            Some("focus_window") => serde_json::json!({"result": {
                "accepted": true, "action_id": "app-shell#1", "settled": false,
                "result": { "focused": "Ask | Hacker News - Chromium" },
            }}),
            Some("close_window") => serde_json::json!({"result": {
                "accepted": true, "action_id": "app-shell#2", "settled": false,
                "result": {
                    "closing": "Notes",
                    "note": "the window was asked to close, which is what pressing × does",
                },
            }}),
            _ => serde_json::json!({"error": {"code": -32602, "message": "unknown action"}}),
        });

        // "hacker news" is part of a title, in the wrong case, and the real title holds a `|` —
        // a string the old tool refused outright for its metacharacters and the old command
        // line could never have matched. Resolving is the shell's job; the tool reports the
        // window the shell found, not the words it was given.
        let focused = focus_via(&desk, "hacker news").expect("the shell resolved the title");
        assert_eq!(focused, "Ask | Hacker News - Chromium");

        let (closing, note) = close_via(&desk, "notes").expect("the shell resolved the title");
        assert_eq!(closing, "Notes");
        assert!(note.contains("asked to close"), "the caveat rides along: {note}");

        let asked = heard.lock().unwrap().clone();
        assert_eq!(asked.len(), 2, "one call each, over the bus: {asked:?}");
        assert_eq!(asked[0]["action"], "focus_window");
        assert_eq!(asked[0]["args"], serde_json::json!({ "title": "hacker news" }));
        assert_eq!(asked[1]["action"], "close_window");
        assert_eq!(asked[1]["args"], serde_json::json!({ "title": "notes" }));
    }

    /// The defect named in the issue, in one sentence: the old tools answered "Focused window:
    /// gimp" on a machine with no gimp, because a wlrctl match on nothing exits zero. The
    /// shell's refusal arrives as an error and leaves as this tool's answer.
    #[cfg(unix)]
    #[test]
    fn a_window_nothing_answers_to_is_a_refusal_not_a_success() {
        let (desk, heard) = desk_with_shell("nomatch", |_| {
            serde_json::json!({"error": {"code": -32602,
                "message": "no open window matches `gimp`; there is: Notes, Terminal"}})
        });
        let err = focus_via(&desk, "gimp").unwrap_err();
        assert!(err.contains("no open window matches `gimp`"), "{err}");
        assert!(err.contains("there is: Notes, Terminal"), "the refusal says what IS open: {err}");
        let err = close_via(&desk, "gimp").unwrap_err();
        assert!(err.contains("no open window matches"), "{err}");
        assert_eq!(heard.lock().unwrap().len(), 2, "both verbs asked; neither invented a success");
    }

    #[test]
    fn the_title_comes_off_a_list_line_whole() {
        // What `wlrctl toplevel list` prints: `app_id: title`, `: title` for our own windows
        // (no app_id declared), and a bare line for a foreign window with no separator.
        assert_eq!(line_title(": Terminal"), "Terminal");
        assert_eq!(line_title("firefox: Mozilla Firefox"), "Mozilla Firefox");
        assert_eq!(line_title(": Notes: Handover"), "Notes: Handover");
        assert_eq!(line_title("Some Foreign Window"), "Some Foreign Window");
    }

    /// No wlrctl focus or close command line is built in this file again (#83).
    ///
    /// Every one it built was wrong the same silent way — a bare word wlrctl reads as an
    /// app_id, matching nothing, exiting zero — and the tools reported success over a desktop
    /// that had not moved. The shell's window actions are the one path that resolves a name
    /// against the open windows before asking the compositor; this file's job is to reach them.
    #[test]
    fn moving_a_window_is_asked_of_the_shell_and_never_of_a_bare_wlrctl_word() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("window.rs");
        let text = std::fs::read_to_string(&path).expect("this file is in the crate it tests");
        // Everything above the tests, which may quote a broken command line in order to name it.
        let body = text.split_once("#[cfg(test)]").map(|(b, _)| b).unwrap_or(&text);
        for verb in ["focus", "close"] {
            assert!(
                !body.contains(&format!("\"toplevel\", \"{verb}\"")),
                "window.rs builds a `wlrctl toplevel {verb}` command line again. Ask the \
                 shell's `{verb}_window` action instead — it resolves the title against the \
                 open windows first, which is the whole of the fix for #83."
            );
        }
        assert!(
            body.contains("act_asking"),
            "focus and close must reach the shell over the socket bus, not a process of their own"
        );
    }
}
