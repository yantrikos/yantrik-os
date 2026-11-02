//! Browser Lifecycle Management — session tracking, zombie cleanup, and process watchdog.
//!
//! Solves the "19 zombie Chromium processes eating all RAM" problem on 4GB VMs.
//! Provides tools for cleanup and status reporting, plus standalone watchdog
//! functions callable from launch_browser and the companion main loop.
//!
//! 2 tools: browser_cleanup, browser_status
//! 2 public functions: watchdog_check(), kill_all_browsers()

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use yantrik_browser_ports::procs;

use super::browser_side;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

/// Default limits — tuned for a 4GB Alpine VM running Yantrik OS.
const DEFAULT_MAX_TABS: usize = 3;
const DEFAULT_TAB_TIMEOUT_SECS: u64 = 300; // 5 minutes
const DEFAULT_MEMORY_LIMIT_MB: u64 = 500;
const MAX_CHROMIUM_PROCESSES: u64 = 10;

// ── Session Manager (in-module state for tracking) ──

/// A tracked browser tab session.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TabSession {
    tab_id: String,
    url: String,
    opened_at: f64,
    last_active: f64,
    task_id: Option<String>,
}

/// Browser session manager — tracks tabs, enforces limits, manages lifecycle.
///
/// NOTE: This is used internally by the tools in this module. Because tools
/// are stateless (trait objects), the manager functions operate on live CDP
/// state rather than maintaining a persistent in-memory registry.
#[allow(dead_code)]
pub struct BrowserSessionManager {
    /// Max concurrent tabs (configurable, default 3)
    pub max_tabs: usize,
    /// Tab idle timeout in seconds (default 300 = 5 min)
    pub tab_timeout_secs: u64,
    /// Total browser memory limit in MB (default 500)
    pub memory_limit_mb: u64,
}

impl Default for BrowserSessionManager {
    fn default() -> Self {
        Self {
            max_tabs: DEFAULT_MAX_TABS,
            tab_timeout_secs: DEFAULT_TAB_TIMEOUT_SECS,
            memory_limit_mb: DEFAULT_MEMORY_LIMIT_MB,
        }
    }
}

// ── Helper functions ──

/// Get current unix timestamp as f64.
fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// A process of a mind's browser: what kind it is and what it holds.
struct Mine {
    pid: u32,
    kind: String,
    rss_mb: u64,
}

/// `--type=renderer` and the like out of a command line, `main` for the browser process itself.
fn kind_of(cmdline: &[u8]) -> String {
    let text = String::from_utf8_lossy(cmdline);
    text.split(['\0', ' '])
        .find_map(|t| t.strip_prefix("--type="))
        .map(|t| t.trim_end_matches("-process").to_string())
        .unwrap_or_else(|| "main".to_string())
}

fn rss_mb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("VmRSS:").and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())))
        .map_or(0, |kb| kb / 1024)
}

/// The processes of the browsers that are a mind's (Mind View's, and the companion's
/// identities), run by this account. The person's Browser is a Chromium too, and these used to
/// be `pgrep chromium`: its renderers were counted, trimmed when there were "too many" (an
/// ordinary browser has more than ten processes), and `browser_cleanup force` killed it by name
/// (#1085).
fn mine() -> Vec<Mine> {
    procs::mind_side_family()
        .into_iter()
        .map(|p| {
            // Only the kind of helper (`--type=renderer`), never whose it is: that was decided by the profile.
            let line = std::fs::read(format!("/proc/{}/cmdline", p.pid)).unwrap_or_default();
            Mine { pid: p.pid, kind: kind_of(&line), rss_mb: rss_mb(p.pid) }
        })
        .collect()
}

/// Returns (total_count, renderer_count, gpu_count, main_count) of a mind's browser processes.
fn count_chromium_processes() -> (u64, u64, u64, u64) {
    let all = mine();
    let of = |kind: &str| all.iter().filter(|m| m.kind == kind).count() as u64;
    let (renderer, gpu) = (of("renderer"), of("gpu"));
    let total = all.len() as u64;
    (total, renderer, gpu, total.saturating_sub(renderer).saturating_sub(gpu))
}

/// Total memory usage of a mind's browser processes in MB.
fn chromium_memory_mb() -> u64 {
    mine().iter().map(|m| m.rss_mb).sum()
}

/// Per-process memory info for a mind's browser processes: (pid, type, rss_mb).
fn chromium_process_details() -> Vec<(u32, String, u64)> {
    mine().into_iter().map(|m| (m.pid, m.kind, m.rss_mb)).collect()
}

/// Kill specific PIDs via `kill`.
fn kill_pids(pids: &[u32], signal: &str) -> u32 {
    procs::signal(pids, signal) as u32
}

/// Kill renderer processes that exceed the limit, keeping the newest ones.
/// Preserves main and gpu processes.
fn kill_excess_renderers(max_renderers: usize) -> (u32, u64) {
    let procs = chromium_process_details();

    // Collect renderer PIDs sorted by PID (higher PID = newer, roughly)
    let mut renderers: Vec<(u32, u64)> = procs
        .iter()
        .filter(|(_, ptype, _)| ptype == "renderer")
        .map(|(pid, _, rss)| (*pid, *rss))
        .collect();

    if renderers.len() <= max_renderers {
        return (0, 0);
    }

    // Sort by PID ascending — kill oldest (lowest PIDs) first
    renderers.sort_by_key(|(pid, _)| *pid);

    let to_kill = renderers.len() - max_renderers;
    let kill_list: Vec<u32> = renderers.iter().take(to_kill).map(|(pid, _)| *pid).collect();
    let freed_mb: u64 = renderers.iter().take(to_kill).map(|(_, rss)| *rss).sum();

    let killed = kill_pids(&kill_list, "-15"); // SIGTERM

    // Give them a moment then force-kill any survivors
    if killed > 0 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        // Check which are still alive and SIGKILL them
        for pid in &kill_list {
            let still_alive = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if still_alive {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
            }
        }
    }

    (killed, freed_mb)
}

/// Mind View's browser's open tabs, once its port is shown to be Mind View's (browser_side).
fn get_cdp_tabs() -> Result<Vec<TabSession>, String> {
    let now = now_secs();
    Ok(browser_side::targets()?
        .iter()
        .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .map(|t| TabSession {
            tab_id: t["id"].as_str().unwrap_or_default().to_string(),
            url: t["url"].as_str().unwrap_or_default().to_string(),
            opened_at: now, // We don't know the real open time from CDP
            last_active: now,
            task_id: None,
        })
        .collect())
}

/// Close one of Mind View's browser's tabs by its target ID.
fn close_cdp_tab(tab_id: &str) -> Result<(), String> {
    browser_side::close_target(tab_id)
}

// ── Public watchdog functions (not tools — called from companion/browser.rs) ──

/// Check browser health and kill zombies if needed.
///
/// Called from `launch_browser` before spawning a new Chromium process,
/// and can be called periodically from the companion main loop.
///
/// Returns `Some(warning_message)` if corrective action was taken,
/// `None` if everything is healthy.
pub fn watchdog_check() -> Option<String> {
    let (total, renderer, _gpu, _main) = count_chromium_processes();

    // Nothing running = healthy
    if total == 0 {
        return None;
    }

    let mut actions = Vec::new();

    // Check 1: Too many total chromium processes
    if total > MAX_CHROMIUM_PROCESSES {
        let max_renderers = (DEFAULT_MAX_TABS + 1).min(renderer as usize);
        let (killed, freed) = kill_excess_renderers(max_renderers);
        if killed > 0 {
            actions.push(format!(
                "Killed {killed} excess renderer processes (freed ~{freed} MB)"
            ));
        }
    }

    // Check 2: Memory limit exceeded
    let mem_mb = chromium_memory_mb();
    if mem_mb > DEFAULT_MEMORY_LIMIT_MB {
        // Kill oldest renderers to get under limit
        let procs = chromium_process_details();
        let mut renderers: Vec<(u32, u64)> = procs
            .iter()
            .filter(|(_, ptype, _)| ptype == "renderer")
            .map(|(pid, _, rss)| (*pid, *rss))
            .collect();

        // Sort oldest first (lowest PID)
        renderers.sort_by_key(|(pid, _)| *pid);

        let mut freed_total: u64 = 0;
        let mut kill_list = Vec::new();
        let target = mem_mb.saturating_sub(DEFAULT_MEMORY_LIMIT_MB);

        for (pid, rss) in &renderers {
            if freed_total >= target {
                break;
            }
            // Keep at least 1 renderer
            if kill_list.len() + 1 >= renderers.len() {
                break;
            }
            kill_list.push(*pid);
            freed_total += rss;
        }

        if !kill_list.is_empty() {
            let killed = kill_pids(&kill_list, "-15");
            std::thread::sleep(std::time::Duration::from_millis(500));
            // Force-kill survivors
            for pid in &kill_list {
                let alive = Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if alive {
                    let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
                }
            }
            actions.push(format!(
                "Memory at {mem_mb} MB (limit {DEFAULT_MEMORY_LIMIT_MB} MB): killed {killed} renderers, freed ~{freed_total} MB"
            ));
        }
    }

    // Check 3: Close excess CDP tabs (keep max_tabs newest)
    if let Ok(tabs) = get_cdp_tabs() {
        if tabs.len() > DEFAULT_MAX_TABS {
            let excess = tabs.len() - DEFAULT_MAX_TABS;
            // Close oldest tabs (first in list from CDP tend to be oldest)
            let mut closed = 0;
            for tab in tabs.iter().take(excess) {
                if close_cdp_tab(&tab.tab_id).is_ok() {
                    closed += 1;
                }
            }
            if closed > 0 {
                actions.push(format!("Closed {closed} excess tabs (limit: {DEFAULT_MAX_TABS})"));
            }
        }
    }

    if actions.is_empty() {
        None
    } else {
        Some(format!("Watchdog: {}", actions.join("; ")))
    }
}

/// Kill all of a mind's browser processes. Nuclear option for full cleanup. Never the person's
/// Browser: only browsers on Mind View's profile or the companion's identities (#1085).
///
/// Sends SIGTERM first, waits 2 seconds, then SIGKILL any survivors.
/// Returns a summary string with counts.
pub fn kill_all_browsers() -> String {
    let pids: Vec<u32> = mine().iter().map(|m| m.pid).collect();
    if pids.is_empty() {
        return "No Chromium processes running.".to_string();
    }
    kill_pids(&pids, "-15");
    std::thread::sleep(std::time::Duration::from_secs(2));
    let survivors: Vec<u32> = mine().iter().map(|m| m.pid).filter(|p| pids.contains(p)).collect();
    if !survivors.is_empty() {
        kill_pids(&survivors, "-9");
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let after = mine().iter().filter(|m| pids.contains(&m.pid)).count();
    format!(
        "Killed {} Chromium processes (was: {}, now: {after}).{}",
        pids.len().saturating_sub(after),
        pids.len(),
        if survivors.is_empty() { "" } else { " Force-killed survivors." }
    )
}

// ── Tool: browser_cleanup ──

pub struct BrowserCleanupTool;

impl Tool for BrowserCleanupTool {
    fn name(&self) -> &'static str { "browser_cleanup" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_cleanup",
                "description": "Close all browser tabs; reset browser state",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "force": {
                            "type": "boolean",
                            "description": "Kill ALL Chromium processes (nuclear option). Default: false (gentle cleanup)"
                        },
                        "keep_tab": {
                            "type": "string",
                            "description": "Tab URL pattern to keep open (others get closed). Optional."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let keep_pattern = args.get("keep_tab").and_then(|v| v.as_str()).unwrap_or("");

        if force {
            return kill_all_browsers();
        }

        let mut report = Vec::new();

        // 1. Close excess tabs (keep newest or matching pattern)
        match get_cdp_tabs() {
            Ok(tabs) => {
                let tab_count = tabs.len();
                if tab_count > 1 {
                    let mut closed = 0;
                    for (i, tab) in tabs.iter().enumerate() {
                        // Keep the last tab (newest / active) always
                        if i == tab_count - 1 {
                            continue;
                        }
                        // Keep tabs matching the pattern
                        if !keep_pattern.is_empty() && tab.url.contains(keep_pattern) {
                            continue;
                        }
                        // Keep about:blank (it's the default)
                        if tab.url == "about:blank" && closed > 0 {
                            continue;
                        }
                        if close_cdp_tab(&tab.tab_id).is_ok() {
                            closed += 1;
                        }
                    }
                    if closed > 0 {
                        report.push(format!("Closed {closed}/{tab_count} tabs"));
                    } else {
                        report.push(format!("{tab_count} tab(s) open, none closed"));
                    }
                } else {
                    report.push(format!("{tab_count} tab(s) open"));
                }
            }
            Err(_) => {
                report.push("Browser not reachable (CDP offline)".to_string());
            }
        }

        // 2. Kill zombie renderer processes
        let (total, renderer, gpu, main) = count_chromium_processes();
        let mem_before = chromium_memory_mb();

        if renderer > DEFAULT_MAX_TABS as u64 + 1 {
            let max_keep = DEFAULT_MAX_TABS + 1; // 1 extra for the active tab
            let (killed, freed) = kill_excess_renderers(max_keep);
            if killed > 0 {
                report.push(format!(
                    "Killed {killed} zombie renderers (freed ~{freed} MB)"
                ));
            }
        }

        // 3. Report final state
        let mem_after = chromium_memory_mb();
        let freed = mem_before.saturating_sub(mem_after);

        report.push(format!(
            "Chromium: {total} processes (main:{main} gpu:{gpu} renderer:{renderer}), {mem_after} MB RAM"
        ));

        if freed > 0 {
            report.push(format!("Total freed: ~{freed} MB"));
        }

        report.join("\n")
    }
}

// ── Tool: browser_status ──

pub struct BrowserStatusTool;

impl Tool for BrowserStatusTool {
    fn name(&self) -> &'static str { "browser_status" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "browser" }

    fn definition(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "browser_status",
                "description": "Check browser health and open-tab count",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &Value) -> String {
        let mut out = String::from("=== Browser Status ===\n\n");

        // Tab info from CDP
        match get_cdp_tabs() {
            Ok(tabs) => {
                out.push_str(&format!("Tabs: {} open\n", tabs.len()));
                for (i, tab) in tabs.iter().enumerate() {
                    let url_display = if tab.url.len() > 80 {
                        // Safe truncation: URLs are ASCII so this is fine,
                        // but use char_indices for safety
                        let end = tab.url.char_indices()
                            .nth(77)
                            .map(|(i, _)| i)
                            .unwrap_or(tab.url.len());
                        format!("{}...", &tab.url[..end])
                    } else {
                        tab.url.clone()
                    };
                    out.push_str(&format!("  {}. {}\n", i + 1, url_display));
                }
            }
            Err(_) => {
                out.push_str("Tabs: Browser not reachable (CDP offline)\n");
            }
        }

        out.push('\n');

        // Process info
        let (total, renderer, gpu, main) = count_chromium_processes();
        out.push_str(&format!("Processes: {total} total\n"));
        out.push_str(&format!("  Main:     {main}\n"));
        out.push_str(&format!("  GPU:      {gpu}\n"));
        out.push_str(&format!("  Renderer: {renderer}\n"));

        // Detailed per-process breakdown
        let procs = chromium_process_details();
        if !procs.is_empty() {
            out.push_str("\n  PID     Type       RSS\n");
            for (pid, ptype, rss) in &procs {
                out.push_str(&format!("  {:<7} {:<10} {} MB\n", pid, ptype, rss));
            }
        }

        out.push('\n');

        // Memory
        let mem_mb = chromium_memory_mb();
        out.push_str(&format!("Memory: {} MB", mem_mb));
        if mem_mb > DEFAULT_MEMORY_LIMIT_MB {
            out.push_str(&format!(" [OVER LIMIT: {} MB]", DEFAULT_MEMORY_LIMIT_MB));
        } else if mem_mb > DEFAULT_MEMORY_LIMIT_MB * 80 / 100 {
            out.push_str(" [WARNING: approaching limit]");
        }
        out.push('\n');

        // Health assessment
        out.push_str("\nHealth: ");
        let mut issues = Vec::new();
        if total > MAX_CHROMIUM_PROCESSES {
            issues.push(format!("too many processes ({total} > {MAX_CHROMIUM_PROCESSES})"));
        }
        if mem_mb > DEFAULT_MEMORY_LIMIT_MB {
            issues.push(format!("memory over limit ({mem_mb} > {DEFAULT_MEMORY_LIMIT_MB} MB)"));
        }
        if renderer > DEFAULT_MAX_TABS as u64 + 2 {
            issues.push(format!("zombie renderers ({renderer} for {} tab limit)", DEFAULT_MAX_TABS));
        }

        if issues.is_empty() {
            out.push_str("OK");
        } else {
            out.push_str(&format!("NEEDS CLEANUP - {}", issues.join(", ")));
            out.push_str("\n\nRun browser_cleanup to fix, or browser_cleanup(force=true) for full reset.");
        }

        out.push('\n');
        out
    }
}

// ── Registration ──

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(BrowserCleanupTool));
    reg.register(Box::new(BrowserStatusTool));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_is_told_by_its_type_flag_in_either_command_line_form() {
        assert_eq!(kind_of(b"/usr/lib/chromium/chromium --type=renderer --lang=en"), "renderer");
        assert_eq!(kind_of(b"chromium\0--type=gpu-process\0--x\0"), "gpu");
        assert_eq!(kind_of(b"chromium --remote-debugging-port=9223"), "main");
    }

    /// The watchdog and `browser_cleanup` count, trim and kill a mind's browsers; the person's
    /// Browser is the same program and is not among them (#1085).
    #[test]
    fn this_module_names_no_browser_by_its_program() {
        let src = include_str!("browser_lifecycle.rs");
        let src = &src[..src.find("#[cfg(test)]").unwrap()];
        for banned in ["pgrep", "pkill"] {
            assert!(!src.contains(&format!("\"{banned}\"")), "{banned} names Chromium, whoever's it is");
        }
        assert!(!src.contains("9222"), "the person's port");
    }
}
