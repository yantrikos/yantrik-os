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

/// List the contents of a directory (hides dotfiles by default).
pub fn list_dir(path: &str) -> Vec<DirEntry> {
    list_dir_filtered(path, false)
}

/// List directory contents with optional hidden file display and name filter.
pub fn list_dir_filtered(path: &str, show_hidden: bool) -> Vec<DirEntry> {
    list_dir_full(path, show_hidden, "", "name", true)
}

/// List directory contents with full options: hidden files, name filter, sort field/direction.
pub fn list_dir_full(
    path: &str,
    show_hidden: bool,
    name_filter: &str,
    sort_field: &str,
    sort_ascending: bool,
) -> Vec<DirEntry> {
    let expanded = expand_home(path);
    let filter_lower = name_filter.to_lowercase();

    let read_dir = match std::fs::read_dir(&expanded) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!(path = %expanded.display(), error = %e, "Failed to read directory");
            return Vec::new();
        }
    };

    let mut entries: Vec<DirEntry> = read_dir
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let hidden_ok = show_hidden || !name.starts_with('.');
            let filter_ok = filter_lower.is_empty() || name.to_lowercase().contains(&filter_lower);
            hidden_ok && filter_ok
        })
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

    sort_entries(&mut entries, sort_field, sort_ascending);

    entries
}

/// Sort entries by field. Directories are always first.
pub fn sort_entries(entries: &mut Vec<DirEntry>, field: &str, ascending: bool) {
    entries.sort_by(|a, b| {
        // Dirs always first
        let dir_cmp = b.is_dir.cmp(&a.is_dir);
        if dir_cmp != std::cmp::Ordering::Equal {
            return dir_cmp;
        }
        let ord = match field {
            "size" => {
                let sa = parse_size_bytes(&a.size_text);
                let sb = parse_size_bytes(&b.size_text);
                sa.cmp(&sb)
            }
            "modified" => a.modified_text.cmp(&b.modified_text),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if ascending { ord } else { ord.reverse() }
    });
}

/// Parse a human-readable size string back to bytes for comparison.
fn parse_size_bytes(s: &str) -> u64 {
    if s.is_empty() {
        return 0;
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 2 {
        return 0;
    }
    let num: f64 = parts[0].parse().unwrap_or(0.0);
    match parts[1] {
        "B" => num as u64,
        "KB" => (num * 1024.0) as u64,
        "MB" => (num * 1024.0 * 1024.0) as u64,
        "GB" => (num * 1024.0 * 1024.0 * 1024.0) as u64,
        _ => 0,
    }
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

// ── File operations ──

/// Split a display path into breadcrumb segments.
/// e.g. "~/Documents/code" → [("~", "~"), ("Documents", "~/Documents"), ("code", "~/Documents/code")]
pub fn breadcrumb_segments(path: &str) -> Vec<(String, String)> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return vec![("/".to_string(), "/".to_string())];
    }
    let mut segments = Vec::new();
    let mut accumulated = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 && *part == "~" {
            accumulated = "~".to_string();
        } else if i == 0 {
            accumulated = format!("/{}", part);
        } else {
            accumulated = format!("{}/{}", accumulated, part);
        }
        segments.push((part.to_string(), accumulated.clone()));
    }
    segments
}

/// Delete a file or empty directory.
pub fn delete_entry(dir: &str, name: &str) -> Result<(), String> {
    let expanded = expand_home(dir);
    let target = expanded.join(name);
    if !target.exists() {
        return Err("File not found".to_string());
    }
    // Safety: don't delete outside home
    if let Ok(home) = std::env::var("HOME") {
        if !target.starts_with(&home) && !target.starts_with("/tmp") {
            return Err("Cannot delete files outside home directory".to_string());
        }
    }
    if target.is_dir() {
        std::fs::remove_dir_all(&target).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(&target).map_err(|e| e.to_string())
    }
}

/// Rename a file or directory.
pub fn rename_entry(dir: &str, old_name: &str, new_name: &str) -> Result<(), String> {
    if new_name.is_empty() || new_name.contains('/') || new_name.contains('\0') {
        return Err("Invalid name".to_string());
    }
    let expanded = expand_home(dir);
    let src = expanded.join(old_name);
    let dst = expanded.join(new_name);
    if !src.exists() {
        return Err("Source not found".to_string());
    }
    if dst.exists() {
        return Err("Name already exists".to_string());
    }
    std::fs::rename(&src, &dst).map_err(|e| e.to_string())
}

/// Create a new directory.
pub fn create_folder(dir: &str, name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\0') {
        return Err("Invalid folder name".to_string());
    }
    let expanded = expand_home(dir);
    let target = expanded.join(name);
    if target.exists() {
        return Err("Already exists".to_string());
    }
    std::fs::create_dir(&target).map_err(|e| e.to_string())
}

/// Copy a file or directory into a destination directory.
pub fn copy_entry(src_dir: &str, name: &str, dst_dir: &str) -> Result<(), String> {
    let src = expand_home(src_dir).join(name);
    let dst = expand_home(dst_dir).join(name);
    if !src.exists() {
        return Err("Source not found".to_string());
    }
    if src.is_dir() {
        copy_dir_recursive(&src, &dst)
    } else {
        std::fs::copy(&src, &dst).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Move a file or directory into a destination directory.
pub fn move_entry(src_dir: &str, name: &str, dst_dir: &str) -> Result<(), String> {
    let src = expand_home(src_dir).join(name);
    let dst = expand_home(dst_dir).join(name);
    if !src.exists() {
        return Err("Source not found".to_string());
    }
    std::fs::rename(&src, &dst).map_err(|e| {
        // rename fails across filesystems — fall back to copy+delete
        if src.is_dir() {
            if let Err(ce) = copy_dir_recursive(&src, &dst) {
                return format!("Move failed: {}, copy fallback failed: {}", e, ce);
            }
            if let Err(de) = std::fs::remove_dir_all(&src) {
                return format!("Copied but failed to remove source: {}", de);
            }
        } else {
            if let Err(ce) = std::fs::copy(&src, &dst) {
                return format!("Move failed: {}, copy fallback failed: {}", e, ce);
            }
            if let Err(de) = std::fs::remove_file(&src) {
                return format!("Copied but failed to remove source: {}", de);
            }
        }
        String::new() // success via fallback
    }).and_then(|_| Ok(()))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());
        if child_src.is_dir() {
            copy_dir_recursive(&child_src, &child_dst)?;
        } else {
            std::fs::copy(&child_src, &child_dst).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ── File details ──

/// Detailed file information for the details panel.
pub struct FileDetail {
    pub name: String,
    pub file_type: String,
    pub size_text: String,
    pub modified_text: String,
    pub path_text: String,
    pub permissions: String,
    pub preview_text: String,
    pub is_text_file: bool,
    pub icon_char: String,
}

/// Get detailed information about a file for the details panel.
pub fn get_file_details(dir: &str, name: &str) -> FileDetail {
    let expanded = expand_home(dir);
    let path = expanded.join(name);
    let meta = std::fs::metadata(&path).ok();

    let icon_char = file_icon(name);
    let file_type = file_type_name(name);

    let size_text = meta
        .as_ref()
        .filter(|m| !m.is_dir())
        .map(|m| format_size(m.len()))
        .unwrap_or_default();

    let modified_text = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(format_modified_full)
        .unwrap_or_default();

    let permissions = format_file_permissions(&meta);

    let is_text = is_text_extension(name);
    let preview_text = if is_text {
        read_preview(&path, 20)
    } else {
        String::new()
    };

    FileDetail {
        name: name.to_string(),
        file_type,
        size_text,
        modified_text,
        path_text: collapse_home(&path),
        permissions,
        is_text_file: is_text && !preview_text.is_empty(),
        preview_text,
        icon_char,
    }
}

fn file_type_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let type_name = if lower.ends_with(".rs") {
        "Rust Source"
    } else if lower.ends_with(".py") {
        "Python Script"
    } else if lower.ends_with(".js") {
        "JavaScript"
    } else if lower.ends_with(".ts") {
        "TypeScript"
    } else if lower.ends_with(".c") {
        "C Source"
    } else if lower.ends_with(".h") {
        "C Header"
    } else if lower.ends_with(".go") {
        "Go Source"
    } else if lower.ends_with(".java") {
        "Java Source"
    } else if lower.ends_with(".txt") {
        "Text File"
    } else if lower.ends_with(".md") {
        "Markdown"
    } else if lower.ends_with(".log") {
        "Log File"
    } else if lower.ends_with(".png") {
        "PNG Image"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "JPEG Image"
    } else if lower.ends_with(".gif") {
        "GIF Image"
    } else if lower.ends_with(".svg") {
        "SVG Image"
    } else if lower.ends_with(".webp") {
        "WebP Image"
    } else if lower.ends_with(".mp3") {
        "MP3 Audio"
    } else if lower.ends_with(".wav") {
        "WAV Audio"
    } else if lower.ends_with(".flac") {
        "FLAC Audio"
    } else if lower.ends_with(".ogg") {
        "OGG Audio"
    } else if lower.ends_with(".mp4") {
        "MP4 Video"
    } else if lower.ends_with(".mkv") {
        "MKV Video"
    } else if lower.ends_with(".avi") {
        "AVI Video"
    } else if lower.ends_with(".webm") {
        "WebM Video"
    } else if lower.ends_with(".zip") {
        "ZIP Archive"
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "Gzipped Tarball"
    } else if lower.ends_with(".7z") {
        "7-Zip Archive"
    } else if lower.ends_with(".rar") {
        "RAR Archive"
    } else if lower.ends_with(".deb") {
        "Debian Package"
    } else if lower.ends_with(".pdf") {
        "PDF Document"
    } else if lower.ends_with(".toml") {
        "TOML Config"
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        "YAML Config"
    } else if lower.ends_with(".json") {
        "JSON"
    } else if lower.ends_with(".xml") {
        "XML"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "HTML"
    } else if lower.ends_with(".css") {
        "CSS Stylesheet"
    } else if lower.ends_with(".sh") {
        "Shell Script"
    } else if lower.ends_with(".csv") {
        "CSV Data"
    } else if lower.starts_with('.') {
        "Hidden File"
    } else {
        "File"
    };
    type_name.to_string()
}

fn format_modified_full(time: std::time::SystemTime) -> String {
    let duration = time.elapsed().unwrap_or_default();
    let secs = duration.as_secs();
    if secs < 60 {
        "Just now".to_string()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 { "1 minute ago".to_string() } else { format!("{} minutes ago", m) }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 { "1 hour ago".to_string() } else { format!("{} hours ago", h) }
    } else if secs < 86400 * 30 {
        let d = secs / 86400;
        if d == 1 { "Yesterday".to_string() } else { format!("{} days ago", d) }
    } else if secs < 86400 * 365 {
        let mo = secs / (86400 * 30);
        if mo == 1 { "1 month ago".to_string() } else { format!("{} months ago", mo) }
    } else {
        let y = secs / (86400 * 365);
        if y == 1 { "1 year ago".to_string() } else { format!("{} years ago", y) }
    }
}

fn format_file_permissions(meta: &Option<std::fs::Metadata>) -> String {
    #[cfg(unix)]
    {
        meta.as_ref()
            .map(|m| {
                use std::os::unix::fs::PermissionsExt;
                let mode = m.permissions().mode();
                let mut s = String::with_capacity(9);
                for shift in [6u32, 3, 0] {
                    let bits = (mode >> shift) & 0o7;
                    s.push(if bits & 4 != 0 { 'r' } else { '-' });
                    s.push(if bits & 2 != 0 { 'w' } else { '-' });
                    s.push(if bits & 1 != 0 { 'x' } else { '-' });
                }
                s
            })
            .unwrap_or_default()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        String::new()
    }
}

fn is_text_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    let text_exts = [
        ".txt", ".md", ".log", ".rs", ".py", ".js", ".ts", ".c", ".h",
        ".go", ".java", ".toml", ".yaml", ".yml", ".json", ".xml",
        ".sh", ".css", ".html", ".htm", ".csv", ".ini", ".cfg",
        ".conf", ".env", ".makefile", ".dockerfile",
    ];
    text_exts.iter().any(|ext| lower.ends_with(ext))
        || lower == "makefile"
        || lower == "dockerfile"
        || lower == ".gitignore"
        || lower == ".dockerignore"
}

fn read_preview(path: &Path, max_lines: usize) -> String {
    use std::io::{BufRead, BufReader};
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .take(max_lines)
        .filter_map(|l| l.ok())
        .map(|l| if l.len() > 120 { format!("{}...", &l[..117]) } else { l })
        .collect();
    lines.join("\n")
}

/// Detect the project type of a directory by checking for marker files.
/// Returns a short label like "Rust project", "Node.js project", etc.
/// Returns empty string if no project type is detected.
pub fn detect_project_type(path: &str) -> String {
    let expanded = expand_home(path);

    // Ordered by specificity — first match wins
    let markers: &[(&str, &str)] = &[
        ("Cargo.toml", "Rust project"),
        ("package.json", "Node.js project"),
        ("pyproject.toml", "Python project"),
        ("setup.py", "Python project"),
        ("requirements.txt", "Python project"),
        ("go.mod", "Go project"),
        ("pom.xml", "Java project"),
        ("build.gradle", "Gradle project"),
        ("CMakeLists.txt", "CMake project"),
        ("Makefile", "C/C++ project"),
        ("composer.json", "PHP project"),
        ("Gemfile", "Ruby project"),
        ("mix.exs", "Elixir project"),
        ("deno.json", "Deno project"),
        ("flake.nix", "Nix project"),
        ("Dockerfile", "Docker project"),
        ("docker-compose.yml", "Docker Compose"),
        (".git", "Git repository"),
    ];

    for (marker, label) in markers {
        if expanded.join(marker).exists() {
            return label.to_string();
        }
    }

    String::new()
}
