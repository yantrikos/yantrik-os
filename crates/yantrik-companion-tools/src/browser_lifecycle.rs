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

/// Count chromium processes using `pgrep`.
/// Returns (total_count, renderer_count, gpu_count, main_count).
fn count_chromium_processes() -> (u64, u64, u64, u64) {
    // Total chromium processes
    let total = Command::new("pgrep")
        .args(["-c", "chromium"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok()
            } else {
                Some(0)
            }
        })
        .unwrap_or(0);

    // Count by type using pgrep + grep on /proc/PID/cmdline
    let renderer = Command::new("sh")
        .args(["-c", "pgrep -a chromium 2>/dev/null | grep -c '\\-\\-type=renderer' || echo 0"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .unwrap_or(0);

    let gpu = Command::new("sh")
        .args(["-c", "pgrep -a chromium 2>/dev/null | grep -c '\\-\\-type=gpu' || echo 0"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .unwrap_or(0);

    let main = total.saturating_sub(renderer).saturating_sub(gpu);

    (total, renderer, gpu, main)
}

/// Get total memory usage of all chromium processes in MB.
/// Reads VmRSS from /proc/PID/status for each chromium PID.
fn chromium_memory_mb() -> u64 {
    let output = Command::new("sh")
        .args(["-c", r#"
            total=0
            for pid in $(pgrep chromium 2>/dev/null); do
                rss=$(grep '^VmRSS:' /proc/$pid/status 2>/dev/null | awk '{print $2}')
                if [ -n "$rss" ]; then
                    total=$((total + rss))
                fi
            done
            echo $((total / 1024))
        "#])
        .output();

    output
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Get per-process memory info for chromium processes.
/// Returns Vec of (pid, type, rss_mb).
fn chromium_process_details() -> Vec<(u32, String, u64)> {
    let output = Command::new("sh")
        .args(["-c", r#"
            for pid in $(pgrep chromium 2>/dev/null); do
                cmdline=$(cat /proc/$pid/cmdline 2>/dev/null | tr '\0' ' ')
                rss=$(grep '^VmRSS:' /proc/$pid/status 2>/dev/null | awk '{print $2}')
                if [ -n "$rss" ]; then
                    rss_mb=$((rss / 1024))
                    if echo "$cmdline" | grep -q '\-\-type=renderer'; then
                        echo "$pid renderer $rss_mb"
                    elif echo "$cmdline" | grep -q '\-\-type=gpu'; then
                        echo "$pid gpu $rss_mb"
                    elif echo "$cmdline" | grep -q '\-\-type=zygote'; then
                        echo "$pid zygote $rss_mb"
                    elif echo "$cmdline" | grep -q '\-\-type=utility'; then
                        echo "$pid utility $rss_mb"
                    else
                        echo "$pid main $rss_mb"
                    fi
                fi
            done
        "#])
        .output();

    let mut procs = Vec::new();
    if let Ok(o) = output {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let (Ok(pid), Ok(rss)) = (parts[0].parse::<u32>(), parts[2].parse::<u64>()) {
                    procs.push((pid, parts[1].to_string(), rss));
                }
            }
        }
    }
    procs
}

/// Kill specific PIDs via `kill`.
fn kill_pids(pids: &[u32], signal: &str) -> u32 {
    let mut killed = 0;
    for pid in pids {
        let result = Command::new("kill")
            .args([signal, &pid.to_string()])
            .output();
        if result.map(|o| o.status.success()).unwrap_or(false) {
            killed += 1;
        }
    }
    killed
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

/// Fetch open tabs from Chromium CDP /json endpoint (mirrors browser.rs get_tabs).
fn get_cdp_tabs() -> Result<Vec<TabSession>, String> {
    let url = "http://127.0.0.1:9222/json";
    let output = Command::new("curl")
        .args(["-s", "--max-time", "3", &url])
        .output()
        .map_err(|e| format!("Cannot reach browser (curl): {e}"))?;

    if !output.status.success() {
        return Err("Browser not reachable on port 9222.".to_string());
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let tabs: Vec<Value> = serde_json::from_str(&body)
        .map_err(|e| format!("Bad CDP response: {e}"))?;

    let now = now_secs();
    Ok(tabs
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

/// Close a CDP tab by its target ID.
fn close_cdp_tab(tab_id: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:9222/json/close/{}", tab_id);
    let output = Command::new("curl")
        .args(["-s", "--max-time", "3", &url])
        .output()
        .map_err(|e| format!("Failed to close tab: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err("Close request failed".to_string())
    }
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

/// Kill ALL browser processes. Nuclear option for full cleanup.
///
/// Sends SIGTERM first, waits 2 seconds, then SIGKILL any survivors.
/// Returns a summary string with counts.
pub fn kill_all_browsers() -> String {
    // Count before
    let (before, _, _, _) = count_chromium_processes();
    if before == 0 {
        return "No Chromium processes running.".to_string();
    }

    // SIGTERM all chromium
    let _ = Command::new("pkill")
        .args(["-15", "chromium"])
        .output();

    // Wait 2 seconds for graceful shutdown
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Check survivors
    let (survivors, _, _, _) = count_chromium_processes();
    if survivors > 0 {
        // SIGKILL survivors
        let _ = Command::new("pkill")
            .args(["-9", "chromium"])
            .output();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let (after, _, _, _) = count_chromium_processes();
    let killed = before.saturating_sub(after);

    format!(
        "Killed {killed} Chromium processes (was: {before}, now: {after}).{}",
        if survivors > 0 { " Force-killed survivors." } else { "" }
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
