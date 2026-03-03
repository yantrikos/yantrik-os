//! Window listing via `wlrctl toplevel list`.
//! Parses output into WindowEntry structs with app_id heuristics.

/// A running window on the Wayland compositor.
pub struct WindowEntry {
    pub title: String,
    pub app_id: String,
    pub icon_char: String,
    pub subtitle: String,
}

/// List all open windows via wlrctl.
pub fn list_windows() -> Vec<WindowEntry> {
    let output = match std::process::Command::new("wlrctl")
        .args(["toplevel", "list"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let title = line.trim().to_string();
            let app_id = derive_app_id(&title);
            let icon_char = icon_for_app(&app_id).to_string();
            let subtitle = derive_context(&title, &app_id);
            WindowEntry {
                title,
                app_id,
                icon_char,
                subtitle,
            }
        })
        .collect()
}

/// Derive a normalized app_id from a window title.
fn derive_app_id(title: &str) -> String {
    let lower = title.to_lowercase();
    if lower.contains("foot") || lower.contains("terminal") {
        "terminal".to_string()
    } else if lower.contains("firefox") || lower.contains("chromium") || lower.contains("browser")
    {
        "browser".to_string()
    } else if lower.contains("file") || lower.contains("pcmanfm") || lower.contains("thunar") {
        "files".to_string()
    } else if lower.contains("yantrik") {
        "yantrik".to_string()
    } else {
        // Use first word as app_id
        lower
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .to_string()
    }
}

/// Map app_id to a single-char icon.
pub fn icon_for_app(app_id: &str) -> &'static str {
    match app_id {
        "terminal" => ">_",
        "browser" => "W",
        "files" => "F",
        "yantrik" => "Y",
        _ => "?",
    }
}

/// Derive a contextual subtitle from a window title.
/// Terminal: extract CWD from "user@host:/path" pattern.
/// Browser: extract site name from "Page Title - Site" pattern.
/// Files: extract current directory.
fn derive_context(title: &str, app_id: &str) -> String {
    match app_id {
        "terminal" => {
            // Terminals often show "user@host:path" or just "foot" or "bash"
            if let Some(idx) = title.find(':') {
                let path = title[idx + 1..].trim();
                if !path.is_empty() {
                    return path.to_string();
                }
            }
            String::new()
        }
        "browser" => {
            // Browser titles: "Page Title - Site Name" or "Page Title — Firefox"
            let sep = if title.contains(" - ") {
                " - "
            } else if title.contains(" — ") {
                " — "
            } else {
                return String::new();
            };
            // Take the last segment as the site/app name
            title.rsplit(sep).next()
                .filter(|s| !s.eq_ignore_ascii_case("firefox") && !s.eq_ignore_ascii_case("chromium"))
                .unwrap_or("")
                .to_string()
        }
        "files" => {
            // File managers often show the current directory in the title
            if title.contains('/') {
                title.rsplit('/').next().unwrap_or("").to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}
