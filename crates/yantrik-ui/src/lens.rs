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
    /// Close the Lens, nothing else.
    #[allow(dead_code)]
    CloseLens,
    /// No-op (unknown action).
    Noop,
}

/// Parse an action_id string into a concrete LensAction.
/// `installed_apps` is the scanned .desktop entries for resolving `launch:` by app_id.
pub fn resolve_action(action_id: &str, installed_apps: &[DesktopEntry]) -> LensAction {
    if action_id.starts_with("launch:") {
        let app_id = &action_id[7..];
        // First check installed .desktop apps
        for entry in installed_apps {
            if entry.app_id == app_id {
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
pub fn build_results(
    query: &str,
    onboarding_step: i32,
    installed_apps: &[DesktopEntry],
    clip_history: &[(usize, ClipEntry)],
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
            results.push(LensResult {
                result_type: "do".into(),
                title: SharedString::from(format!("Open {}", entry.name)),
                subtitle: SharedString::from(if entry.comment.is_empty() {
                    format!("Launch {}", entry.exec.split_whitespace().next().unwrap_or(&entry.exec))
                } else {
                    entry.comment.clone()
                }),
                icon_char: SharedString::from(&entry.icon_char),
                action_id: SharedString::from(format!("exec:{}", entry.exec)),
            });
        }
    }

    // Fallback: hardcoded KNOWN_APPS (when no .desktop files available)
    if installed_apps.is_empty() {
        for (app_id, _cmd, desc) in KNOWN_APPS {
            if app_id.contains(&lower) || lower.contains(app_id) || lower.contains("open") {
                results.push(LensResult {
                    result_type: "do".into(),
                    title: SharedString::from(format!("Open {}", capitalize(app_id))),
                    subtitle: SharedString::from(*desc),
                    icon_char: "▶".into(),
                    action_id: SharedString::from(format!("launch:{}", app_id)),
                });
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
                results.push(LensResult {
                    result_type: "do".into(),
                    title: SharedString::from(format!("Search: \"{}\"", rest)),
                    subtitle: "Open in browser".into(),
                    icon_char: "🔍".into(),
                    action_id: SharedString::from(format!("url:{}", search_url)),
                });
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
            results.push(LensResult {
                result_type: "do".into(),
                title: SharedString::from(format!("Open {}", &url)),
                subtitle: "Open in browser".into(),
                icon_char: "🌐".into(),
                action_id: SharedString::from(format!("url:{}", url)),
            });
        }
    }

    // Clipboard: "copy X", "paste", "clipboard", "clipboard history"
    if lower == "paste" || lower == "clipboard" || lower.starts_with("what's on clipboard")
        || lower.starts_with("what did i copy") || lower.contains("clipboard history")
        || lower.contains("paste history") || lower.starts_with("copied")
    {
        results.push(LensResult {
            result_type: "do".into(),
            title: "Read clipboard".into(),
            subtitle: "Show current clipboard".into(),
            icon_char: "📋".into(),
            action_id: "clipboard:read".into(),
        });

        // Show clipboard history entries
        for &(index, ref entry) in clip_history.iter().take(6) {
            results.push(LensResult {
                result_type: "clipboard".into(),
                title: SharedString::from(entry.preview()),
                subtitle: SharedString::from(entry.time_ago()),
                icon_char: "C".into(),
                action_id: SharedString::from(format!("clipboard:paste:{}", index)),
            });
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
            results.push(LensResult {
                result_type: "find".into(),
                title: SharedString::from(format!("Browse {}", dir)),
                subtitle: "List directory contents".into(),
                icon_char: "📁".into(),
                action_id: SharedString::from(format!("files:{}", dir)),
            });
        }
    }

    // File browser: "files", "browse", "file manager"
    if lower == "files" || lower == "browse" || lower == "browse files"
        || lower == "file manager" || lower == "file browser"
    {
        results.push(LensResult {
            result_type: "find".into(),
            title: "Open File Browser".into(),
            subtitle: "Browse files on this device".into(),
            icon_char: "📁".into(),
            action_id: "navigate:files".into(),
        });
    }

    // Setting matches: "focus", "timer", "settings"
    if lower.contains("focus") || lower.contains("timer") {
        let focus_secs = parse_focus_duration(&lower);
        let focus_mins = focus_secs / 60;
        results.push(LensResult {
            result_type: "setting".into(),
            title: SharedString::from(format!("Focus for {} min", focus_mins)),
            subtitle: "Dim desktop, suppress notifications".into(),
            icon_char: "◎".into(),
            action_id: SharedString::from(format!("setting:focus:{}", focus_secs)),
        });
    }

    // Settings: "settings", "preferences", "config"
    if lower == "settings" || lower == "preferences" || lower == "config"
        || lower == "configuration" || lower.starts_with("setting")
    {
        results.push(LensResult {
            result_type: "setting".into(),
            title: "Settings".into(),
            subtitle: "Open system settings".into(),
            icon_char: "⚙".into(),
            action_id: "navigate:settings".into(),
        });
    }

    // Lock screen: "lock", "lock screen"
    if lower == "lock" || lower == "lock screen" || lower == "lock the screen" {
        results.push(LensResult {
            result_type: "setting".into(),
            title: "Lock screen".into(),
            subtitle: "Lock the desktop".into(),
            icon_char: "🔒".into(),
            action_id: "setting:lock".into(),
        });
    }

    // Memory search: "remember", "what do you know about"
    if lower.starts_with("remember") || lower.contains("you know about")
        || lower.starts_with("recall ")
    {
        results.push(LensResult {
            result_type: "memory".into(),
            title: SharedString::from(format!("Search memories: \"{}\"", query)),
            subtitle: "Search Yantrik's memory".into(),
            icon_char: "🧠".into(),
            action_id: SharedString::from(format!("memory:{}", query)),
        });
    }

    // Smart Intent: NL → tool routing
    results.extend(match_tool_intents(&lower, query));

    // Always offer AI conversation as the last option
    results.push(LensResult {
        result_type: "ask".into(),
        title: SharedString::from(format!("Ask: \"{}\"", query)),
        subtitle: "Send to Yantrik AI".into(),
        icon_char: "?".into(),
        action_id: SharedString::from(format!("ask:{}", query)),
    });

    results
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
        results.push(LensResult {
            result_type: "tool".into(),
            title: SharedString::from(title),
            subtitle: "Current conditions via wttr.in".into(),
            icon_char: "W".into(),
            action_id: SharedString::from(format!("tool:{}", query)),
        });
    }

    // ── WiFi ──
    if lower.contains("wifi") || lower.contains("wi-fi") || lower.contains("available networks") {
        if lower.contains("scan") || lower.contains("available") || lower.contains("networks") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Scan WiFi networks".into(),
                subtitle: "Find available wireless networks".into(),
                icon_char: "~".into(),
                action_id: "tool:Scan for available WiFi networks.".into(),
            });
        } else if lower.contains("disconnect") || lower.contains("turn off") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Disconnect WiFi".into(),
                subtitle: "Turn off wireless connection".into(),
                icon_char: "~".into(),
                action_id: "tool:Disconnect from WiFi.".into(),
            });
        } else if lower.contains("connect") {
            let ssid = lower
                .strip_prefix("connect to wifi ")
                .or_else(|| lower.strip_prefix("connect to wi-fi "))
                .or_else(|| lower.strip_prefix("wifi connect "))
                .or_else(|| lower.strip_prefix("connect to "))
                .unwrap_or("")
                .trim();
            if ssid.is_empty() {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: "Connect to WiFi".into(),
                    subtitle: "Scan and connect to a network".into(),
                    icon_char: "~".into(),
                    action_id: "tool:Scan WiFi networks so I can pick one to connect to.".into(),
                });
            } else {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Connect to '{}'", ssid)),
                    subtitle: "Join wireless network".into(),
                    icon_char: "~".into(),
                    action_id: SharedString::from(format!("tool:Connect to WiFi network '{}'.", ssid)),
                });
            }
        } else {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "WiFi status".into(),
                subtitle: "Show current connection info".into(),
                icon_char: "~".into(),
                action_id: "tool:Show my WiFi connection status.".into(),
            });
        }
    }

    // ── Bluetooth ──
    if lower.contains("bluetooth") || lower.contains("bt ") || lower.starts_with("bt") {
        if lower.contains("scan") || lower.contains("devices") || lower.contains("find") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Scan Bluetooth devices".into(),
                subtitle: "Find nearby Bluetooth devices".into(),
                icon_char: "B".into(),
                action_id: "tool:Scan for nearby Bluetooth devices.".into(),
            });
        } else if lower.contains("pair") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Pair Bluetooth device".into(),
                subtitle: "Enter pairing mode".into(),
                icon_char: "B".into(),
                action_id: "tool:Show Bluetooth devices so I can pair one.".into(),
            });
        } else if lower.contains("disconnect") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Disconnect Bluetooth".into(),
                subtitle: "Disconnect current device".into(),
                icon_char: "B".into(),
                action_id: "tool:Show connected Bluetooth devices and disconnect them.".into(),
            });
        } else {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Bluetooth info".into(),
                subtitle: "Show paired/connected devices".into(),
                icon_char: "B".into(),
                action_id: "tool:Show Bluetooth status and connected devices.".into(),
            });
        }
    }

    // ── Volume / Audio ──
    if lower.contains("volume") || lower.contains("mute") || lower.contains("unmute")
        || lower.contains("audio") || lower.contains("sound")
        || lower.starts_with("what's playing")
    {
        if lower.contains("mute") && !lower.contains("unmute") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Mute audio".into(),
                subtitle: "Mute system volume".into(),
                icon_char: "M".into(),
                action_id: "tool:Mute the system audio.".into(),
            });
        } else if lower.contains("unmute") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Unmute audio".into(),
                subtitle: "Restore system volume".into(),
                icon_char: "V".into(),
                action_id: "tool:Unmute the system audio.".into(),
            });
        } else if lower.contains("volume") {
            let vol = extract_number(lower);
            if let Some(v) = vol {
                let v = v.min(100);
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Set volume to {}%", v)),
                    subtitle: "Adjust system volume".into(),
                    icon_char: "V".into(),
                    action_id: SharedString::from(format!("tool:Set the system volume to {}%.", v)),
                });
            } else {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: "Audio info".into(),
                    subtitle: "Show volume and audio device info".into(),
                    icon_char: "V".into(),
                    action_id: "tool:Show current audio volume and device info.".into(),
                });
            }
        } else {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Audio info".into(),
                subtitle: "Show volume and audio devices".into(),
                icon_char: "V".into(),
                action_id: "tool:Show current audio volume and device info.".into(),
            });
        }
    }

    // ── Screenshot ──
    if lower.contains("screenshot") || lower.contains("screen capture")
        || lower.starts_with("capture screen") || lower.starts_with("take a screen")
    {
        results.push(LensResult {
            result_type: "tool".into(),
            title: "Take screenshot".into(),
            subtitle: "Capture the current screen".into(),
            icon_char: "S".into(),
            action_id: "tool:Take a screenshot of the screen.".into(),
        });
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
            results.push(LensResult {
                result_type: "tool".into(),
                title: SharedString::from(format!("Calculate: {}", expr)),
                subtitle: "Evaluate expression".into(),
                icon_char: "=".into(),
                action_id: SharedString::from(format!("tool:Calculate: {}", expr)),
            });
        }
    }

    // ── Unit conversion ──
    if lower.starts_with("convert ") || lower.contains(" to ") && has_unit_keyword(lower) {
        results.push(LensResult {
            result_type: "tool".into(),
            title: SharedString::from(format!("Convert: {}", original)),
            subtitle: "Unit conversion".into(),
            icon_char: "=".into(),
            action_id: SharedString::from(format!("tool:{}", original)),
        });
    }

    // ── Git ──
    if lower.starts_with("git ") {
        let sub = &lower[4..];
        if sub.starts_with("status") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Git status".into(),
                subtitle: "Show working tree status".into(),
                icon_char: "G".into(),
                action_id: "tool:Show the git status of the current repository.".into(),
            });
        } else if sub.starts_with("log") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Git log".into(),
                subtitle: "Show recent commits".into(),
                icon_char: "G".into(),
                action_id: "tool:Show the recent git commit log.".into(),
            });
        } else if sub.starts_with("diff") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Git diff".into(),
                subtitle: "Show uncommitted changes".into(),
                icon_char: "G".into(),
                action_id: "tool:Show the current git diff of uncommitted changes.".into(),
            });
        } else if sub.starts_with("branch") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Git branches".into(),
                subtitle: "List branches".into(),
                icon_char: "G".into(),
                action_id: "tool:Show all git branches.".into(),
            });
        } else if sub.starts_with("clone") {
            let url = sub.strip_prefix("clone ").unwrap_or("").trim();
            if !url.is_empty() {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Git clone {}", url)),
                    subtitle: "Clone repository".into(),
                    icon_char: "G".into(),
                    action_id: SharedString::from(format!("tool:Clone the git repository: {}", url)),
                });
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
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Install {}", pkg)),
                    subtitle: "Install system package".into(),
                    icon_char: "P".into(),
                    action_id: SharedString::from(format!("tool:Install the package '{}'.", pkg)),
                });
            }
        } else if lower.starts_with("uninstall ") || lower.starts_with("remove package") {
            let pkg = lower
                .strip_prefix("uninstall ")
                .or_else(|| lower.strip_prefix("remove package "))
                .unwrap_or("")
                .trim();
            if !pkg.is_empty() {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Remove {}", pkg)),
                    subtitle: "Uninstall system package".into(),
                    icon_char: "P".into(),
                    action_id: SharedString::from(format!("tool:Remove the package '{}'.", pkg)),
                });
            }
        } else if let Some(pkg) = lower.strip_prefix("search package ").or_else(|| lower.strip_prefix("package search ")) {
            let pkg = pkg.trim();
            if !pkg.is_empty() {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Search packages: {}", pkg)),
                    subtitle: "Search available packages".into(),
                    icon_char: "P".into(),
                    action_id: SharedString::from(format!("tool:Search for packages matching '{}'.", pkg)),
                });
            }
        }
    }

    // ── Service management ──
    if lower.contains("service") || lower.starts_with("restart ")
        || lower.starts_with("stop ") && !lower.contains("timer")
        || lower.starts_with("start ") && !lower.contains("focus")
    {
        if lower.contains("list") || lower == "services" {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "List services".into(),
                subtitle: "Show running system services".into(),
                icon_char: "D".into(),
                action_id: "tool:List all running system services.".into(),
            });
        } else if lower.starts_with("restart ") {
            let svc = lower.strip_prefix("restart ").unwrap_or("").trim();
            if !svc.is_empty() && !svc.contains("service") {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Restart {}", svc)),
                    subtitle: "Restart system service".into(),
                    icon_char: "D".into(),
                    action_id: SharedString::from(format!("tool:Restart the service '{}'.", svc)),
                });
            }
        } else if let Some(rest) = lower.strip_prefix("service status ") {
            let svc = rest.trim();
            if !svc.is_empty() {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Status of {}", svc)),
                    subtitle: "Check service status".into(),
                    icon_char: "D".into(),
                    action_id: SharedString::from(format!("tool:Show the status of service '{}'.", svc)),
                });
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
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Kill {}", proc)),
                    subtitle: "Terminate process".into(),
                    icon_char: "X".into(),
                    action_id: SharedString::from(format!("tool:Kill the process '{}'.", proc)),
                });
            }
        } else {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Running processes".into(),
                subtitle: "List active processes".into(),
                icon_char: "P".into(),
                action_id: "tool:List running processes sorted by CPU usage.".into(),
            });
        }
    }

    // ── Disk / Storage ──
    if lower.contains("disk") || lower.contains("storage") || lower.contains("space left")
        || lower.contains("how much space") || lower.contains("dir size")
        || lower.contains("directory size") || lower.contains("mount")
    {
        if lower.contains("mount") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Mount info".into(),
                subtitle: "Show mounted filesystems".into(),
                icon_char: "H".into(),
                action_id: "tool:Show mounted filesystems and their info.".into(),
            });
        } else if lower.contains("dir") || lower.contains("directory") || lower.contains("folder size") {
            let path = extract_path_from_query(lower);
            let query = if path.is_empty() {
                "Show the size of my home directory.".to_string()
            } else {
                format!("Show the size of directory {}.", path)
            };
            results.push(LensResult {
                result_type: "tool".into(),
                title: SharedString::from(if path.is_empty() { "Directory size (~)".to_string() } else { format!("Size of {}", path) }),
                subtitle: "Calculate directory size".into(),
                icon_char: "H".into(),
                action_id: SharedString::from(format!("tool:{}", query)),
            });
        } else {
            results.push(LensResult {
                result_type: "find".into(),
                title: "Disk usage".into(),
                subtitle: "Show disk space for all partitions".into(),
                icon_char: "H".into(),
                action_id: "tool:Show disk space usage for all partitions.".into(),
            });
        }
    }

    // ── Display / Resolution ──
    if lower.contains("resolution") || lower.contains("display info")
        || lower.contains("monitors") || lower.contains("screen info")
    {
        if lower.contains("set") || lower.contains("change") {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Change resolution".into(),
                subtitle: "Set display resolution".into(),
                icon_char: "D".into(),
                action_id: SharedString::from(format!("tool:{}", original)),
            });
        } else {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Display info".into(),
                subtitle: "Show connected displays and resolutions".into(),
                icon_char: "D".into(),
                action_id: "tool:Show display info — connected monitors and resolutions.".into(),
            });
        }
    }

    // ── Wallpaper ──
    if lower.contains("wallpaper") || lower.contains("background") && lower.contains("change")
        || lower.contains("background") && lower.contains("set")
    {
        let path = extract_path_from_query(lower);
        if path.is_empty() {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "Set wallpaper".into(),
                subtitle: "Change desktop background".into(),
                icon_char: "I".into(),
                action_id: SharedString::from(format!("tool:{}", original)),
            });
        } else {
            results.push(LensResult {
                result_type: "tool".into(),
                title: SharedString::from(format!("Set wallpaper: {}", path)),
                subtitle: "Change desktop background".into(),
                icon_char: "I".into(),
                action_id: SharedString::from(format!("tool:Set the wallpaper to {}.", path)),
            });
        }
    }

    // ── Encoding / Base64 / JSON ──
    if lower.starts_with("base64 ") || lower.starts_with("url encode")
        || lower.starts_with("url decode") || lower.contains("format json")
        || lower.contains("pretty json") || lower.starts_with("encode ")
        || lower.starts_with("decode ")
    {
        results.push(LensResult {
            result_type: "tool".into(),
            title: SharedString::from(capitalize(original)),
            subtitle: "Encoding / formatting tool".into(),
            icon_char: "#".into(),
            action_id: SharedString::from(format!("tool:{}", original)),
        });
    }

    // ── Archive ──
    if lower.starts_with("extract ") || lower.starts_with("compress ")
        || lower.starts_with("unzip ") || lower.starts_with("untar ")
        || lower.contains("create archive") || lower.contains("make tar")
    {
        results.push(LensResult {
            result_type: "tool".into(),
            title: SharedString::from(capitalize(original)),
            subtitle: "Archive create/extract".into(),
            icon_char: "Z".into(),
            action_id: SharedString::from(format!("tool:{}", original)),
        });
    }

    // ── Window management ──
    if lower.contains("window") || lower.starts_with("close ") && !lower.contains("lens")
        || lower.starts_with("switch to ") || lower.starts_with("focus ")
    {
        if lower.contains("list") || lower == "windows" {
            results.push(LensResult {
                result_type: "tool".into(),
                title: "List windows".into(),
                subtitle: "Show all open windows".into(),
                icon_char: "W".into(),
                action_id: "tool:List all open windows.".into(),
            });
        } else if lower.starts_with("close ") {
            let target = lower.strip_prefix("close ").unwrap_or("").trim();
            if !target.is_empty() && target != "lens" {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Close {}", target)),
                    subtitle: "Close window".into(),
                    icon_char: "X".into(),
                    action_id: SharedString::from(format!("tool:Close the window titled '{}'.", target)),
                });
            }
        } else if lower.starts_with("switch to ") || lower.starts_with("focus ") {
            let target = lower
                .strip_prefix("switch to ")
                .or_else(|| lower.strip_prefix("focus "))
                .unwrap_or("")
                .trim();
            if !target.is_empty() && target != "mode" && target != "timer" {
                results.push(LensResult {
                    result_type: "tool".into(),
                    title: SharedString::from(format!("Focus {}", target)),
                    subtitle: "Bring window to front".into(),
                    icon_char: "W".into(),
                    action_id: SharedString::from(format!("tool:Focus the window titled '{}'.", target)),
                });
            }
        }
    }

    // ── Date/Time ──
    if lower.starts_with("what time") || lower.starts_with("what date")
        || lower.starts_with("what day") || lower.starts_with("how long until")
        || lower.starts_with("days until") || lower.starts_with("date calc")
    {
        results.push(LensResult {
            result_type: "tool".into(),
            title: SharedString::from(capitalize(original)),
            subtitle: "Date/time calculation".into(),
            icon_char: "T".into(),
            action_id: SharedString::from(format!("tool:{}", original)),
        });
    }

    // ── Network / Download ──
    if lower.starts_with("download ") || lower.starts_with("fetch ") {
        let target = lower
            .strip_prefix("download ")
            .or_else(|| lower.strip_prefix("fetch "))
            .unwrap_or("")
            .trim();
        if !target.is_empty() {
            results.push(LensResult {
                result_type: "tool".into(),
                title: SharedString::from(format!("Download {}", target)),
                subtitle: "Download file from URL".into(),
                icon_char: "D".into(),
                action_id: SharedString::from(format!("tool:Download the file from {}.", target)),
            });
        }
    }

    // ── File hash / diff / word count ──
    if lower.starts_with("hash ") || lower.starts_with("sha256 ")
        || lower.starts_with("diff ") || lower.starts_with("word count ")
        || lower.starts_with("wc ")
    {
        results.push(LensResult {
            result_type: "tool".into(),
            title: SharedString::from(capitalize(original)),
            subtitle: "Text/file utility".into(),
            icon_char: "#".into(),
            action_id: SharedString::from(format!("tool:{}", original)),
        });
    }

    // ── System info (extended) ──
    if lower.contains("battery") || lower.contains("memory") || lower.contains("ram")
        || lower.contains("uptime") || lower == "system info" || lower == "sysinfo"
        || lower.starts_with("system status")
    {
        results.push(LensResult {
            result_type: "tool".into(),
            title: "System info".into(),
            subtitle: "CPU, RAM, disk, uptime, kernel".into(),
            icon_char: "I".into(),
            action_id: "tool:Show detailed system info — CPU, RAM, disk, uptime, kernel.".into(),
        });
    }

    // ── Notification ──
    if lower.starts_with("notify ") || lower.starts_with("send notification") {
        let msg = lower
            .strip_prefix("notify ")
            .or_else(|| lower.strip_prefix("send notification "))
            .unwrap_or("")
            .trim();
        if !msg.is_empty() {
            results.push(LensResult {
                result_type: "tool".into(),
                title: SharedString::from(format!("Notify: {}", msg)),
                subtitle: "Send desktop notification".into(),
                icon_char: "N".into(),
                action_id: SharedString::from(format!("tool:Send a notification with the message '{}'.", msg)),
            });
        }
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
    has_digit && has_op && s.len() < 100
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
