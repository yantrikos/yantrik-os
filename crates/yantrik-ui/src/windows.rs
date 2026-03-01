//! Window listing via `wlrctl toplevel list`.
//! Parses output into WindowEntry structs with app_id heuristics.

/// A running window on the Wayland compositor.
pub struct WindowEntry {
    pub title: String,
    pub app_id: String,
    pub icon_char: String,
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
            WindowEntry {
                title,
                app_id,
                icon_char,
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
