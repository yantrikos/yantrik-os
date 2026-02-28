//! File browser — directory listing and file operations.
//!
//! Provides synchronous directory listing for the file browser screen (screen 8).
//! Files are sorted: directories first, then alphabetically.

use std::path::{Path, PathBuf};

/// A single directory entry for display.
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size_text: String,
    pub modified_text: String,
    pub icon_char: String,
}

/// Expand ~ to $HOME.
pub fn expand_home(path: &str) -> PathBuf {
    if path.starts_with("~/") || path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or(""));
        }
    }
    PathBuf::from(path)
}

/// Collapse $HOME back to ~ for display.
pub fn collapse_home(path: &Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        let home_path = Path::new(&home);
        if let Ok(relative) = path.strip_prefix(home_path) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}

/// List the contents of a directory.
/// Returns entries sorted: directories first, then alphabetically.
pub fn list_dir(path: &str) -> Vec<DirEntry> {
    let expanded = expand_home(path);

    let read_dir = match std::fs::read_dir(&expanded) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!(path = %expanded.display(), error = %e, "Failed to read directory");
            return Vec::new();
        }
    };

    let mut entries: Vec<DirEntry> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| {
            let meta = e.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let name = e.file_name().to_string_lossy().to_string();

            let size_text = if is_dir {
                String::new()
            } else {
                meta.as_ref()
                    .map(|m| format_size(m.len()))
                    .unwrap_or_default()
            };

            let modified_text = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(format_modified)
                .unwrap_or_default();

            let icon_char = if is_dir {
                "📁".to_string()
            } else {
                file_icon(&name)
            };

            DirEntry {
                name,
                is_dir,
                size_text,
                modified_text,
                icon_char,
            }
        })
        .collect();

    // Sort: dirs first, then alpha (case-insensitive)
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    entries
}

/// Get the parent directory path.
pub fn parent_path(path: &str) -> String {
    let expanded = expand_home(path);
    expanded
        .parent()
        .map(|p| collapse_home(p))
        .unwrap_or_else(|| "/".to_string())
}

/// Resolve a child path (for navigating into a subdirectory).
pub fn child_path(current: &str, child_name: &str) -> String {
    let expanded = expand_home(current);
    let child = expanded.join(child_name);
    collapse_home(&child)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_modified(time: std::time::SystemTime) -> String {
    let duration = time
        .elapsed()
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));
    let secs = duration.as_secs();

    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else if secs < 86400 * 30 {
        format!("{}d", secs / 86400)
    } else {
        format!("{}mo", secs / (86400 * 30))
    }
}

fn file_icon(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".rs") || lower.ends_with(".py") || lower.ends_with(".js")
        || lower.ends_with(".ts") || lower.ends_with(".c") || lower.ends_with(".h")
        || lower.ends_with(".go") || lower.ends_with(".java")
    {
        "◇".to_string()
    } else if lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".log") {
        "≡".to_string()
    } else if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")
        || lower.ends_with(".gif") || lower.ends_with(".svg") || lower.ends_with(".webp")
    {
        "▣".to_string()
    } else if lower.ends_with(".mp3") || lower.ends_with(".wav") || lower.ends_with(".flac")
        || lower.ends_with(".ogg")
    {
        "♪".to_string()
    } else if lower.ends_with(".mp4") || lower.ends_with(".mkv") || lower.ends_with(".avi")
        || lower.ends_with(".webm")
    {
        "▶".to_string()
    } else if lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".7z")
        || lower.ends_with(".rar") || lower.ends_with(".deb")
    {
        "▤".to_string()
    } else if lower.ends_with(".pdf") {
        "▧".to_string()
    } else if lower.ends_with(".toml") || lower.ends_with(".yaml") || lower.ends_with(".yml")
        || lower.ends_with(".json") || lower.ends_with(".xml")
    {
        "⚙".to_string()
    } else if lower.starts_with('.') {
        "·".to_string()
    } else {
        "□".to_string()
    }
}
