//! A tool that acts outward cannot be added without asking the person (#1054).
//!
//! The companion's tools used to send, post, submit and call other parties' tools under their own
//! ceiling with nothing in front of the person (#1026, #1054). Every tool that does is now gated
//! by `yantrik_companion_tools::outward`, the shell's approvals module: a card in `ask`, `auto`
//! and plain bypass, plan refuses, full bypass skips it and is written down.
//!
//! This is the registry that keeps it so. Every tool in the companion's crates is in exactly one
//! of two lists, found by reading the sources, so a new tool that is in neither fails here and its
//! author has to decide which it is:
//!
//! - [`OUTWARD`]: acts outside this machine on the person's behalf, with how it is gated. The test
//!   reads the tool's own implementation for that gate.
//! - [`NOT_OUTWARD`]: stays on this machine or only reads, with the reason.
//!
//! A tool that is outward and calls no gate, a name in both lists, a name in neither, and a name
//! listed that no source defines (a renamed or removed tool left behind) all fail.

use std::path::{Path, PathBuf};

/// How an outward tool is gated, and so what the test looks for in its implementation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gate {
    /// It asks through `outward` (`acting`, `ask_person_before_acting`) or through the browser
    /// helpers that do (`before_press`, `before_submit`).
    Acts,
    /// It runs code or a command on this machine and asks unless it is provably contained (the
    /// strict sandbox, a read-only command): `acting_unless`.
    Contained,
    /// It reads from an address the model chose and asks once the conversation holds something of
    /// the person's: `reading`.
    Reads,
    /// It sends mail through `smtp_client::send_email`, which asks first (`email::smtp_client`).
    Mail,
    /// It acts through an app's own action, and `act_asking` raises the shell's card when the
    /// app's grade and the person's mode say so.
    App,
}

/// Every tool that acts outside this machine on the person's behalf. Dynamic tools are named by
/// their type, with a leading `?`.
const OUTWARD: &[(&str, Gate)] = &[
    // Messages to other people, in the person's name.
    ("telegram_send", Gate::Acts),
    ("whatsapp_send", Gate::Acts),
    ("email_send", Gate::Mail),
    ("email_reply", Gate::Mail),
    ("email_reply_all", Gate::Mail),
    ("email_forward", Gate::Mail),
    // Pressing and submitting in a browser the person is signed in to; signing in.
    ("browser_click", Gate::Acts),
    ("browser_click_element", Gate::Acts),
    ("browser_click_xy", Gate::Acts),
    ("browser_type", Gate::Acts),
    ("browser_type_element", Gate::Acts),
    ("browser_type_xy", Gate::Acts),
    ("browser_login", Gate::Acts),
    // Pressing a button in another program's window.
    ("window_action", Gate::Acts),
    // Other hosts, devices and parties' tools.
    ("ssh_run", Gate::Acts),
    ("ha_call_service", Gate::Acts),
    ("claude_code", Gate::Acts),
    ("?McpProxyTool", Gate::Acts),
    // Code and commands that can send anything anywhere unless the sandbox is strict.
    ("run_background", Gate::Contained),
    ("code_execute", Gate::Contained),
    ("script_run", Gate::Contained),
    // Reads aimed at an address (or a search) the model chose: asked about once the conversation
    // holds something of the person's, because the address can carry it.
    ("browse", Gate::Reads),
    ("open_url", Gate::Reads),
    ("launch_browser", Gate::Reads),
    ("browser_search", Gate::Reads),
    ("web_search", Gate::Reads),
    ("web_fetch", Gate::Reads),
    ("http_fetch", Gate::Reads),
    ("download_file", Gate::Reads),
    ("git_clone", Gate::Reads),
    // An app's own actions: the app and the shell decide.
    ("app_action", Gate::App),
];

/// Every other tool, with why it is not outward: it stays on this machine, only reads, or is a
/// question whose outward steps are other tools in this registry.
const NOT_OUTWARD: &[(&str, &[&str])] = &[
    (
        "reads this machine's disks",
        &["analyze_disk", "dir_size", "disk_usage", "mount_info"],
    ),
    (
        "looks at this screen",
        &["analyze_screen", "describe_image"],
    ),
    (
        "scans this machine",
        &["antivirus_quarantine", "antivirus_scan", "antivirus_status", "antivirus_update"],
    ),
    (
        "saves or restores window layouts on this machine",
        &["apply_workspace_template", "list_workspace_templates", "recall_workspace", "save_workspace", "save_workspace_template"],
    ),
    (
        "archives on this machine",
        &["archive_create", "archive_extract"],
    ),
    (
        "this machine's audio and screenshots",
        &["audio_control", "audio_info", "screenshot"],
    ),
    (
        "reads what apps show (an app's own actions are decided by the app and the shell)",
        &["await_app", "describe_app", "describe_window", "list_apps", "list_readable_windows"],
    ),
    (
        "pure computation",
        &["base64_decode", "base64_encode", "calculate", "json_format", "unit_convert", "url_encode"],
    ),
    (
        "this machine's own settings and desktop notifications",
        &["battery_forecast", "system_control"],
    ),
    (
        "this machine's Bluetooth devices",
        &["bluetooth_connect", "bluetooth_disconnect", "bluetooth_info", "bluetooth_pair", "bluetooth_scan"],
    ),
    (
        "the companion's own memory: nothing leaves the machine",
        &["brain_status", "check_bond", "create_inside_joke", "forget_memory", "forget_topic", "form_opinion", "introspect", "memory_stats", "purge_system_noise", "recall", "relate_entities", "remember", "resolve_conflicts", "review_memories", "save_user_fact", "set_reminder", "update_memory"],
    ),
    (
        "reads or navigates (a GET); the pressing and submitting tools are listed as outward",
        &["browser_cleanup", "browser_read", "browser_screenshot", "browser_scroll", "browser_see", "browser_snapshot", "browser_status", "browser_tabs"],
    ),
    (
        "the local calendar service; no invitation leaves the machine",
        &["calendar_create_event", "calendar_delete_event", "calendar_list_events", "calendar_today", "calendar_update_event"],
    ),
    (
        "schedules a later run (each outward act in it is gated when it runs)",
        &["cancel_schedule", "create_schedule", "list_schedules", "update_schedule"],
    ),
    (
        "the companion's own task queue (what it runs is gated when it runs)",
        &["cancel_task", "complete_task", "list_tasks", "queue_task", "update_task"],
    ),
    (
        "local background commands on this machine",
        &["check_background_task", "list_background_tasks", "stop_background_task"],
    ),
    (
        "an answer from the person's own Claude sign-in, run with every tool and MCP server disabled in an empty directory (`claude_think`)",
        &["claude_think"],
    ),
    (
        "this machine's clipboard (fetching a URL is a GET)",
        &["clipboard_analyze", "clipboard_fetch_url", "clipboard_history", "clipboard_transform", "text_action"],
    ),
    (
        "this desktop's window manager",
        &["close_window", "focus_context", "focus_window", "list_windows"],
    ),
    (
        "connects an account in the person's own browser consent; syncing only reads",
        &["connect_service", "disconnect_service", "list_connections", "sync_service"],
    ),
    (
        "runs recipes made of other tools (each outward tool in them is gated itself)",
        &["create_automation", "create_recipe", "delete_automation", "find_recipe", "list_automations", "list_recipes", "run_automation", "run_recipe", "toggle_automation"],
    ),
    (
        "local clock and timers",
        &["date_calc", "timer"],
    ),
    (
        "reads and ranks what other tools found; sends nothing",
        &["deduplicate_results", "extract_search_results", "life_search", "rank_results", "recall_preferences", "search_sources"],
    ),
    (
        "reads or sets the companion's own notion of the active project",
        &["detect_projects", "get_project_context", "set_active_project"],
    ),
    (
        "reads this desktop's terminal",
        &["detect_terminal_errors", "explain_last_error", "read_terminal_buffer", "search_terminal_history"],
    ),
    (
        "this machine's processes",
        &["diagnose_process", "kill_process", "list_processes", "system_info"],
    ),
    (
        "reads or compares local text",
        &["diff_files", "hash_file", "word_count"],
    ),
    (
        "lists tools",
        &["discover_tools"],
    ),
    (
        "this machine's display",
        &["display_info", "set_resolution"],
    ),
    (
        "a command inside a container on this machine",
        &["docker_exec"],
    ),
    (
        "containers on this machine",
        &["docker_images", "docker_logs", "docker_ps", "docker_start", "docker_stop"],
    ),
    (
        "diagrams on this machine",
        &["edit_diagram", "generate_diagram"],
    ),
    (
        "files on this machine",
        &["edit_file", "file_info", "glob", "grep", "manage_files", "search_files", "write_file"],
    ),
    (
        "reads or changes mail on the server; sending is listed as outward",
        &["email_archive", "email_check", "email_delete", "email_flag", "email_list", "email_list_folders", "email_mark_read", "email_mark_unread", "email_move", "email_read", "email_search", "email_unflag"],
    ),
    (
        "this machine's own firewall, read-only",
        &["firewall_list_rules", "firewall_status"],
    ),
    (
        "the companion's own saved fixes on this machine",
        &["generate_fix_summary", "list_fixes", "read_fix"],
    ),
    (
        "reads a forecast (GET)",
        &["get_weather"],
    ),
    (
        "repositories on this machine (clone fetches, nothing is pushed)",
        &["git_branch", "git_commit", "git_diff", "git_diff_file", "git_log", "git_show", "git_stash", "git_status"],
    ),
    (
        "reads public profile data (GET)",
        &["github_profile", "github_repos", "github_stars"],
    ),
    (
        "reads state only",
        &["ha_get_state", "ha_list_entities"],
    ),
    (
        "this desktop: clipboard, files, commands and the browser opened for the person to look at",
        &["list_files", "read_clipboard", "read_file", "write_clipboard"],
    ),
    (
        "this machine's own network settings and probes",
        &["network_diagnose", "network_dns", "network_dns_set", "network_interfaces", "network_ping", "network_ports", "network_traceroute", "network_vpn_status"],
    ),
    (
        "packages on this machine",
        &["package_info", "package_install", "package_list", "package_remove", "package_search"],
    ),
    (
        "the companion's own open loops: nothing leaves the machine",
        &["resolve_loop", "show_open_loops", "snooze_loop"],
    ),
    (
        "a command on this machine",
        &["run_command"],
    ),
    (
        "scripts on this machine",
        &["script_delete", "script_list", "script_patch", "script_read", "script_write"],
    ),
    (
        "reads the companion's own history; sends nothing",
        &["search_by_timeframe", "search_fix_history", "summarize_work_session"],
    ),
    (
        "a notification on this desktop",
        &["send_notification"],
    ),
    (
        "this machine's own services",
        &["service_control", "service_list", "service_status"],
    ),
    (
        "this machine's wallpaper",
        &["set_wallpaper"],
    ),
    (
        "starts sub-agents whose tools are gated themselves",
        &["spawn_agents"],
    ),
    (
        "reads this machine's ssh configuration",
        &["ssh_check_host", "ssh_list_hosts"],
    ),
    (
        "the local credential vault: reads and writes on this machine",
        &["vault_delete", "vault_generate_password", "vault_get", "vault_list", "vault_set_pin", "vault_store"],
    ),
    (
        "this machine's own Wi-Fi",
        &["wifi_connect", "wifi_disconnect", "wifi_radio", "wifi_scan", "wifi_status"],
    ),
    (
        "a plugin the person installed and wrote the command for; it runs on this machine",
        &["?PluginTool"],
    ),
    (
        "renames files on this machine",
        &["smart_rename"],
    ),
];

/// Where the companion's tools are defined: every `yantrik-companion*` crate's `src`.
fn source_files() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the crates directory");
    let mut found = Vec::new();
    let mut stack: Vec<PathBuf> = std::fs::read_dir(crates)
        .expect("read crates")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("yantrik-companion")))
        .map(|p| p.join("src"))
        .collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// One tool as the sources define it: its name (`?Type` when the name is not a literal) and its
/// implementation, from `impl Tool for` to the next one.
struct Defined {
    name: String,
    body: String,
    file: PathBuf,
}

fn defined_tools() -> Vec<Defined> {
    let mut out = Vec::new();
    for file in source_files() {
        if file.ends_with("tools/outward_pin.rs") {
            continue; // this file mentions the pattern it looks for
        }
        // The whole file, test modules included: a tool defined below a `#[cfg(test)]` module is
        // still a tool (`browser_login` is), and a test double is named and skipped below.
        let production = std::fs::read_to_string(&file).expect("read source").replace("\r\n", "\n");
        let production = production.as_str();
        let starts: Vec<usize> = production.match_indices("impl Tool for ").map(|(i, _)| i).collect();
        for (n, &start) in starts.iter().enumerate() {
            let end = starts.get(n + 1).copied().unwrap_or(production.len());
            // From `impl Tool for` to the closing brace in column 0: this tool's own code and no other.
            let body = &production[start..end];
            let body = &body[..body.find("\n}\n").map(|i| i + 3).unwrap_or(body.len())];
            let ty = body["impl Tool for ".len()..].split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("");
            let name = body
                .split("fn name(&self) -> &'static str")
                .nth(1)
                .and_then(|rest| rest.split_once('{'))
                .and_then(|(_, rest)| {
                    let rest = rest.trim_start();
                    rest.strip_prefix('"').and_then(|r| r.split('"').next()).map(str::to_string)
                })
                .unwrap_or_else(|| format!("?{ty}"));
            out.push(Defined { name, body: body.to_string(), file: file.clone() });
        }
    }
    out
}

/// The code without its line comments: a gate named in a comment is not a gate.
fn without_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(at) if !line[..at].ends_with(':') => &line[..at],
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_listed_outward(name: &str) -> bool {
    OUTWARD.iter().any(|(n, _)| *n == name)
}

fn is_listed_not_outward(name: &str) -> bool {
    NOT_OUTWARD.iter().any(|(_, names)| names.contains(&name))
}

#[test]
fn every_tool_is_either_outward_and_gated_or_listed_as_staying_here() {
    let defined = defined_tools();
    assert!(defined.len() > 200, "the scan stopped finding the companion's tools: {}", defined.len());
    let mut unlisted = Vec::new();
    for tool in &defined {
        if tool.name == "?Fake" {
            continue; // a test double in companion-core's registry tests
        }
        let (out, not) = (is_listed_outward(&tool.name), is_listed_not_outward(&tool.name));
        assert!(!(out && not), "`{}` is listed as both outward and not outward", tool.name);
        if !out && !not {
            unlisted.push(format!("{} ({})", tool.name, tool.file.display()));
        }
    }
    assert!(
        unlisted.is_empty(),
        "a tool nobody has classified. If it sends, posts, submits, purchases, messages, runs something on another \
         host, or calls another party's tool with the person's identity, it must ask the person first \
         (`yantrik_companion_tools::outward::acting`) and go in OUTWARD; if it only reads or stays on this machine, \
         go in NOT_OUTWARD with the reason:\n  {}",
        unlisted.join("\n  ")
    );
}

#[test]
fn nothing_is_listed_that_no_source_defines() {
    let names: Vec<String> = defined_tools().into_iter().map(|t| t.name).collect();
    for (name, _) in OUTWARD {
        assert!(names.iter().any(|n| n == name), "OUTWARD lists `{name}`, which no source defines");
    }
    for (_, listed) in NOT_OUTWARD {
        for name in *listed {
            assert!(names.iter().any(|n| n == name), "NOT_OUTWARD lists `{name}`, which no source defines");
        }
    }
}

#[test]
fn every_outward_tool_calls_its_gate() {
    let defined = defined_tools();
    for (name, gate) in OUTWARD {
        let tool = defined.iter().find(|t| t.name == *name).unwrap_or_else(|| panic!("`{name}` is not defined"));
        let needles: &[&str] = match gate {
            Gate::Acts => &["outward::acting(", "ask_person_before_acting(", "ask_before_press(", "ask_before_typing("],
            Gate::Contained => &["outward::acting_unless("],
            Gate::Reads => &["outward::reading("],
            Gate::Mail => &["smtp_client::send_email(", "smtp_client::send_email_to_each("],
            Gate::App => &["act_asking("],
        };
        assert!(
            needles.iter().any(|n| without_comments(&tool.body).contains(n)),
            "`{name}` acts outward and does not call its gate ({gate:?}: one of {needles:?}) in {}",
            tool.file.display()
        );
    }
}

/// Every browser tool that can act asks before it makes the act, on every path (#1063, H1:
/// `browser_type_xy` once clicked wherever it was pointed whenever `submit` was false).
#[test]
fn the_browser_tools_ask_before_the_act_not_after() {
    let defined = defined_tools();
    for name in ["browser_click", "browser_click_element", "browser_click_xy", "browser_type", "browser_type_element", "browser_type_xy"] {
        let tool = defined.iter().find(|t| t.name == name).unwrap();
        let code = without_comments(&tool.body);
        let asked = ["ask_before_press(", "ask_before_typing("]
            .iter()
            .filter_map(|n| code.find(n))
            .min()
            .unwrap_or_else(|| panic!("{name} does not ask"));
        // The first thing in the tool that touches the page's controls other than to describe them.
        let acted = ["pressing.press(", "type_and_submit(", "Input.dispatchMouseEvent", "Input.insertText", "eval_js(&mut ws, &js)"]
            .iter()
            .filter_map(|n| code.find(n))
            .min()
            .expect("it acts");
        assert!(asked < acted, "{name} acts before it asks");
        // The tools that click a point or an element ask on every path, not only when `submit` is
        // set: typing at a point clicks it first.
        if name == "browser_type_xy" || name.starts_with("browser_click") {
            assert!(!code[asked.saturating_sub(120)..asked].contains("if submit"), "{name}: the ask depends on `submit`");
        }
    }
}

/// The mail tools' gate is one function, and the only one that speaks SMTP.
#[test]
fn mail_is_sent_from_one_place_and_that_place_asks_first() {
    let smtp = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/email/smtp_client.rs")).expect("smtp_client").replace("\r\n", "\n");
    let production = smtp.split("#[cfg(test)]").next().unwrap();
    for name in ["pub fn send_email(", "pub fn send_email_to_each("] {
        let body = production.split(name).nth(1).unwrap_or_else(|| panic!("{name}"));
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.find("ask(").expect("it asks") < body.find("deliver(").expect("it delivers"), "{name}");
    }
}
