//! Intent Lens — query routing, NL→tool matching, action resolution.
//!
//! The Lens is the primary interaction surface. This module handles:
//! - Building result lists from user queries (keyword + NL matching)
//! - Matching natural language to the 70+ tool store tools
//! - Resolving action IDs into concrete actions for main.rs to execute

use slint::SharedString;

use super::LensResult;
use super::apps::DesktopEntry;
use super::clipboard::ClipEntry;

/// Construct a LensResult with sensible defaults for the new fields.
fn lr(
    result_type: &str,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    icon_char: &str,
    action_id: impl Into<SharedString>,
) -> LensResult {
    LensResult {
        result_type: result_type.into(),
        title: title.into(),
        subtitle: subtitle.into(),
        icon_char: icon_char.into(),
        action_id: action_id.into(),
        score: 0.0,
        is_loading: false,
        inline_value: SharedString::default(),
    }
}

/// Construct a divider LensResult.
pub fn lr_divider(title: &str) -> LensResult {
    lr("divider", title, "", "", "")
}

/// Construct an inline answer LensResult.
pub fn answer_result(
    title: &str,
    value: &str,
    icon_char: &str,
    action_id: &str,
) -> LensResult {
    LensResult {
        result_type: "answer".into(),
        title: title.into(),
        subtitle: SharedString::default(),
        icon_char: icon_char.into(),
        action_id: action_id.into(),
        score: 1.0,
        is_loading: false,
        inline_value: value.into(),
    }
}

/// Known apps (fallback when no .desktop files found).
pub const KNOWN_APPS: &[(&str, &str, &str)] = &[
    ("terminal", "foot", "Open terminal emulator"),
    ("browser", "firefox-esr", "Open web browser"),
    ("files", "thunar", "Open file manager"),
];

/// What the main event loop should do when a Lens result is selected.
pub enum LensAction {
    /// Launch an app by command string (may include args).
    Launch(String),
    /// Launch a built-in Yantrik app by app_id (routes via on_launch_app).
    LaunchBuiltin(String),
    /// Open a URL in the default browser.
    OpenUrl(String),
    /// Submit a query to the AI companion (LLM).
    SubmitToAI(String),
    /// Start focus mode with the given duration in seconds.
    StartFocus(u32),
    /// Paste clipboard history entry by index.
    ClipboardPaste(usize),
    /// Lock the screen.
    LockScreen,
    /// Open settings panel (screen 7).
    OpenSettings,
    /// Open file browser (screen 8).
    OpenFileBrowser,
    /// Copy text to the clipboard (used by calculator results).
    CopyToClipboard(String),
    /// Close the Lens, nothing else.
    #[allow(dead_code)]
    CloseLens,
    /// No-op (unknown action).
    Noop,
}

/// Parse an action_id string into a concrete LensAction.
/// `installed_apps` is the scanned .desktop entries for resolving `launch:` by app_id.
pub fn resolve_action(action_id: &str, installed_apps: &[DesktopEntry]) -> LensAction {
    if action_id.starts_with("copy-result:") {
        return LensAction::CopyToClipboard(action_id["copy-result:".len()..].to_string());
    }
    if action_id.starts_with("launch:") {
        let app_id = &action_id[7..];
        // First check installed / built-in apps
        for entry in installed_apps {
            if entry.app_id == app_id {
                if entry.exec == "__builtin__" {
                    return LensAction::LaunchBuiltin(app_id.to_string());
                }
                return LensAction::Launch(entry.exec.clone());
            }
        }
        // Fallback to hardcoded KNOWN_APPS
        for (_id, cmd, _) in KNOWN_APPS {
            if app_id == *_id {
                return LensAction::Launch(cmd.to_string());
            }
        }
        LensAction::Noop
    } else if action_id.starts_with("exec:") {
        // Direct exec command from .desktop entry
        LensAction::Launch(action_id[5..].to_string())
    } else if action_id.starts_with("url:") {
        LensAction::OpenUrl(action_id[4..].to_string())
    } else if action_id.starts_with("tool:") {
        LensAction::SubmitToAI(action_id[5..].to_string())
    } else if action_id.starts_with("clipboard:paste:") {
        let index: usize = action_id
            .strip_prefix("clipboard:paste:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        LensAction::ClipboardPaste(index)
    } else if action_id == "clipboard:read" {
        LensAction::SubmitToAI("What's on my clipboard?".to_string())
    } else if action_id == "system:status" {
        LensAction::SubmitToAI("Show me system status — battery, memory, disk.".to_string())
    } else if action_id.starts_with("files:") {
        LensAction::SubmitToAI(format!("List the files in {}", &action_id[6..]))
    } else if action_id.starts_with("memory:") {
        LensAction::SubmitToAI(action_id[7..].to_string())
    } else if action_id.starts_with("ask:") {
        LensAction::SubmitToAI(action_id[4..].to_string())
    } else if action_id.starts_with("setting:focus:") {
        let secs: u32 = action_id
            .strip_prefix("setting:focus:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(25 * 60);
        LensAction::StartFocus(secs)
    } else if action_id == "setting:lock" {
        LensAction::LockScreen
    } else if action_id == "navigate:settings" {
        LensAction::OpenSettings
    } else if action_id == "navigate:files" {
        LensAction::OpenFileBrowser
    } else {
        LensAction::Noop
    }
}

/// Build the full list of Lens results for a given query.
/// `installed_apps` is the scanned .desktop entries (may be empty on first boot).
/// `clip_history` is the recent clipboard entries (newest first).
/// `companion_online` — when false, AI-dependent results (ask, tool, memory) are hidden.
pub fn build_results(
    query: &str,
    onboarding_step: i32,
    installed_apps: &[DesktopEntry],
    clip_history: &[(usize, ClipEntry)],
    companion_online: bool,
) -> Vec<LensResult> {
    let lower = query.to_lowercase();
    let mut results = Vec::new();

    // During onboarding, prepend guided suggestion
    if onboarding_step > 0 {
        results.push(super::onboarding::guide_result(onboarding_step));
    }

    // App matches from .desktop scanner
    if !installed_apps.is_empty() {
        // Strip "open " prefix for better matching
        let app_query = lower.strip_prefix("open ").unwrap_or(&lower);
        let matches = super::apps::search(app_query, installed_apps);
        for entry in matches {
            results.push(lr("do", SharedString::from(format!("Open {}", entry.name)), SharedString::from(if entry.comment.is_empty() {
                    format!("Launch {}", entry.exec.split_whitespace().next().unwrap_or(&entry.exec))
                } else {
                    entry.comment.clone()
                }), &entry.icon_char, SharedString::from(format!("exec:{}", entry.exec))));
        }
    }

    // Fallback: hardcoded KNOWN_APPS (when no .desktop files available)
    if installed_apps.is_empty() {
        for (app_id, _cmd, desc) in KNOWN_APPS {
            if app_id.contains(&lower) || lower.contains(app_id) || lower.contains("open") {
                results.push(lr("do", SharedString::from(format!("Open {}", capitalize(app_id))), SharedString::from(*desc), "▶", SharedString::from(format!("launch:{}", app_id))));
            }
        }
    }

    // Web search: "search for X", "google X", "look up X"
    let search_prefixes = ["search for ", "search ", "google ", "look up ", "find online "];
    for prefix in &search_prefixes {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if !rest.is_empty() {
                let search_url = format!(
                    "https://duckduckgo.com/?q={}",
                    rest.replace(' ', "+")
                );
                results.push(lr("do", SharedString::from(format!("Search: \"{}\"", rest)), "Open in browser", "🔍", SharedString::from(format!("url:{}", search_url))));
                break;
            }
        }
    }

    // URL: "go to example.com", pasted URLs
    if lower.starts_with("http://") || lower.starts_with("https://")
        || lower.starts_with("go to ")
    {
        let url = if let Some(rest) = lower.strip_prefix("go to ") {
            let rest = rest.trim();
            if rest.contains('.') {
                format!("https://{}", rest)
            } else {
                String::new()
            }
        } else {
            query.to_string()
        };
        if !url.is_empty() {
            results.push(lr("do", SharedString::from(format!("Open {}", &url)), "Open in browser", "🌐", SharedString::from(format!("url:{}", url))));
        }
    }

    // Clipboard: "copy X", "paste", "clipboard", "clipboard history"
    if lower == "paste" || lower == "clipboard" || lower.starts_with("what's on clipboard")
        || lower.starts_with("what did i copy") || lower.contains("clipboard history")
        || lower.contains("paste history") || lower.starts_with("copied")
    {
        results.push(lr("do", "Read clipboard", "Show current clipboard", "📋", "clipboard:read"));

        // Show clipboard history entries
        for &(index, ref entry) in clip_history.iter().take(6) {
            results.push(lr("clipboard", SharedString::from(entry.preview()), SharedString::from(entry.time_ago()), "C", SharedString::from(format!("clipboard:paste:{}", index))));
        }
    }

    // Text actions: "rewrite", "summarize", "translate", "fix grammar", "proofread"
    if lower.starts_with("rewrite") || lower.starts_with("make formal") || lower.starts_with("make casual") {
        if companion_online {
            let style = if lower.contains("casual") { "casually" } else { "formally" };
            results.push(lr("ask", SharedString::from(format!("Rewrite {} clipboard text", style)),
                "AI rewrites text from your clipboard", "R",
                SharedString::from(format!("ask:Read my clipboard with read_clipboard, rewrite the text {}, and put the result back with write_clipboard", style))));
        }
    }
    if lower.starts_with("summarize") || lower.starts_with("summarise") || lower == "tldr" {
        if companion_online {
            results.push(lr("ask", "Summarize clipboard text",
                "AI summarizes text from your clipboard", "S",
                "ask:Read my clipboard with read_clipboard, summarize the text concisely, and put the summary back with write_clipboard"));
        }
    }
    if lower.starts_with("translate") {
        if companion_online {
            let lang = lower.strip_prefix("translate to ").unwrap_or(
                lower.strip_prefix("translate ").unwrap_or("English")
            );
            let lang = capitalize(lang.trim());
            results.push(lr("ask", SharedString::from(format!("Translate to {}", lang)),
                "AI translates clipboard text", "T",
                SharedString::from(format!("ask:Read my clipboard with read_clipboard, translate the text to {}, and put the translation back with write_clipboard", lang))));
        }
    }
    if lower.starts_with("fix grammar") || lower.starts_with("fix spelling")
        || lower.starts_with("proofread") || lower.starts_with("grammar")
    {
        if companion_online {
            results.push(lr("ask", "Fix grammar & spelling",
                "AI proofreads clipboard text", "G",
                "ask:Read my clipboard with read_clipboard, fix all grammar and spelling errors, and put the corrected text back with write_clipboard"));
        }
    }
    if lower.starts_with("explain") && !lower.contains("error") {
        if companion_online {
            results.push(lr("ask", "Explain clipboard text",
                "AI explains the text in your clipboard", "E",
                "ask:Read my clipboard with read_clipboard and explain the text in simple terms"));
        }
    }

    // Scheduling: "remind me...", "every Monday...", "schedule..."
    if lower.starts_with("remind me") || lower.starts_with("remind ") {
        if companion_online {
            results.push(lr("ask", "Set reminder",
                "AI creates a scheduled reminder", "\u{23f0}",
                SharedString::from(format!("ask:{}", query))));
        }
    }
    if lower.starts_with("every ") && (lower.contains("day") || lower.contains("week")
        || lower.contains("monday") || lower.contains("tuesday") || lower.contains("wednesday")
        || lower.contains("thursday") || lower.contains("friday") || lower.contains("saturday")
        || lower.contains("sunday") || lower.contains("hour") || lower.contains("minute")
        || lower.contains("month") || lower.contains("morning") || lower.contains("evening")
        || lower.contains("night") || lower.contains("noon"))
    {
        if companion_online {
            results.push(lr("ask", "Create schedule",
                "AI sets up a recurring schedule", "\u{1f4c5}",
                SharedString::from(format!("ask:Create a schedule: {}", query))));
        }
    }
    if lower.starts_with("schedule ") {
        if companion_online {
            results.push(lr("ask", "Schedule task",
                "AI creates a scheduled task", "\u{1f4c5}",
                SharedString::from(format!("ask:{}", query))));
        }
    }

    // Automation: "automate", "create automation", "run workflow", "automations"
    if lower.starts_with("automate") || lower.starts_with("create automation")
        || lower.starts_with("create workflow") || lower.starts_with("new automation")
    {
        if companion_online {
            results.push(lr("ask", "Create automation",
                "AI creates an automation rule", "\u{26a1}",
                SharedString::from(format!("ask:{}", query))));
        }
    }
    if (lower.starts_with("run ") && (lower.contains("workflow") || lower.contains("automation")))
        || lower.starts_with("execute ")
    {
        if companion_online {
            results.push(lr("ask", "Run automation",
                "AI runs a saved automation", "\u{25b6}",
                SharedString::from(format!("ask:{}", query))));
        }
    }
    if lower == "automations" || lower == "workflows" || lower == "my automations"
        || lower == "list automations"
    {
        if companion_online {
            results.push(lr("ask", "List automations",
                "Show all saved automations", "\u{26a1}",
                "ask:List my automations"));
        }
    }
    if lower.starts_with("when ") && (lower.contains("then") || lower.contains(",")) {
        if companion_online {
            results.push(lr("ask", "Event automation",
                "AI creates an event-triggered automation", "\u{26a1}",
                SharedString::from(format!("ask:Create an automation: {}", query))));
        }
    }

    // File operations: "show downloads", "list files", "what's in ~/X"
    if lower.starts_with("show ") || lower.starts_with("list ") || lower.contains("downloads")
        || lower.starts_with("what's in ")
    {
        let dir = if lower.contains("downloads") {
            "~/Downloads"
        } else if lower.contains("documents") {
            "~/Documents"
        } else if lower.contains("desktop") {
            "~/Desktop"
        } else {
            ""
        };
        if !dir.is_empty() {
            results.push(lr("find", SharedString::from(format!("Browse {}", dir)), "List directory contents", "📁", SharedString::from(format!("files:{}", dir))));
        }
    }

    // File browser: "files", "browse", "file manager"
    if lower == "files" || lower == "browse" || lower == "browse files"
        || lower == "file manager" || lower == "file browser"
    {
        results.push(lr("find", "Open File Browser", "Browse files on this device", "📁", "navigate:files"));
    }

    // Setting matches: "focus", "timer", "settings"
    if lower.contains("focus") || lower.contains("timer") {
        let focus_secs = parse_focus_duration(&lower);
        let focus_mins = focus_secs / 60;
        results.push(lr("setting", SharedString::from(format!("Focus for {} min", focus_mins)), "Dim desktop, suppress notifications", "◎", SharedString::from(format!("setting:focus:{}", focus_secs))));
    }

    // Settings: "settings", "preferences", "config"
    if lower == "settings" || lower == "preferences" || lower == "config"
        || lower == "configuration" || lower.starts_with("setting")
    {
        results.push(lr("setting", "Settings", "Open system settings", "⚙", "navigate:settings"));
    }

    // Lock screen: "lock", "lock screen"
    if lower == "lock" || lower == "lock screen" || lower == "lock the screen" {
        results.push(lr("setting", "Lock screen", "Lock the desktop", "🔒", "setting:lock"));
    }

    // ── AI-dependent results (hidden when companion is offline) ──
    if companion_online {
        // Memory search: "remember", "what do you know about"
        if lower.starts_with("remember") || lower.contains("you know about")
            || lower.starts_with("recall ")
        {
            results.push(lr("memory", SharedString::from(format!("Search memories: \"{}\"", query)), "Search Yantrik's memory", "🧠", SharedString::from(format!("memory:{}", query))));
        }

        // Smart Intent: NL → tool routing
        let tool_results = match_tool_intents(&lower, query);

        // Build AI section: tool intents + free-form AI chat.
        // If we have instant results above, add a visual separator before AI results.
        let has_instant = !results.is_empty();
        let has_ai = !tool_results.is_empty();

        if has_instant && has_ai {
            results.push(lr("divider", "── AI ──", "", "", ""));
        }

        results.extend(tool_results);

        // Always offer AI conversation as the last option
        if has_instant && !has_ai {
            results.push(lr("divider", "── AI ──", "", "", ""));
        }
        results.push(lr("ask", SharedString::from(format!("Ask: \"{}\"", query)), "Send to Yantrik AI", "?", SharedString::from(format!("ask:{}", query))));
    }

    results
}

// ── Smart Ranking ──

/// Apply frecency and context boosting to results, then sort by composite score.
/// This wraps `build_results()` with intelligent ranking.
pub fn apply_smart_ranking(
    results: &mut Vec<LensResult>,
    query: &str,
    frecency: &crate::frecency::FrecencyStore,
    running_processes: &[yantrik_os::ProcessInfo],
) {
    let lower = query.to_lowercase();

    for r in results.iter_mut() {
        // Skip dividers and answers
        if r.result_type == "divider" || r.result_type == "answer" {
            continue;
        }

        let mut score: f64 = 0.0;

        // Base match quality score
        let title_lower = r.title.to_lowercase();
        if title_lower == lower {
            score += 1.0; // exact match
        } else if title_lower.starts_with(&lower) {
            score += 0.8; // starts with
        } else if title_lower.contains(&lower) {
            score += 0.6; // contains
        } else {
            score += 0.4; // fuzzy / tool match
        }

        // Frecency boost (0.0 - 0.3)
        let frecency_score = frecency.score(r.action_id.as_str());
        score += (frecency_score / 100.0).min(0.3); // Normalize: 100 frecency → 0.3 boost

        // Context boost: running apps get a bump
        if r.result_type == "do" || r.action_id.starts_with("exec:") || r.action_id.starts_with("launch:") {
            let action_lower = r.action_id.to_lowercase();
            let is_running = running_processes.iter().any(|p| {
                action_lower.contains(&p.name.to_lowercase())
            });
            if is_running {
                score += 0.15;
            }
        }

        r.score = score as f32;
    }

    // Sort results by score (descending), but preserve dividers in-place
    // Strategy: extract non-divider results, sort them, rebuild with dividers
    let divider_positions: Vec<(usize, LensResult)> = results
        .iter()
        .enumerate()
        .filter(|(_, r)| r.result_type == "divider")
        .map(|(i, r)| (i, r.clone()))
        .collect();

    if divider_positions.is_empty() {
        // No dividers — simple sort
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        // Sort within sections (between dividers)
        // Find section boundaries
        let mut sections: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        for &(div_pos, _) in &divider_positions {
            if start < div_pos {
                sections.push((start, div_pos));
            }
            start = div_pos + 1;
        }
        if start < results.len() {
            sections.push((start, results.len()));
        }

        // Sort each section independently
        for (s, e) in sections {
            results[s..e].sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }
}

// ── Context Suggestions (empty-query experience) ──

/// Build contextual suggestions based on time, system state, running apps, and notifications.
/// This fires when the Lens opens with no query — the "ambient awareness" experience.
pub fn build_context_suggestions(
    snapshot: &yantrik_os::SystemSnapshot,
    unread_notifications: usize,
    companion_online: bool,
    running_processes: &[yantrik_os::ProcessInfo],
) -> Vec<LensResult> {
    let mut results = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let hour = (now / 3600) % 24;

    // 1. Time-of-day suggestion
    match hour {
        5..=9 => {
            if companion_online {
                results.push(lr(
                    "context", "Morning brief", "Catch up on what's new", "☀",
                    "tool:Give me a morning brief. Check my recent memories and system status.",
                ));
            }
        }
        20..=23 | 0..=4 => {
            if companion_online {
                results.push(lr(
                    "context", "Evening review", "Recap today's activity", "🌙",
                    "tool:Give me an evening review. Summarize what I worked on today using recall_workspace.",
                ));
            }
        }
        _ => {}
    }

    // 2. Battery warning
    if snapshot.battery_level <= 20 && !snapshot.battery_charging {
        let msg = format!("Battery {}% — plug in soon", snapshot.battery_level);
        results.push(lr("context", msg, "Low power warning", "🔋", "system:status"));
    }

    // 3. High CPU usage
    if snapshot.cpu_usage_percent > 80.0 {
        let msg = format!("CPU at {:.0}%", snapshot.cpu_usage_percent);
        results.push(lr("context", msg, "Something is working hard", "⚡", "system:status"));
    }

    // 4. Memory pressure
    if snapshot.memory_total_bytes > 0 {
        let used_pct = (snapshot.memory_used_bytes as f64 / snapshot.memory_total_bytes as f64) * 100.0;
        if used_pct > 85.0 {
            let msg = format!("Memory {:.0}% used", used_pct);
            results.push(lr("context", msg, "Consider closing some apps", "💾", "system:status"));
        }
    }

    // 5. Unread notifications
    if unread_notifications > 0 {
        let msg = format!("{} unread notification{}", unread_notifications,
            if unread_notifications == 1 { "" } else { "s" });
        results.push(lr("context", msg, "Tap to check", "🔔", "navigate:notifications"));
    }

    // 6. Running apps as "switch to" suggestions
    // Look for known GUI apps in the process list
    let gui_apps: &[(&str, &str)] = &[
        ("firefox", "Firefox"), ("chromium", "Chromium"), ("foot", "Terminal"),
        ("thunar", "Files"), ("code", "VS Code"), ("mpv", "Media Player"),
        ("gimp", "GIMP"), ("libreoffice", "LibreOffice"),
    ];
    for (proc_name, label) in gui_apps {
        if running_processes.iter().any(|p| p.name.contains(proc_name)) {
            results.push(lr(
                "context",
                format!("Switch to {}", label),
                "Running",
                "↗",
                format!("window:{}", proc_name),
            ));
        }
    }

    // 7. Always offer to ask the AI (if online)
    if companion_online && results.is_empty() {
        results.push(lr(
            "context", "What can I help with?", "Ask me anything", "✦",
            "ask:What can I help you with today?",
        ));
    }

    results
}

/// Convert frecency entries into "recent" LensResult items for the idle state.
pub fn frecency_to_recents(entries: &[&crate::frecency::FrecencyEntry]) -> Vec<LensResult> {
    entries
        .iter()
        .map(|e| {
            lr(
                "recent",
                &e.title,
                &e.result_type,
                &e.icon_char,
                &e.action_id,
            )
        })
        .collect()
}

// ── Percentage & Math formatting helpers ──

/// Evaluate percentage expressions: "X% of Y" → X/100 * Y, "X% off Y" → Y - X/100 * Y.
fn eval_percentage(input: &str) -> Option<String> {
    let lower = input.to_lowercase();

    // "X% of Y"
    if let Some(idx) = lower.find("% of ") {
        let pct_str = input[..idx].trim();
        let val_str = input[idx + 5..].trim();
        let pct: f64 = pct_str.parse().ok()?;
        let val: f64 = val_str.parse().ok()?;
        let result = pct / 100.0 * val;
        return Some(format_math_result(result));
    }

    // "X% off Y"
    if let Some(idx) = lower.find("% off ") {
        let pct_str = input[..idx].trim();
        let val_str = input[idx + 6..].trim();
        let pct: f64 = pct_str.parse().ok()?;
        let val: f64 = val_str.parse().ok()?;
        let result = val - (pct / 100.0 * val);
        return Some(format_math_result(result));
    }

    None
}

/// Format a math result: integers without decimals, floats trimmed of trailing zeros.
fn format_math_result(result: f64) -> String {
    if result == result.floor() && result.abs() < 1e15 {
        format!("= {}", result as i64)
    } else {
        format!("= {:.6}", result)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

// ── Inline Instant Answers ──

/// Try to produce an inline answer for common queries (math, time, battery, date, percentages).
/// Returns None if the query doesn't match any instant-answer pattern.
pub fn instant_answer(query: &str, snapshot: &yantrik_os::SystemSnapshot) -> Option<LensResult> {
    let lower = query.trim().to_lowercase();

    // Time
    if lower == "time" || lower == "what time" || lower == "what time is it" {
        let time = crate::app_context::current_time_hhmm();
        return Some(answer_result("Time", &time, "🕐", ""));
    }

    // Date
    if lower == "date" || lower == "what day" || lower == "today" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let days_since_epoch = now / 86400;
        // Zeller-ish: compute day-of-week from epoch day (Jan 1 1970 = Thursday = 4)
        let dow = (days_since_epoch + 4) % 7;
        let day_names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
        let day_name = day_names[dow as usize];
        // Approximate month/day (good enough for display)
        let date_str = format_epoch_date(now);
        let val = format!("{}, {}", day_name, date_str);
        return Some(answer_result("Date", &val, "📅", ""));
    }

    // Battery
    if lower == "battery" || lower == "battery level" || lower == "power" {
        let charging = if snapshot.battery_charging { " (charging)" } else { "" };
        let val = format!("{}%{}", snapshot.battery_level, charging);
        return Some(answer_result("Battery", &val, "🔋", "system:status"));
    }

    // Percentage: "X% of Y" or "X% off Y"
    if let Some(val) = eval_percentage(query.trim()) {
        // Strip "= " prefix for the clipboard value
        let copy_val = val.strip_prefix("= ").unwrap_or(&val);
        let action = format!("copy-result:{}", copy_val);
        return Some(answer_result("Calculate", &val, "🧮", &action));
    }

    // Math: try to evaluate expressions (including functions and constants)
    if let Some(result) = eval_math(query.trim()) {
        let val = format_math_result(result);
        // Strip "= " prefix for the clipboard value
        let copy_val = val.strip_prefix("= ").unwrap_or(&val);
        let action = format!("copy-result:{}", copy_val);
        return Some(answer_result("Calculate", &val, "🧮", &action));
    }

    None
}

/// Format epoch seconds as "Month Day, Year".
fn format_epoch_date(epoch_secs: u64) -> String {
    // Civil date from Unix timestamp (no chrono dependency needed)
    let days = (epoch_secs / 86400) as i64;
    // Algorithm from Howard Hinnant (http://howardhinnant.github.io/date_algorithms.html)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let month_names = [
        "", "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ];
    let month_name = month_names.get(m as usize).unwrap_or(&"???");
    format!("{} {}, {}", month_name, d, y)
}

/// Recursive-descent math evaluator.
/// Handles: +, -, *, /, ^, %, parentheses, negative numbers,
/// functions (sqrt, sin, cos, tan, log, ln, abs, ceil, floor, round),
/// and constants (pi, e).
/// Returns None if the input isn't a valid math expression.
pub fn eval_math(input: &str) -> Option<f64> {
    // Quick check: must contain at least one operator, function call, or parentheses to be math
    let has_operator = input
        .chars()
        .any(|c| matches!(c, '+' | '-' | '*' | '/' | '^' | '%' | '(' | ')'));
    let has_function = [
        "sqrt", "sin", "cos", "tan", "log", "ln", "abs", "ceil", "floor", "round", "pi",
    ]
    .iter()
    .any(|f| input.to_lowercase().contains(f));

    if !has_operator && !has_function {
        return None;
    }

    // Must not contain alphabetic chars that aren't known functions/constants
    // (prevent "hello + 5" from being evaluated)
    let lower = input.to_lowercase();
    let cleaned = lower
        .replace("sqrt", "")
        .replace("sin", "")
        .replace("cos", "")
        .replace("tan", "")
        .replace("log", "")
        .replace("ln", "")
        .replace("abs", "")
        .replace("ceil", "")
        .replace("floor", "")
        .replace("round", "")
        .replace("pi", "");
    if cleaned
        .chars()
        .any(|c| c.is_alphabetic() && c != 'e')
    {
        return None;
    }

    let tokens: Vec<char> = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut pos = 0;
    let result = parse_expr(&tokens, &mut pos)?;
    // Must have consumed all tokens
    if pos != tokens.len() {
        return None;
    }
    // Check for NaN/Inf
    if result.is_nan() || result.is_infinite() {
        return None;
    }
    Some(result)
}

fn parse_expr(tokens: &[char], pos: &mut usize) -> Option<f64> {
    let mut left = parse_term(tokens, pos)?;
    while *pos < tokens.len() {
        match tokens[*pos] {
            '+' => { *pos += 1; left += parse_term(tokens, pos)?; }
            '-' => { *pos += 1; left -= parse_term(tokens, pos)?; }
            _ => break,
        }
    }
    Some(left)
}

fn parse_term(tokens: &[char], pos: &mut usize) -> Option<f64> {
    let mut left = parse_power(tokens, pos)?;
    while *pos < tokens.len() {
        match tokens[*pos] {
            '*' => { *pos += 1; left *= parse_power(tokens, pos)?; }
            '/' => { *pos += 1; let r = parse_power(tokens, pos)?; if r == 0.0 { return None; } left /= r; }
            '%' => { *pos += 1; let r = parse_power(tokens, pos)?; if r == 0.0 { return None; } left %= r; }
            _ => break,
        }
    }
    Some(left)
}

fn parse_power(tokens: &[char], pos: &mut usize) -> Option<f64> {
    let base = parse_unary(tokens, pos)?;
    if *pos < tokens.len() && tokens[*pos] == '^' {
        *pos += 1;
        let exp = parse_power(tokens, pos)?; // right-associative
        Some(base.powf(exp))
    } else {
        Some(base)
    }
}

fn parse_unary(tokens: &[char], pos: &mut usize) -> Option<f64> {
    if *pos < tokens.len() && tokens[*pos] == '-' {
        *pos += 1;
        Some(-parse_atom(tokens, pos)?)
    } else {
        parse_atom(tokens, pos)
    }
}

fn parse_atom(tokens: &[char], pos: &mut usize) -> Option<f64> {
    if *pos >= tokens.len() {
        return None;
    }

    // Parenthesized expression
    if tokens[*pos] == '(' {
        *pos += 1;
        let val = parse_expr(tokens, pos)?;
        if *pos >= tokens.len() || tokens[*pos] != ')' {
            return None;
        }
        *pos += 1;
        return Some(val);
    }

    // Try to read a word (function name or constant)
    if tokens[*pos].is_alphabetic() {
        let start = *pos;
        while *pos < tokens.len() && tokens[*pos].is_alphabetic() {
            *pos += 1;
        }
        let word: String = tokens[start..*pos].iter().collect();
        let word_lower = word.to_lowercase();

        // Constants
        match word_lower.as_str() {
            "pi" => return Some(std::f64::consts::PI),
            "e" if *pos >= tokens.len() || tokens[*pos] != '(' => {
                // Only treat as Euler's number if not followed by '('
                return Some(std::f64::consts::E);
            }
            _ => {}
        }

        // Functions: must be followed by '('
        if *pos >= tokens.len() || tokens[*pos] != '(' {
            return None;
        }
        *pos += 1; // skip '('
        let arg = parse_expr(tokens, pos)?;
        if *pos >= tokens.len() || tokens[*pos] != ')' {
            return None;
        }
        *pos += 1; // skip ')'

        return match word_lower.as_str() {
            "sqrt" => Some(arg.sqrt()),
            "sin" => Some(arg.to_radians().sin()),
            "cos" => Some(arg.to_radians().cos()),
            "tan" => Some(arg.to_radians().tan()),
            "log" => Some(arg.log10()),
            "ln" => Some(arg.ln()),
            "abs" => Some(arg.abs()),
            "ceil" => Some(arg.ceil()),
            "floor" => Some(arg.floor()),
            "round" => Some(arg.round()),
            _ => None,
        };
    }

    // Number
    let start = *pos;
    while *pos < tokens.len() && (tokens[*pos].is_ascii_digit() || tokens[*pos] == '.') {
        *pos += 1;
    }
    // Handle scientific notation
    if *pos < tokens.len() && (tokens[*pos] == 'e' || tokens[*pos] == 'E') {
        *pos += 1;
        if *pos < tokens.len() && (tokens[*pos] == '+' || tokens[*pos] == '-') {
            *pos += 1;
        }
        while *pos < tokens.len() && tokens[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    if start == *pos {
        return None;
    }
    let num_str: String = tokens[start..*pos].iter().collect();
    num_str.parse::<f64>().ok()
}

// ── Smart Intent: NL → tool matching ──

/// Match natural language queries to tool store tools.
/// Returns LensResult entries with `tool:QUERY` action IDs that route to the LLM
/// with optimized prompts for instant tool invocation.
fn match_tool_intents(lower: &str, original: &str) -> Vec<LensResult> {
    let mut results = Vec::new();

    // ── Weather ──
    if lower.contains("weather") || lower.contains("forecast")
        || lower.starts_with("is it raining") || lower.starts_with("is it snowing")
        || (lower.contains("temperature") && lower.contains("outside"))
    {
        let location = lower
            .strip_prefix("weather in ")
            .or_else(|| lower.strip_prefix("weather for "))
            .or_else(|| lower.strip_prefix("forecast for "))
            .or_else(|| lower.strip_prefix("forecast in "))
            .or_else(|| {
                let stripped = lower.strip_prefix("weather ")?;
                if stripped == "forecast" { None } else { Some(stripped) }
            })
            .unwrap_or("")
            .trim();
        let query = if location.is_empty() {
            "Get the current weather.".to_string()
        } else {
            format!("Get the weather in {}.", location)
        };
        let title = if location.is_empty() {
            "Weather (current location)".to_string()
        } else {
            format!("Weather in {}", capitalize(location))
        };
        results.push(lr("tool", SharedString::from(title), "Current conditions via wttr.in", "W", SharedString::from(format!("tool:{}", query))));
    }

    // ── WiFi ──
    if lower.contains("wifi") || lower.contains("wi-fi") || lower.contains("available networks") {
        if lower.contains("scan") || lower.contains("available") || lower.contains("networks") {
            results.push(lr("tool", "Scan WiFi networks", "Find available wireless networks", "~", "tool:Scan for available WiFi networks."));
        } else if lower.contains("disconnect") || lower.contains("turn off") {
            results.push(lr("tool", "Disconnect WiFi", "Turn off wireless connection", "~", "tool:Disconnect from WiFi."));
        } else if lower.contains("connect") {
            let ssid = lower
                .strip_prefix("connect to wifi ")
                .or_else(|| lower.strip_prefix("connect to wi-fi "))
                .or_else(|| lower.strip_prefix("wifi connect "))
                .or_else(|| lower.strip_prefix("connect to "))
                .unwrap_or("")
                .trim();
            if ssid.is_empty() {
                results.push(lr("tool", "Connect to WiFi", "Scan and connect to a network", "~", "tool:Scan WiFi networks so I can pick one to connect to."));
            } else {
                results.push(lr("tool", SharedString::from(format!("Connect to '{}'", ssid)), "Join wireless network", "~", SharedString::from(format!("tool:Connect to WiFi network '{}'.", ssid))));
            }
        } else {
            results.push(lr("tool", "WiFi status", "Show current connection info", "~", "tool:Show my WiFi connection status."));
        }
    }

    // ── Bluetooth ──
    if lower.contains("bluetooth") || lower.contains("bt ") || lower.starts_with("bt") {
        if lower.contains("scan") || lower.contains("devices") || lower.contains("find") {
            results.push(lr("tool", "Scan Bluetooth devices", "Find nearby Bluetooth devices", "B", "tool:Scan for nearby Bluetooth devices."));
        } else if lower.contains("pair") {
            results.push(lr("tool", "Pair Bluetooth device", "Enter pairing mode", "B", "tool:Show Bluetooth devices so I can pair one."));
        } else if lower.contains("disconnect") {
            results.push(lr("tool", "Disconnect Bluetooth", "Disconnect current device", "B", "tool:Show connected Bluetooth devices and disconnect them."));
        } else {
            results.push(lr("tool", "Bluetooth info", "Show paired/connected devices", "B", "tool:Show Bluetooth status and connected devices."));
        }
    }

    // ── Volume / Audio ──
    if lower.contains("volume") || lower.contains("mute") || lower.contains("unmute")
        || lower.contains("audio") || lower.contains("sound")
        || lower.starts_with("what's playing")
    {
        if lower.contains("mute") && !lower.contains("unmute") {
            results.push(lr("tool", "Mute audio", "Mute system volume", "M", "tool:Mute the system audio."));
        } else if lower.contains("unmute") {
            results.push(lr("tool", "Unmute audio", "Restore system volume", "V", "tool:Unmute the system audio."));
        } else if lower.contains("volume") {
            let vol = extract_number(lower);
            if let Some(v) = vol {
                let v = v.min(100);
                results.push(lr("tool", SharedString::from(format!("Set volume to {}%", v)), "Adjust system volume", "V", SharedString::from(format!("tool:Set the system volume to {}%.", v))));
            } else {
                results.push(lr("tool", "Audio info", "Show volume and audio device info", "V", "tool:Show current audio volume and device info."));
            }
        } else {
            results.push(lr("tool", "Audio info", "Show volume and audio devices", "V", "tool:Show current audio volume and device info."));
        }
    }

    // ── Screenshot ──
    if lower.contains("screenshot") || lower.contains("screen capture")
        || lower.starts_with("capture screen") || lower.starts_with("take a screen")
    {
        results.push(lr("tool", "Take screenshot", "Capture the current screen", "S", "tool:Take a screenshot of the screen."));
    }

    // ── Calculator / Math ──
    if lower.starts_with("calculate ") || lower.starts_with("calc ")
        || lower.starts_with("what is ") || lower.starts_with("what's ")
        || lower.starts_with("how much is ")
        || looks_like_math(lower)
    {
        let expr = lower
            .strip_prefix("calculate ")
            .or_else(|| lower.strip_prefix("calc "))
            .or_else(|| lower.strip_prefix("what is "))
            .or_else(|| lower.strip_prefix("what's "))
            .or_else(|| lower.strip_prefix("how much is "))
            .unwrap_or(lower)
            .trim();
        if !expr.is_empty() {
            results.push(lr("tool", SharedString::from(format!("Calculate: {}", expr)), "Evaluate expression", "=", SharedString::from(format!("tool:Calculate: {}", expr))));
        }
    }

    // ── Unit conversion ──
    if lower.starts_with("convert ") || lower.contains(" to ") && has_unit_keyword(lower) {
        results.push(lr("tool", SharedString::from(format!("Convert: {}", original)), "Unit conversion", "=", SharedString::from(format!("tool:{}", original))));
    }

    // ── Git ──
    if lower.starts_with("git ") {
        let sub = &lower[4..];
        if sub.starts_with("status") {
            results.push(lr("tool", "Git status", "Show working tree status", "G", "tool:Show the git status of the current repository."));
        } else if sub.starts_with("log") {
            results.push(lr("tool", "Git log", "Show recent commits", "G", "tool:Show the recent git commit log."));
        } else if sub.starts_with("diff") {
            results.push(lr("tool", "Git diff", "Show uncommitted changes", "G", "tool:Show the current git diff of uncommitted changes."));
        } else if sub.starts_with("branch") {
            results.push(lr("tool", "Git branches", "List branches", "G", "tool:Show all git branches."));
        } else if sub.starts_with("clone") {
            let url = sub.strip_prefix("clone ").unwrap_or("").trim();
            if !url.is_empty() {
                results.push(lr("tool", SharedString::from(format!("Git clone {}", url)), "Clone repository", "G", SharedString::from(format!("tool:Clone the git repository: {}", url))));
            }
        }
    }

    // ── Package management ──
    if lower.starts_with("install ") || lower.starts_with("uninstall ")
        || lower.starts_with("remove package") || lower.starts_with("search package")
        || lower.starts_with("package ")
    {
        if let Some(pkg) = lower.strip_prefix("install ") {
            let pkg = pkg.trim();
            if !pkg.is_empty() {
                results.push(lr("tool", SharedString::from(format!("Install {}", pkg)), "Install system package", "P", SharedString::from(format!("tool:Install the package '{}'.", pkg))));
            }
        } else if lower.starts_with("uninstall ") || lower.starts_with("remove package") {
            let pkg = lower
                .strip_prefix("uninstall ")
                .or_else(|| lower.strip_prefix("remove package "))
                .unwrap_or("")
                .trim();
            if !pkg.is_empty() {
                results.push(lr("tool", SharedString::from(format!("Remove {}", pkg)), "Uninstall system package", "P", SharedString::from(format!("tool:Remove the package '{}'.", pkg))));
            }
        } else if let Some(pkg) = lower.strip_prefix("search package ").or_else(|| lower.strip_prefix("package search ")) {
            let pkg = pkg.trim();
            if !pkg.is_empty() {
                results.push(lr("tool", SharedString::from(format!("Search packages: {}", pkg)), "Search available packages", "P", SharedString::from(format!("tool:Search for packages matching '{}'.", pkg))));
            }
        }
    }

    // ── Service management ──
    if lower.contains("service") || lower.starts_with("restart ")
        || lower.starts_with("stop ") && !lower.contains("timer")
        || lower.starts_with("start ") && !lower.contains("focus")
    {
        if lower.contains("list") || lower == "services" {
            results.push(lr("tool", "List services", "Show running system services", "D", "tool:List all running system services."));
        } else if lower.starts_with("restart ") {
            let svc = lower.strip_prefix("restart ").unwrap_or("").trim();
            if !svc.is_empty() && !svc.contains("service") {
                results.push(lr("tool", SharedString::from(format!("Restart {}", svc)), "Restart system service", "D", SharedString::from(format!("tool:Restart the service '{}'.", svc))));
            }
        } else if let Some(rest) = lower.strip_prefix("service status ") {
            let svc = rest.trim();
            if !svc.is_empty() {
                results.push(lr("tool", SharedString::from(format!("Status of {}", svc)), "Check service status", "D", SharedString::from(format!("tool:Show the status of service '{}'.", svc))));
            }
        }
    }

    // ── Processes ──
    if lower.contains("processes") || lower.starts_with("kill ")
        || lower.contains("using cpu") || lower.contains("top processes")
        || lower == "htop" || lower == "top"
    {
        if lower.starts_with("kill ") {
            let proc = lower.strip_prefix("kill ").unwrap_or("").trim();
            if !proc.is_empty() {
                results.push(lr("tool", SharedString::from(format!("Kill {}", proc)), "Terminate process", "X", SharedString::from(format!("tool:Kill the process '{}'.", proc))));
            }
        } else {
            results.push(lr("tool", "Running processes", "List active processes", "P", "tool:List running processes sorted by CPU usage."));
        }
    }

    // ── Disk / Storage ──
    if lower.contains("disk") || lower.contains("storage") || lower.contains("space left")
        || lower.contains("how much space") || lower.contains("dir size")
        || lower.contains("directory size") || lower.contains("mount")
    {
        if lower.contains("mount") {
            results.push(lr("tool", "Mount info", "Show mounted filesystems", "H", "tool:Show mounted filesystems and their info."));
        } else if lower.contains("dir") || lower.contains("directory") || lower.contains("folder size") {
            let path = extract_path_from_query(lower);
            let query = if path.is_empty() {
                "Show the size of my home directory.".to_string()
            } else {
                format!("Show the size of directory {}.", path)
            };
            results.push(lr("tool", SharedString::from(if path.is_empty() { "Directory size (~)".to_string() } else { format!("Size of {}", path) }), "Calculate directory size", "H", SharedString::from(format!("tool:{}", query))));
        } else {
            results.push(lr("find", "Disk usage", "Show disk space for all partitions", "H", "tool:Show disk space usage for all partitions."));
        }
    }

    // ── Display / Resolution ──
    if lower.contains("resolution") || lower.contains("display info")
        || lower.contains("monitors") || lower.contains("screen info")
    {
        if lower.contains("set") || lower.contains("change") {
            results.push(lr("tool", "Change resolution", "Set display resolution", "D", SharedString::from(format!("tool:{}", original))));
        } else {
            results.push(lr("tool", "Display info", "Show connected displays and resolutions", "D", "tool:Show display info — connected monitors and resolutions."));
        }
    }

    // ── Wallpaper ──
    if lower.contains("wallpaper") || lower.contains("background") && lower.contains("change")
        || lower.contains("background") && lower.contains("set")
    {
        let path = extract_path_from_query(lower);
        if path.is_empty() {
            results.push(lr("tool", "Set wallpaper", "Change desktop background", "I", SharedString::from(format!("tool:{}", original))));
        } else {
            results.push(lr("tool", SharedString::from(format!("Set wallpaper: {}", path)), "Change desktop background", "I", SharedString::from(format!("tool:Set the wallpaper to {}.", path))));
        }
    }

    // ── Encoding / Base64 / JSON ──
    if lower.starts_with("base64 ") || lower.starts_with("url encode")
        || lower.starts_with("url decode") || lower.contains("format json")
        || lower.contains("pretty json") || lower.starts_with("encode ")
        || lower.starts_with("decode ")
    {
        results.push(lr("tool", SharedString::from(capitalize(original)), "Encoding / formatting tool", "#", SharedString::from(format!("tool:{}", original))));
    }

    // ── Archive ──
    if lower.starts_with("extract ") || lower.starts_with("compress ")
        || lower.starts_with("unzip ") || lower.starts_with("untar ")
        || lower.contains("create archive") || lower.contains("make tar")
    {
        results.push(lr("tool", SharedString::from(capitalize(original)), "Archive create/extract", "Z", SharedString::from(format!("tool:{}", original))));
    }

    // ── Window management ──
    if lower.contains("window") || lower.starts_with("close ") && !lower.contains("lens")
        || lower.starts_with("switch to ") || lower.starts_with("focus ")
    {
        if lower.contains("list") || lower == "windows" {
            results.push(lr("tool", "List windows", "Show all open windows", "W", "tool:List all open windows."));
        } else if lower.starts_with("close ") {
            let target = lower.strip_prefix("close ").unwrap_or("").trim();
            if !target.is_empty() && target != "lens" {
                results.push(lr("tool", SharedString::from(format!("Close {}", target)), "Close window", "X", SharedString::from(format!("tool:Close the window titled '{}'.", target))));
            }
        } else if lower.starts_with("switch to ") || lower.starts_with("focus ") {
            let target = lower
                .strip_prefix("switch to ")
                .or_else(|| lower.strip_prefix("focus "))
                .unwrap_or("")
                .trim();
            if !target.is_empty() && target != "mode" && target != "timer" {
                results.push(lr("tool", SharedString::from(format!("Focus {}", target)), "Bring window to front", "W", SharedString::from(format!("tool:Focus the window titled '{}'.", target))));
            }
        }
    }

    // ── Date/Time ──
    if lower.starts_with("what time") || lower.starts_with("what date")
        || lower.starts_with("what day") || lower.starts_with("how long until")
        || lower.starts_with("days until") || lower.starts_with("date calc")
    {
        results.push(lr("tool", SharedString::from(capitalize(original)), "Date/time calculation", "T", SharedString::from(format!("tool:{}", original))));
    }

    // ── Network / Download ──
    if lower.starts_with("download ") || lower.starts_with("fetch ") {
        let target = lower
            .strip_prefix("download ")
            .or_else(|| lower.strip_prefix("fetch "))
            .unwrap_or("")
            .trim();
        if !target.is_empty() {
            results.push(lr("tool", SharedString::from(format!("Download {}", target)), "Download file from URL", "D", SharedString::from(format!("tool:Download the file from {}.", target))));
        }
    }

    // ── File hash / diff / word count ──
    if lower.starts_with("hash ") || lower.starts_with("sha256 ")
        || lower.starts_with("diff ") || lower.starts_with("word count ")
        || lower.starts_with("wc ")
    {
        results.push(lr("tool", SharedString::from(capitalize(original)), "Text/file utility", "#", SharedString::from(format!("tool:{}", original))));
    }

    // ── System info (extended) ──
    if lower.contains("battery") || lower.contains("memory") || lower.contains("ram")
        || lower.contains("uptime") || lower == "system info" || lower == "sysinfo"
        || lower.starts_with("system status")
    {
        results.push(lr("tool", "System info", "CPU, RAM, disk, uptime, kernel", "I", "tool:Show detailed system info — CPU, RAM, disk, uptime, kernel."));
    }

    // ── Sysadmin NL queries ──
    // "what's using port X", "why is CPU high", "clean up Docker"
    if lower.contains("port ") && (lower.contains("using") || lower.contains("listening")
        || lower.contains("what's on") || lower.contains("who's on"))
    {
        let port = extract_number(lower).unwrap_or(0);
        let query = if port > 0 {
            format!("Check what process is using port {}. Use run_command to run: ss -tlnp | grep {}", port, port)
        } else {
            "List all listening network ports. Use run_command to run: ss -tlnp".to_string()
        };
        results.push(lr("tool", SharedString::from(if port > 0 { format!("What's on port {}", port) } else { "Listening ports".to_string() }), "Check network port usage", "N", SharedString::from(format!("tool:{}", query))));
    }

    if lower.contains("cpu") && (lower.contains("high") || lower.contains("why") || lower.contains("hot")
        || lower.contains("slow") || lower.contains("100"))
    {
        results.push(lr("tool", "Diagnose high CPU", "Find what's consuming CPU", "C", "tool:List running processes sorted by CPU usage, identify what's consuming the most CPU, and suggest what to do about it."));
    }

    if lower.contains("docker") && (lower.contains("clean") || lower.contains("prune")
        || lower.contains("unused") || lower.contains("space"))
    {
        results.push(lr("tool", "Clean up Docker", "Remove unused containers, images, volumes", "D", "tool:Show Docker disk usage with run_command 'docker system df', then suggest cleanup steps."));
    }

    if lower.contains("ip") && (lower.starts_with("what") || lower.starts_with("my ") || lower.contains("my ip"))
        || lower == "ip address" || lower == "ip addr"
    {
        results.push(lr("tool", "My IP address", "Show network interfaces and IPs", "N", "tool:Show my network interfaces and IP addresses."));
    }

    // ── Terminal / Fix Error (the killer feature) ──
    if lower.starts_with("fix") || lower.contains("error") || lower.contains("what went wrong")
        || lower.starts_with("debug") || lower.starts_with("why did")
        || lower.contains("terminal output") || lower.contains("scrollback")
        || lower.contains("what happened") || lower == "help"
    {
        // "fix this", "fix this error", "what went wrong", "debug this"
        results.push(lr("tool", "Fix this error", "Read terminal output and diagnose the problem", "!", "tool:Read the terminal scrollback buffer with read_terminal_buffer, analyze any errors or failures you find, explain what went wrong, and suggest a fix."));
    }

    // ── Workspace / Session resume ──
    if lower.contains("where was i") || lower.contains("what was i doing")
        || lower.starts_with("resume") || lower.contains("last session")
        || lower.contains("what was i working") || lower.contains("pick up where")
    {
        results.push(lr("tool", "Resume last session", "Recall what you were working on", "R", "tool:Use recall_workspace to find my last workspace snapshot, then summarize what I was working on and suggest how to resume."));
    }

    // ── Notification ──
    if lower.starts_with("notify ") || lower.starts_with("send notification") {
        let msg = lower
            .strip_prefix("notify ")
            .or_else(|| lower.strip_prefix("send notification "))
            .unwrap_or("")
            .trim();
        if !msg.is_empty() {
            results.push(lr("tool", SharedString::from(format!("Notify: {}", msg)), "Send desktop notification", "N", SharedString::from(format!("tool:Send a notification with the message '{}'.", msg))));
        }
    }

    // ── NL App Launch (intent-based) ──
    // "open my browser", "browse the web", "play music", "edit code", "write notes"
    if results.is_empty() {
        let intent_apps: &[(&[&str], &str, &str, &str)] = &[
            (&["open my browser", "browse the web", "browse web", "web browser", "go online"],
             "Open Browser", "Launch web browser", "launch:browser"),
            (&["open my terminal", "command line", "open terminal", "open shell", "open a terminal"],
             "Open Terminal", "Launch terminal emulator", "launch:terminal"),
            (&["play music", "music player", "media player", "play video", "play media"],
             "Open Media Player", "Launch media app", "exec:mpv"),
            (&["edit code", "code editor", "open editor", "text editor"],
             "Open Editor", "Launch text editor", "navigate:editor"),
        ];
        for (triggers, title, subtitle, action) in intent_apps {
            if triggers.iter().any(|t| lower == *t || lower.starts_with(t)) {
                results.push(lr("do", *title, *subtitle, "▶", *action));
                break;
            }
        }
    }

    // ── File Content Search ──
    // "find file about X", "where is my X", "find my resume", "that file about X"
    if lower.contains("find file") || lower.contains("find my ")
        || lower.contains("where is my") || lower.contains("that file about")
        || lower.contains("where did i put") || lower.contains("locate ")
    {
        let subject = lower
            .strip_prefix("find file about ").or_else(|| lower.strip_prefix("find file "))
            .or_else(|| lower.strip_prefix("find my "))
            .or_else(|| lower.strip_prefix("where is my "))
            .or_else(|| lower.strip_prefix("locate "))
            .or_else(|| lower.strip_prefix("where did i put "))
            .unwrap_or(lower)
            .trim()
            .replace("that file about ", "");
        if !subject.is_empty() {
            results.push(lr("find", SharedString::from(format!("Find: \"{}\"", subject)),
                "Search files by content via AI", "F",
                SharedString::from(format!("tool:Search for files related to '{}'. Use search_files and recall to find relevant files, then tell me the path so I can open it.", subject))));
        }
    }

    // ── Window Organization ──
    // "organize for coding", "close all browsers", "focus on writing"
    if lower.starts_with("organize ") || lower.contains("organize for")
        || (lower.starts_with("close all ") && !lower.contains("lens"))
        || lower.starts_with("show only ")
    {
        if lower.starts_with("close all ") {
            let target = lower.strip_prefix("close all ").unwrap_or("").trim();
            if !target.is_empty() {
                results.push(lr("tool", SharedString::from(format!("Close all {} windows", target)),
                    "Batch close by app type", "X",
                    SharedString::from(format!("tool:List all open windows. Close every window that looks like a {}. Tell me what you closed.", target))));
            }
        } else {
            let context = lower
                .strip_prefix("organize for ").or_else(|| lower.strip_prefix("organize "))
                .or_else(|| lower.strip_prefix("show only "))
                .unwrap_or("general")
                .trim();
            results.push(lr("tool", SharedString::from(format!("Organize for {}", context)),
                "Focus relevant windows, suggest closing distractors", ">_",
                SharedString::from(format!("tool:Use focus_context with context '{}'. Focus the most relevant window for that task. List any unrelated windows and ask if I want to close them.", context))));
        }
    }

    // ── Workspace Templates ──
    // "start coding workspace", "save workspace", "load workspace"
    if (lower.starts_with("start ") || lower.starts_with("load ") || lower.starts_with("setup for "))
        && lower.contains("workspace")
    {
        let template = lower
            .strip_prefix("start ").or_else(|| lower.strip_prefix("load "))
            .or_else(|| lower.strip_prefix("setup for "))
            .unwrap_or("")
            .replace("workspace", "").trim().to_string();
        let template = if template.is_empty() { "default".to_string() } else { template };
        results.push(lr("tool", SharedString::from(format!("Start {} workspace", template)),
            "Launch apps for this activity", "W",
            SharedString::from(format!("tool:Apply the workspace template '{}'. Use apply_workspace_template to launch the right apps and restore context.", template))));
    } else if lower.starts_with("save ") && lower.contains("workspace") {
        results.push(lr("tool", "Save workspace template",
            "Remember this app layout as a template", "W",
            "tool:Save the current workspace as a named template. Use save_workspace_template. Ask me what to name it."));
    }

    results
}

// ── Helpers ──

/// Parse a natural language query for focus duration.
/// "25min", "30 minutes", "1 hour", "2h" → seconds.
pub fn parse_focus_duration(query: &str) -> u32 {
    let words: Vec<&str> = query.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let num_end = word.find(|c: char| !c.is_ascii_digit()).unwrap_or(word.len());
        if num_end > 0 {
            if let Ok(n) = word[..num_end].parse::<u32>() {
                let suffix = &word[num_end..];
                if suffix.starts_with("min") || suffix == "m" {
                    if n > 0 && n <= 480 {
                        return n * 60;
                    }
                }
                if suffix.starts_with("hour") || suffix.starts_with("hr") || suffix == "h" {
                    if n > 0 && n <= 8 {
                        return n * 3600;
                    }
                }
                if suffix.is_empty() && n > 0 {
                    let next = words.get(i + 1).copied().unwrap_or("");
                    if next.starts_with("hour") || next.starts_with("hr") || next == "h" {
                        if n <= 8 {
                            return n * 3600;
                        }
                    }
                    if next.starts_with("min") || next == "m" || next.is_empty() {
                        if n <= 480 {
                            return n * 60;
                        }
                    }
                }
            }
        }
    }
    25 * 60 // Default: 25 minutes (pomodoro)
}

fn looks_like_math(s: &str) -> bool {
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let has_op = s.contains('+') || s.contains('-') || s.contains('*') || s.contains('/')
        || s.contains('^') || s.contains('%');
    let has_func = ["sqrt(", "sin(", "cos(", "tan(", "log(", "ln(", "abs(", "ceil(", "floor(", "round("]
        .iter()
        .any(|f| s.to_lowercase().contains(f));
    (has_digit && has_op || has_func) && s.len() < 100
}

fn has_unit_keyword(s: &str) -> bool {
    let units = [
        "km", "mi", "mile", "meter", "feet", "ft", "inch", "cm", "mm", "yard",
        "kg", "lb", "pound", "oz", "ounce", "gram", "ton",
        "celsius", "fahrenheit", "kelvin",
        "gb", "mb", "kb", "tb", "byte",
        "hour", "minute", "second", "day", "week",
    ];
    let lower = s.to_lowercase();
    units.iter().any(|u| lower.contains(u))
}

fn extract_number(s: &str) -> Option<u32> {
    let mut num_str = String::new();
    let mut found = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            num_str.push(c);
            found = true;
        } else if found {
            break;
        }
    }
    num_str.parse().ok()
}

fn extract_path_from_query(s: &str) -> String {
    for word in s.split_whitespace() {
        if word.starts_with("~/") || word.starts_with('/') {
            return word.to_string();
        }
    }
    String::new()
}

/// Capitalize first letter of a string.
pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
