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

/// Scan all XDG application directories for .desktop files.
/// Returns a deduplicated list sorted by name.
pub fn scan() -> Vec<DesktopEntry> {
    let mut entries = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

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

    if cats.contains("terminal") || cats.contains("system") {
        ">_".to_string()
    } else if cats.contains("webbrowser") || cats.contains("browser") {
        "W".to_string()
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
