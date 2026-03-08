//! .desktop file scanner — discovers installed apps from XDG application directories.
//!
//! Scans /usr/share/applications/ and ~/.local/share/applications/ for .desktop files.
//! Parses Name, Exec, Icon, Categories, Comment, and visibility flags.
//! Provides fuzzy search for Intent Lens integration.

use std::path::{Path, PathBuf};

/// A parsed .desktop entry.
#[derive(Debug, Clone)]
pub struct DesktopEntry {
    /// Display name (Name= field).
    pub name: String,
    /// Executable command (Exec= field, with field codes stripped).
    pub exec: String,
    /// Icon name or path (Icon= field).
    pub icon: String,
    /// Semicolon-separated categories (Categories= field).
    pub categories: String,
    /// Short description (Comment= field).
    pub comment: String,
    /// Desktop file basename without .desktop extension (used as app_id).
    pub app_id: String,
    /// Single-char icon for Lens display (derived from categories/name).
    pub icon_char: String,
}

/// Built-in Yantrik apps that appear in the app grid alongside system apps.
pub fn builtin_apps() -> Vec<DesktopEntry> {
    vec![
        DesktopEntry {
            name: "Terminal".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;TerminalEmulator;".into(), comment: "Shell terminal".into(),
            app_id: "terminal".into(), icon_char: ">_".into(),
        },
        DesktopEntry {
            name: "Files".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;FileManager;".into(), comment: "Browse files".into(),
            app_id: "files".into(), icon_char: "F".into(),
        },
        DesktopEntry {
            name: "Email".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Network;Email;".into(), comment: "AI-powered email client".into(),
            app_id: "email".into(), icon_char: "@".into(),
        },
        DesktopEntry {
            name: "Notes".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Office;TextEditor;".into(), comment: "Quick notes".into(),
            app_id: "notes".into(), icon_char: "\u{270E}".into(),
        },
        DesktopEntry {
            name: "Editor".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Development;TextEditor;".into(), comment: "Text editor with AI assist".into(),
            app_id: "editor".into(), icon_char: "\u{2261}".into(),
        },
        DesktopEntry {
            name: "Media Player".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "AudioVideo;Player;".into(), comment: "Music & media".into(),
            app_id: "media".into(), icon_char: "\u{266A}".into(),
        },
        DesktopEntry {
            name: "Bond".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Companion bond tracker".into(),
            app_id: "bond".into(), icon_char: "\u{2665}".into(),
        },
        DesktopEntry {
            name: "Personality".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Companion personality evolution".into(),
            app_id: "personality".into(), icon_char: "\u{2727}".into(),
        },
        DesktopEntry {
            name: "Memory".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Browse companion memories".into(),
            app_id: "memory".into(), icon_char: "\u{25C8}".into(),
        },
        DesktopEntry {
            name: "Notifications".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Notification center".into(),
            app_id: "notifications".into(), icon_char: "N".into(),
        },
        DesktopEntry {
            name: "System".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;Monitor;".into(), comment: "System dashboard".into(),
            app_id: "system".into(), icon_char: "\u{25C9}".into(),
        },
        DesktopEntry {
            name: "Settings".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Settings;".into(), comment: "Yantrik settings".into(),
            app_id: "settings".into(), icon_char: "\u{2699}".into(),
        },
        DesktopEntry {
            name: "About".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "System info".into(),
            app_id: "about".into(), icon_char: "\u{2139}".into(),
        },
        DesktopEntry {
            name: "Packages".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;PackageManager;".into(), comment: "Install and manage packages".into(),
            app_id: "packages".into(), icon_char: "P".into(),
        },
        DesktopEntry {
            name: "Network".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;Network;".into(), comment: "WiFi, Ethernet, Bluetooth, VPN, Firewall".into(),
            app_id: "network".into(), icon_char: "\u{25CE}".into(),
        },
        DesktopEntry {
            name: "System Monitor".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;Monitor;".into(), comment: "CPU, memory, disk, network, processes".into(),
            app_id: "sysmonitor".into(), icon_char: "\u{2699}".into(),
        },
        DesktopEntry {
            name: "Weather".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Weather forecast dashboard".into(),
            app_id: "weather".into(), icon_char: "\u{2602}".into(),
        },
        DesktopEntry {
            name: "Music".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "AudioVideo;Music;Player;".into(), comment: "Music library and player".into(),
            app_id: "music".into(), icon_char: "\u{266B}".into(),
        },
        DesktopEntry {
            name: "Downloads".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Network;FileTransfer;".into(), comment: "Download manager".into(),
            app_id: "downloads".into(), icon_char: "\u{2B07}".into(),
        },
        DesktopEntry {
            name: "Snippets".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Development;Utility;".into(), comment: "Code snippet manager".into(),
            app_id: "snippets".into(), icon_char: "<>".into(),
        },
        DesktopEntry {
            name: "Containers".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Development;System;".into(), comment: "Docker/Podman container manager".into(),
            app_id: "containers".into(), icon_char: "\u{2338}".into(),
        },
        DesktopEntry {
            name: "Devices".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;HardwareSettings;".into(), comment: "Hardware device dashboard".into(),
            app_id: "devices".into(), icon_char: "\u{2699}".into(),
        },
        DesktopEntry {
            name: "Permissions".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;Security;".into(), comment: "File & system permissions".into(),
            app_id: "permissions".into(), icon_char: "\u{2318}".into(),
        },
    ]
}

/// System apps that duplicate Yantrik built-ins or are noise in the launcher.
const HIDDEN_APP_IDS: &[&str] = &[
    "xfce4-about",        // About Xfce — we have our own About
    "xfce4-settings-manager", // Xfce Settings — we have Settings
    "thunar",             // Thunar — we have Files
    "thunar-settings",    // Thunar settings
    "thunar-bulk-rename", // Bulk rename — Thunar extension
    "Thunar-bulk-rename", // Alternate casing
    "xfce4-file-manager", // Another Thunar alias
    "foot-client",        // Foot Client — we have Terminal
    "footclient",         // Alternate ID
    "foot-server",        // Foot Server — not user-facing
    "foot",               // Foot — we have Terminal
    "xfce4-terminal",     // Xfce Terminal — we have Terminal
    "org.freedesktop.Xwayland", // Xwayland — not user-facing
    "mpv",                    // mpv — we have Media Player built-in
];

/// Scan all XDG application directories for .desktop files.
/// Returns built-in Yantrik apps first, then system apps sorted by name.
pub fn scan() -> Vec<DesktopEntry> {
    let mut entries = builtin_apps();
    let mut seen_ids: std::collections::HashSet<String> = entries.iter().map(|e| e.app_id.clone()).collect();

    // Pre-block hidden system app IDs
    for id in HIDDEN_APP_IDS {
        seen_ids.insert(id.to_string());
    }

    // XDG dirs: user-local first (overrides system), then system
    let dirs = app_dirs();

    for dir in &dirs {
        if !dir.is_dir() {
            continue;
        }
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }

            if let Some(de) = parse_desktop_file(&path) {
                if !seen_ids.contains(&de.app_id) {
                    seen_ids.insert(de.app_id.clone());
                    entries.push(de);
                }
            }
        }
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    tracing::info!(count = entries.len(), "Scanned .desktop files");
    entries
}

/// Search entries by query. Returns matches sorted by relevance.
/// Matches against name, app_id, exec, categories, and comment.
pub fn search<'a>(query: &str, entries: &'a [DesktopEntry]) -> Vec<&'a DesktopEntry> {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    let mut scored: Vec<(&DesktopEntry, u32)> = entries
        .iter()
        .filter_map(|e| {
            let score = match_score(e, &lower, &words);
            if score > 0 { Some((e, score)) } else { None }
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().take(6).map(|(e, _)| e).collect()
}

/// Compute a match score (0 = no match, higher = better).
fn match_score(entry: &DesktopEntry, query: &str, words: &[&str]) -> u32 {
    let name_lower = entry.name.to_lowercase();
    let app_id_lower = entry.app_id.to_lowercase();
    let exec_lower = entry.exec.to_lowercase();
    let cats_lower = entry.categories.to_lowercase();
    let comment_lower = entry.comment.to_lowercase();

    let mut score = 0u32;

    // Exact name match
    if name_lower == query {
        score += 100;
    }
    // Name starts with query
    else if name_lower.starts_with(query) {
        score += 80;
    }
    // Name contains query
    else if name_lower.contains(query) {
        score += 60;
    }

    // App ID match (e.g. "firefox" matches "firefox-esr")
    if app_id_lower.contains(query) {
        score += 50;
    }

    // Exec basename match
    let exec_basename = exec_lower.split('/').last().unwrap_or(&exec_lower);
    if exec_basename.contains(query) {
        score += 40;
    }

    // Category match (e.g. "editor" matches "TextEditor")
    if cats_lower.contains(query) {
        score += 20;
    }

    // Comment match
    if comment_lower.contains(query) {
        score += 10;
    }

    // Word-by-word matching (all words must appear somewhere)
    if words.len() > 1 && score == 0 {
        let all_text = format!("{} {} {} {} {}", name_lower, app_id_lower, exec_lower, cats_lower, comment_lower);
        if words.iter().all(|w| all_text.contains(w)) {
            score += 30;
        }
    }

    score
}

/// XDG application directories to scan.
fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // User-local (higher priority)
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
    }
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(data_home).join("applications"));
    }

    // System directories
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for dir in data_dirs.split(':') {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/share/applications"));
        dirs.push(PathBuf::from("/usr/local/share/applications"));
    }

    dirs
}

/// Parse a single .desktop file. Returns None if hidden, not an Application, or malformed.
fn parse_desktop_file(path: &Path) -> Option<DesktopEntry> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut name = String::new();
    let mut exec = String::new();
    let mut icon = String::new();
    let mut categories = String::new();
    let mut comment = String::new();
    let mut entry_type = String::new();
    let mut no_display = false;
    let mut hidden = false;
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let line = line.trim();

        // Track sections — only parse [Desktop Entry]
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }

        if !in_desktop_entry {
            continue;
        }

        // Skip comments
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "Name" => name = value.to_string(),
                "Exec" => exec = strip_field_codes(value),
                "Icon" => icon = value.to_string(),
                "Categories" => categories = value.to_string(),
                "Comment" => comment = value.to_string(),
                "Type" => entry_type = value.to_string(),
                "NoDisplay" => no_display = value == "true",
                "Hidden" => hidden = value == "true",
                _ => {}
            }
        }
    }

    // Filter: must be Application type, not hidden, have a name and exec
    if entry_type != "Application" || no_display || hidden || name.is_empty() || exec.is_empty() {
        return None;
    }

    // Derive app_id from filename
    let app_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Derive icon_char from categories or name
    let icon_char = derive_icon_char(&categories, &name);

    Some(DesktopEntry {
        name,
        exec,
        icon,
        categories,
        comment,
        app_id,
        icon_char,
    })
}

/// Strip .desktop Exec field codes (%f, %F, %u, %U, %d, %D, %n, %N, %i, %c, %k).
fn strip_field_codes(exec: &str) -> String {
    let mut result = String::new();
    let mut chars = exec.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            // Skip the next char (field code)
            if let Some(&next) = chars.peek() {
                if "fFuUdDnNickv".contains(next) {
                    chars.next();
                    continue;
                }
            }
        }
        result.push(c);
    }

    result.trim().to_string()
}

/// Derive a single icon character from categories or app name.
fn derive_icon_char(categories: &str, name: &str) -> String {
    let cats = categories.to_lowercase();
    let name_lower = name.to_lowercase();

    if cats.contains("terminal") || cats.contains("system") {
        ">_".to_string()
    } else if cats.contains("webbrowser") || cats.contains("browser") {
        // Distinguish browsers by name
        if name_lower.contains("firefox") {
            "\u{2740}".to_string() // ❀ flower for Firefox
        } else {
            "W".to_string()
        }
    } else if cats.contains("filemanager") || cats.contains("filesystem") {
        "F".to_string()
    } else if cats.contains("texteditor") || cats.contains("editor") {
        "E".to_string()
    } else if cats.contains("game") {
        "G".to_string()
    } else if cats.contains("audio") || cats.contains("music") || cats.contains("player") {
        "M".to_string()
    } else if cats.contains("video") {
        "V".to_string()
    } else if cats.contains("graphics") || cats.contains("image") {
        "I".to_string()
    } else if cats.contains("office") || cats.contains("document") {
        "D".to_string()
    } else if cats.contains("network") || cats.contains("email") || cats.contains("chat") {
        "N".to_string()
    } else if cats.contains("settings") || cats.contains("preferences") {
        "*".to_string()
    } else if cats.contains("development") || cats.contains("ide") {
        "<>".to_string()
    } else {
        // First letter of name
        name.chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}
