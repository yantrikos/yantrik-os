//! File browser — directory listing and file operations.
//!
//! Provides synchronous directory listing for the file browser screen (screen 8).
//! Files are sorted: directories first, then alphabetically.

use std::path::{Path, PathBuf};

/// A single directory entry for display.
#[derive(Clone)]
pub struct DirEntry {
    pub size_bytes: u64,
    pub modified: std::time::SystemTime,
    pub name: String,
    pub is_dir: bool,
    pub size_text: String,
    pub modified_text: String,
    pub icon_char: String,
    pub selected: bool,
    /// For a folder, how many entries it holds, once [`count_folders`] has looked. `None` for a
    /// file, and for a folder nobody has counted yet.
    pub items: Option<ItemCount>,
}

// ── What a folder holds, and when it changed ──
//
// The folder tiles say "12 items · 2 h ago" (desk-and-mind, "Files"). Both halves are read off
// the disk for every tile, and a half that could not be read says so: a folder the shell may
// not open is not an empty folder, and drawing it as "0 items" would be the same lie #131 took
// out of the mail folder list.

/// How many entries a folder holds, as far as it could be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemCount {
    /// Every entry, and the ones whose names do not start with a dot.
    Known { all: usize, visible: usize },
    /// Not counted, and why — the OS refused, or a budget ran out. Never shown as zero.
    Unknown(String),
}

impl ItemCount {
    /// The number to show beside the folder: the entries you would see on opening it, with
    /// hidden files shown or not. `None` when the folder was not counted.
    pub fn shown(&self, show_hidden: bool) -> Option<usize> {
        match self {
            ItemCount::Known { all, visible } => Some(if show_hidden { *all } else { *visible }),
            ItemCount::Unknown(_) => None,
        }
    }
}

/// Past this many entries a count stops, and says it stopped rather than print a number that
/// is only a floor.
pub const COUNT_CAP: usize = 100_000;

/// How many folders one listing counts. Each count is a `read_dir` — names only, no `stat` —
/// which is nothing for a home folder's dozen and real time for a `node_modules` with thousands
/// of packages. The rest are marked as not counted, with this reason, rather than guessed.
pub const FOLDER_COUNT_BUDGET: usize = 400;

/// Count the entries in one folder. Reads names only.
pub fn count_items(dir: &Path) -> ItemCount {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) => return ItemCount::Unknown(io_reason(&e)),
    };
    let (mut all, mut visible) = (0usize, 0usize);
    for entry in read {
        match entry {
            Ok(entry) => {
                all += 1;
                if !entry.file_name().to_string_lossy().starts_with('.') {
                    visible += 1;
                }
            }
            Err(e) => return ItemCount::Unknown(io_reason(&e)),
        }
        if all >= COUNT_CAP {
            return ItemCount::Unknown(format!("more than {COUNT_CAP} entries; not counted to the end"));
        }
    }
    ItemCount::Known { all, visible }
}

/// Count every folder in a listing of `dir`, up to [`FOLDER_COUNT_BUDGET`] of them.
///
/// `stop` is asked between folders, so a listing that has been superseded (the person clicked
/// somewhere else) stops counting for a view nobody will see.
pub fn count_folders(dir: &Path, entries: &mut [DirEntry], stop: &dyn Fn() -> bool) {
    let mut counted = 0usize;
    for entry in entries.iter_mut().filter(|e| e.is_dir) {
        if stop() {
            return;
        }
        entry.items = Some(if counted < FOLDER_COUNT_BUDGET {
            counted += 1;
            count_items(&dir.join(&entry.name))
        } else {
            ItemCount::Unknown(format!(
                "not counted: this folder holds more than {FOLDER_COUNT_BUDGET} folders"
            ))
        });
    }
}

/// An I/O error in the words a tile has room for.
fn io_reason(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "permission denied".into(),
        std::io::ErrorKind::NotFound => "no longer there".into(),
        _ => e.to_string(),
    }
}

/// "2 h ago": how long before `now` a thing changed, short enough for a tile.
///
/// An unreadable time arrives as the epoch (see `list_dir_checked`) and comes back as "", so a
/// tile says nothing rather than "56 y ago". A time in the future — a clock that moved — is
/// "just now", not a negative age.
pub fn ago(modified: std::time::SystemTime, now: std::time::SystemTime) -> String {
    if modified == std::time::UNIX_EPOCH {
        return String::new();
    }
    let secs = now.duration_since(modified).map(|d| d.as_secs()).unwrap_or(0);
    const MIN: u64 = 60;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    match secs {
        s if s < MIN => "just now".into(),
        s if s < HOUR => format!("{} min ago", s / MIN),
        s if s < DAY => format!("{} h ago", s / HOUR),
        s if s < 30 * DAY => format!("{} d ago", s / DAY),
        s if s < 365 * DAY => format!("{} mo ago", s / (30 * DAY)),
        s => format!("{} y ago", s / (365 * DAY)),
    }
}

/// [`ago`], from now.
pub fn changed_text(modified: std::time::SystemTime) -> String {
    ago(modified, std::time::SystemTime::now())
}

/// The files in a folder that changed most recently, newest first: the row under the grid.
///
/// Files only — a folder's time moves when anything is added to it, which is not "recent work"
/// — and only ones whose time could be read. Hidden files count only when they are shown.
/// Ties go by name, so the row does not reshuffle between two refreshes of the same folder.
pub fn most_recent<'a, I>(entries: I, show_hidden: bool, n: usize) -> Vec<&'a DirEntry>
where
    I: IntoIterator<Item = &'a DirEntry>,
{
    let mut files: Vec<&DirEntry> = entries
        .into_iter()
        .filter(|e| !e.is_dir && e.modified != std::time::UNIX_EPOCH)
        .filter(|e| show_hidden || !e.name.starts_with('.'))
        .collect();
    files.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    files.truncate(n);
    files
}

// ── Places ──

/// One entry in the Files sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// What the sidebar draws its icon by: home, documents, downloads, pictures, music, videos,
    /// projects.
    pub id: &'static str,
    pub label: String,
    /// As the address bar shows it: `~`, `~/Documents`.
    pub path: String,
}

/// The sidebar's places for a home folder: Home, then each standard folder that exists.
///
/// A place that is not on disk is not listed. Offering "Music" on a machine with no ~/Music
/// would be a button whose only result is an error, so the list is whatever this home really
/// has. The standard folders are read from `~/.config/user-dirs.dirs` when it names them — on
/// a German desktop Documents is `~/Dokumente` — and fall back to the English names.
/// Projects is not an XDG folder; it is listed when `~/Projects` or `~/projects` exists.
pub fn places(home: &Path) -> Vec<Place> {
    let named = user_dirs(home);
    let mut out = vec![Place { id: "home", label: "Home".into(), path: "~".into() }];
    let standard: [(&str, &str, &str); 5] = [
        ("documents", "Documents", "XDG_DOCUMENTS_DIR"),
        ("downloads", "Downloads", "XDG_DOWNLOAD_DIR"),
        ("pictures", "Pictures", "XDG_PICTURES_DIR"),
        ("music", "Music", "XDG_MUSIC_DIR"),
        ("videos", "Videos", "XDG_VIDEOS_DIR"),
    ];
    for (id, label, key) in standard {
        let dir = named
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| home.join(label));
        // XDG points an unused folder at $HOME itself; that is not a place of its own.
        if dir != home && dir.is_dir() {
            out.push(Place { id, label: label.into(), path: display_under(home, &dir) });
        }
    }
    if let Some(dir) = ["Projects", "projects"].iter().map(|n| home.join(n)).find(|d| d.is_dir()) {
        out.push(Place { id: "projects", label: "Projects".into(), path: display_under(home, &dir) });
    }
    out
}

/// `~/x` for a folder under `home`, the full path otherwise.
fn display_under(home: &Path, dir: &Path) -> String {
    match dir.strip_prefix(home) {
        Ok(rel) if rel.as_os_str().is_empty() => "~".into(),
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => dir.display().to_string(),
    }
}

/// `XDG_*_DIR="$HOME/…"` lines from `~/.config/user-dirs.dirs`, resolved against `home`.
fn user_dirs(home: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(home.join(".config/user-dirs.dirs")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            if key.starts_with('#') {
                return None;
            }
            let value = value.trim().trim_matches('"');
            let path = if let Some(rest) = value.strip_prefix("$HOME") {
                home.join(rest.trim_start_matches('/'))
            } else if value.starts_with('/') {
                PathBuf::from(value)
            } else {
                return None;
            };
            Some((key.trim().to_string(), path))
        })
        .collect()
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
            return if relative.as_os_str().is_empty() { "~".into() } else { format!("~/{}", relative.display()) };
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
    list_dir_checked(path, show_hidden, name_filter, sort_field, sort_ascending).unwrap_or_default()
}

pub fn list_dir_checked(
    path: &str,
    show_hidden: bool,
    name_filter: &str,
    sort_field: &str,
    sort_ascending: bool,
) -> Result<Vec<DirEntry>, String> {
    let expanded = expand_home(path);
    let filter = name_filter.to_lowercase();
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&expanded).map_err(|e| format!("{}: {e}", expanded.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().into_string().map_err(|_| {
            "This folder has non-UTF-8 filenames. Use Terminal to rename them before browsing here."
        })?;
        if (!show_hidden && name.starts_with('.')) || !name.to_lowercase().contains(&filter) {
            continue;
        }
        let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| format!("{name}: {e}"))?;
        // A link to a directory can be entered explicitly; recursive operations never follow it.
        let is_dir = if meta.file_type().is_symlink() {
            entry.path().is_dir()
        } else {
            meta.is_dir()
        };
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        entries.push(DirEntry {
            size_bytes: meta.len(),
            modified,
            name: name.clone(),
            is_dir,
            size_text: if is_dir {
                String::new()
            } else {
                format_size(meta.len())
            },
            modified_text: format_modified(modified),
            icon_char: if is_dir {
                "folder".into()
            } else {
                file_icon(&name)
            },
            selected: false,
            items: None,
        });
        if entries.len() > 50000 {
            return Err(
                "This folder has more than 50,000 items. Use Terminal to narrow it down.".into(),
            );
        }
    }
    sort_entries(&mut entries, sort_field, sort_ascending);
    Ok(entries)
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
            "size" => a.size_bytes.cmp(&b.size_bytes),
            "modified" => a.modified.cmp(&b.modified),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if ascending {
            ord
        } else {
            ord.reverse()
        }
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
    if lower.ends_with(".rs")
        || lower.ends_with(".py")
        || lower.ends_with(".js")
        || lower.ends_with(".ts")
        || lower.ends_with(".c")
        || lower.ends_with(".h")
        || lower.ends_with(".go")
        || lower.ends_with(".java")
    {
        "◇".to_string()
    } else if lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".log") {
        "≡".to_string()
    } else if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".svg")
        || lower.ends_with(".webp")
    {
        "▣".to_string()
    } else if lower.ends_with(".mp3")
        || lower.ends_with(".wav")
        || lower.ends_with(".flac")
        || lower.ends_with(".ogg")
    {
        "♪".to_string()
    } else if lower.ends_with(".mp4")
        || lower.ends_with(".mkv")
        || lower.ends_with(".avi")
        || lower.ends_with(".webm")
    {
        "▶".to_string()
    } else if lower.ends_with(".zip")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".7z")
        || lower.ends_with(".rar")
        || lower.ends_with(".deb")
    {
        "▤".to_string()
    } else if lower.ends_with(".pdf") {
        "▧".to_string()
    } else if lower.ends_with(".toml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".json")
        || lower.ends_with(".xml")
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
    let mut segments = if path.starts_with('/') {
        vec![("/".into(), "/".into())]
    } else {
        Vec::new()
    };
    let mut accumulated = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 && *part == "~" {
            accumulated = "~".to_string();
        } else if i == 0 {
            accumulated = format!("/{}", part);
        } else {
            accumulated = format!("{}/{}", accumulated, part);
        }
        segments.push((
            if *part == "~" {
                "Home".into()
            } else {
                part.to_string()
            },
            accumulated.clone(),
        ));
    }
    segments
}

/// Delete uses recoverable Trash; permanent removal is confined to explicit Empty Trash.
pub fn delete_entry(dir: &str, name: &str) -> Result<(), String> {
    crate::fileops::name(name)?;
    crate::fileops::trash(&expand_home(dir).join(name), &crate::fileops::trash_root()).map(|_| ())
}
pub fn rename_entry(dir: &str, old: &str, new: &str) -> Result<(), String> {
    crate::fileops::name(old)?;
    crate::fileops::name(new)?;
    if old == new {
        return Ok(());
    }
    crate::fileops::rename_no_replace(&expand_home(dir).join(old), &expand_home(dir).join(new))
}
pub fn create_folder(dir: &str, name: &str) -> Result<(), String> {
    crate::fileops::create_folder(&expand_home(dir), name)
}
pub fn copy_entry(src: &str, name: &str, dst: &str) -> Result<(), String> {
    crate::fileops::name(name)?;
    crate::fileops::transfer(
        &expand_home(src).join(name),
        &expand_home(dst),
        false,
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_| {},
    )
}
pub fn move_entry(src: &str, name: &str, dst: &str) -> Result<(), String> {
    crate::fileops::name(name)?;
    crate::fileops::transfer(
        &expand_home(src).join(name),
        &expand_home(dst),
        true,
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_| {},
    )
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
        if m == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", m)
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", h)
        }
    } else if secs < 86400 * 30 {
        let d = secs / 86400;
        if d == 1 {
            "Yesterday".to_string()
        } else {
            format!("{} days ago", d)
        }
    } else if secs < 86400 * 365 {
        let mo = secs / (86400 * 30);
        if mo == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", mo)
        }
    } else {
        let y = secs / (86400 * 365);
        if y == 1 {
            "1 year ago".to_string()
        } else {
            format!("{} years ago", y)
        }
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
        ".txt",
        ".md",
        ".log",
        ".rs",
        ".py",
        ".js",
        ".ts",
        ".c",
        ".h",
        ".go",
        ".java",
        ".toml",
        ".yaml",
        ".yml",
        ".json",
        ".xml",
        ".sh",
        ".css",
        ".html",
        ".htm",
        ".csv",
        ".ini",
        ".cfg",
        ".conf",
        ".env",
        ".makefile",
        ".dockerfile",
    ];
    text_exts.iter().any(|ext| lower.ends_with(ext))
        || lower == "makefile"
        || lower == "dockerfile"
        || lower == ".gitignore"
        || lower == ".dockerignore"
}

pub fn read_preview(path: &Path, max_lines: usize) -> String {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
    else {
        return "Could not read this file.".into();
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return String::new();
    }
    let mut data = Vec::new();
    if let Err(e) = file.take(65536).read_to_end(&mut data) {
        return format!("Could not read preview: {e}");
    }
    String::from_utf8_lossy(&data)
        .lines()
        .take(max_lines)
        .map(|line| {
            let mut text: String = line.chars().take(240).collect();
            if line.chars().count() > 240 {
                text.push('…');
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn create_file(dir: &str, name: &str) -> Result<(), String> {
    crate::fileops::create_file(&expand_home(dir), name)
}

/// Compress a file or directory into a .tar.gz archive.
pub fn compress_entry(dir: &str, name: &str) -> Result<(), String> {
    let expanded = expand_home(dir);
    let target = expanded.join(name);
    if !target.exists() {
        return Err("File not found".to_string());
    }
    let archive_name = format!("{}.tar.gz", name);
    let result = std::process::Command::new("tar")
        .args(["czf", &archive_name, name])
        .current_dir(&expanded)
        .output();
    match result {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Get the full path of a file for clipboard copy.
pub fn get_full_path(dir: &str, name: &str) -> String {
    let expanded = expand_home(dir);
    let path = expanded.join(name);
    path.display().to_string()
}

/// Calculate total size of a list of files in a directory.
pub fn calculate_selection_size(dir: &str, names: &[String]) -> String {
    let total = names
        .iter()
        .filter_map(|n| std::fs::symlink_metadata(expand_home(dir).join(n)).ok())
        .filter(|m| m.is_file())
        .fold(0u64, |sum, m| sum.saturating_add(m.len()));
    format_size(total)
}

/// The line the Files footer shows for a selection (issue #208). "1 selected · 0.0 KiB in
/// files" said nothing about the folder that was picked, so a folder answers with what it
/// holds, a file with its size, and a bigger selection counts its folders and totals the
/// file bytes.
pub fn selection_text(items: &[&DirEntry], show_hidden: bool) -> String {
    let folders = items.iter().filter(|e| e.is_dir).count();
    if items.len() == 1 {
        let only = items[0];
        if only.is_dir {
            return match only.items.as_ref().and_then(|c| c.shown(show_hidden)) {
                Some(1) => "1 folder · 1 item".to_string(),
                Some(count) => format!("1 folder · {count} items"),
                None => "1 folder · items unknown".to_string(),
            };
        }
        return format!("1 file · {}", only.size_text);
    }
    let mut parts = vec![format!("{} selected", items.len())];
    if folders == 1 {
        parts.push("1 folder".to_string());
    } else if folders > 1 {
        parts.push(format!("{folders} folders"));
    }
    if folders < items.len() {
        let bytes = items
            .iter()
            .filter(|e| !e.is_dir)
            .fold(0u64, |total, e| total.saturating_add(e.size_bytes));
        parts.push(format!("{} in files", format_size(bytes)));
    }
    parts.join(" · ")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, items: Option<ItemCount>) -> DirEntry {
        DirEntry {
            size_bytes: 0,
            modified: std::time::SystemTime::UNIX_EPOCH,
            name: name.to_string(),
            is_dir: true,
            size_text: String::new(),
            modified_text: String::new(),
            icon_char: String::new(),
            selected: false,
            items,
        }
    }

    fn file(name: &str, bytes: u64) -> DirEntry {
        DirEntry {
            is_dir: false,
            size_bytes: bytes,
            size_text: format_size(bytes),
            items: None,
            ..dir(name, None)
        }
    }

    #[test]
    fn one_folder_answers_with_what_it_holds() {
        let empty = dir("Tour 23 Sep", Some(ItemCount::Known { all: 0, visible: 0 }));
        assert_eq!(selection_text(&[&empty], false), "1 folder · 0 items");
        let one = dir("src", Some(ItemCount::Known { all: 1, visible: 1 }));
        assert_eq!(selection_text(&[&one], false), "1 folder · 1 item");
        let shared = dir("Shared", Some(ItemCount::Unknown("permission denied".into())));
        assert_eq!(selection_text(&[&shared], false), "1 folder · items unknown");
    }

    #[test]
    fn the_folder_count_follows_the_hidden_files_view() {
        let counted = dir("docs", Some(ItemCount::Known { all: 3, visible: 2 }));
        assert_eq!(selection_text(&[&counted], false), "1 folder · 2 items");
        assert_eq!(selection_text(&[&counted], true), "1 folder · 3 items");
    }

    #[test]
    fn one_file_answers_with_its_size() {
        let notes = file("notes.md", 2458);
        assert_eq!(selection_text(&[&notes], false), "1 file · 2.4 KB");
    }

    #[test]
    fn a_bigger_selection_counts_folders_and_totals_files() {
        let a = dir("projects", Some(ItemCount::Known { all: 6, visible: 6 }));
        let b = dir("src", Some(ItemCount::Known { all: 2, visible: 2 }));
        let c = file("forge.py", 2048);
        let d = file("notes.md", 1024);
        assert_eq!(
            selection_text(&[&a, &b, &c, &d], false),
            "4 selected · 2 folders · 3.0 KB in files"
        );
        assert_eq!(selection_text(&[&a, &b], false), "2 selected · 2 folders");
        assert_eq!(
            selection_text(&[&a, &c], false),
            "2 selected · 1 folder · 2.0 KB in files"
        );
    }
}
