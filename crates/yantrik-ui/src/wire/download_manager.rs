//! Download Manager wire module — curl-based downloads with progress tracking.
//!
//! Each download runs in a background thread using `curl` subprocess.
//! A 500ms timer polls shared state and updates the UI.
//! Download history is persisted to `~/.config/yantrik/downloads.json`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, DownloadItem};

/// Internal download state shared between UI thread and download threads.
#[derive(Clone, Debug)]
struct DownloadState {
    id: i32,
    filename: String,
    url: String,
    progress: f32,       // 0.0 to 1.0
    speed: String,       // "2.4 MB/s"
    downloaded: u64,     // bytes downloaded
    total: u64,          // total bytes (0 if unknown)
    status: DlStatus,
    eta: String,
    output_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
enum DlStatus {
    Downloading,
    Paused,
    Completed,
    Failed(String),
    Queued,
    Cancelled,
}

impl DlStatus {
    fn as_str(&self) -> &str {
        match self {
            DlStatus::Downloading => "downloading",
            DlStatus::Paused => "paused",
            DlStatus::Completed => "completed",
            DlStatus::Failed(_) => "failed",
            DlStatus::Queued => "queued",
            DlStatus::Cancelled => "cancelled",
        }
    }
}

/// Shared state between threads.
type SharedState = Arc<Mutex<Vec<DownloadState>>>;

/// Map of download id -> child process handle (for pause/cancel).
type ProcessMap = Arc<Mutex<HashMap<i32, u32>>>;  // id -> pid

/// Next download ID counter.
static NEXT_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

/// Wire download manager callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    let state: SharedState = Arc::new(Mutex::new(Vec::new()));
    let process_map: ProcessMap = Arc::new(Mutex::new(HashMap::new()));
    let filter: Rc<RefCell<i32>> = Rc::new(RefCell::new(0));

    // Load persisted history on startup
    if let Ok(history) = load_history() {
        let mut s = state.lock().unwrap();
        *s = history;
        // Update NEXT_ID to be past all existing IDs
        let max_id = s.iter().map(|d| d.id).max().unwrap_or(0);
        NEXT_ID.store(max_id + 1, std::sync::atomic::Ordering::Relaxed);
    }

    // ── Add URL callback ──
    {
        let state = state.clone();
        let process_map = process_map.clone();
        let ui_weak = ui.as_weak();
        let filter = filter.clone();
        ui.on_dl_add(move |url| {
            let url_str = url.to_string().trim().to_string();
            if url_str.is_empty() {
                return;
            }

            let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let filename = extract_filename(&url_str);
            let download_dir = download_dir();
            let output_path = download_dir.join(&filename);

            // Ensure download directory exists
            let _ = std::fs::create_dir_all(&download_dir);

            let dl = DownloadState {
                id,
                filename: filename.clone(),
                url: url_str.clone(),
                progress: 0.0,
                speed: String::new(),
                downloaded: 0,
                total: 0,
                status: DlStatus::Downloading,
                eta: String::new(),
                output_path: output_path.clone(),
            };

            {
                let mut s = state.lock().unwrap();
                s.push(dl);
            }

            // Start download in background thread
            start_download(id, url_str, output_path, state.clone(), process_map.clone());

            // Immediately refresh UI
            if let Some(ui) = ui_weak.upgrade() {
                update_ui(&ui, &state, *filter.borrow());
            }
        });
    }

    // ── Pause callback ──
    {
        let state = state.clone();
        let process_map = process_map.clone();
        ui.on_dl_pause(move |id| {
            // Kill the curl process to pause
            if let Some(pid) = process_map.lock().unwrap().remove(&id) {
                kill_process(pid);
            }
            let mut s = state.lock().unwrap();
            if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                dl.status = DlStatus::Paused;
                dl.speed = String::new();
                dl.eta = String::new();
            }
        });
    }

    // ── Resume callback ──
    {
        let state = state.clone();
        let process_map = process_map.clone();
        ui.on_dl_resume(move |id| {
            let (url, output_path) = {
                let mut s = state.lock().unwrap();
                if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                    dl.status = DlStatus::Downloading;
                    (dl.url.clone(), dl.output_path.clone())
                } else {
                    return;
                }
            };
            // Resume with -C - (continue from where we left off)
            start_download(id, url, output_path, state.clone(), process_map.clone());
        });
    }

    // ── Cancel callback ──
    {
        let state = state.clone();
        let process_map = process_map.clone();
        ui.on_dl_cancel(move |id| {
            // Kill process if running
            if let Some(pid) = process_map.lock().unwrap().remove(&id) {
                kill_process(pid);
            }
            let mut s = state.lock().unwrap();
            if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                dl.status = DlStatus::Cancelled;
                dl.speed = String::new();
                dl.eta = String::new();
            }
            // Remove cancelled downloads from the list
            s.retain(|d| d.id != id);
        });
    }

    // ── Retry callback ──
    {
        let state = state.clone();
        let process_map = process_map.clone();
        ui.on_dl_retry(move |id| {
            let (url, output_path) = {
                let mut s = state.lock().unwrap();
                if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                    dl.status = DlStatus::Downloading;
                    dl.progress = 0.0;
                    dl.speed = String::new();
                    dl.eta = String::new();
                    dl.downloaded = 0;
                    (dl.url.clone(), dl.output_path.clone())
                } else {
                    return;
                }
            };
            // Remove partial file before retry
            let _ = std::fs::remove_file(&output_path);
            start_download(id, url, output_path, state.clone(), process_map.clone());
        });
    }

    // ── Open folder callback ──
    {
        let state = state.clone();
        ui.on_dl_open_folder(move |id| {
            let s = state.lock().unwrap();
            if let Some(dl) = s.iter().find(|d| d.id == id) {
                let dir = dl.output_path.parent().unwrap_or(&dl.output_path);
                // Try xdg-open on Linux (Alpine)
                let _ = Command::new("xdg-open")
                    .arg(dir)
                    .spawn();
            }
        });
    }

    // ── Clear completed callback ──
    {
        let state = state.clone();
        ui.on_dl_clear_completed(move || {
            let mut s = state.lock().unwrap();
            s.retain(|d| d.status != DlStatus::Completed);
        });
    }

    // ── Filter callback ──
    {
        let filter = filter.clone();
        ui.on_dl_filter(move |idx| {
            *filter.borrow_mut() = idx;
        });
    }

    // ── 500ms progress poll timer ──
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let filter = filter.clone();

        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            if let Some(ui) = ui_weak.upgrade() {
                if ui.get_current_screen() == 24 {
                    update_ui(&ui, &state, *filter.borrow());
                }
            }
            // Save history periodically
            if let Ok(s) = state.lock() {
                let _ = save_history(&s);
            }
        });
        std::mem::forget(timer);
    }
}

/// Start a curl download in a background thread.
fn start_download(
    id: i32,
    url: String,
    output_path: PathBuf,
    state: SharedState,
    process_map: ProcessMap,
) {
    std::thread::spawn(move || {
        // Use curl with progress output:
        // -L: follow redirects
        // -C -: continue partial download (for resume)
        // --progress-bar: show progress bar on stderr
        // -o: output file
        let result = Command::new("curl")
            .args([
                "-L",
                "-C", "-",
                "--progress-bar",
                "-o",
                output_path.to_str().unwrap_or("download"),
                &url,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();

        let mut child: Child = match result {
            Ok(child) => child,
            Err(e) => {
                let mut s = state.lock().unwrap();
                if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                    dl.status = DlStatus::Failed(format!("Failed to start curl: {}", e));
                }
                return;
            }
        };

        // Record PID for pause/cancel
        let pid = child.id();
        process_map.lock().unwrap().insert(id, pid);

        // Parse stderr for progress
        if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };

                // curl progress bar output looks like:
                // ###                                                 5.8%
                // or with --progress-bar:
                // ####################################################100.0%
                // Parse percentage from the line
                if let Some(pct) = parse_curl_progress(&line) {
                    let mut s = state.lock().unwrap();
                    if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                        if dl.status == DlStatus::Cancelled || dl.status == DlStatus::Paused {
                            break;
                        }
                        dl.progress = pct / 100.0;

                        // Try to get file size from disk for size text
                        if let Ok(meta) = std::fs::metadata(&dl.output_path) {
                            dl.downloaded = meta.len();
                            if pct > 0.0 {
                                dl.total = (meta.len() as f64 / (pct as f64 / 100.0)) as u64;
                            }
                        }
                    }
                }

                // Also parse speed if present in curl output
                if let Some(speed) = parse_curl_speed(&line) {
                    let mut s = state.lock().unwrap();
                    if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                        dl.speed = speed;
                    }
                }
            }
        }

        // Wait for completion
        let exit = child.wait();
        process_map.lock().unwrap().remove(&id);

        let mut s = state.lock().unwrap();
        if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
            // Don't overwrite paused/cancelled status
            if dl.status == DlStatus::Downloading {
                match exit {
                    Ok(status) if status.success() => {
                        dl.status = DlStatus::Completed;
                        dl.progress = 1.0;
                        dl.speed = String::new();
                        dl.eta = String::new();
                        // Update final file size
                        if let Ok(meta) = std::fs::metadata(&dl.output_path) {
                            dl.downloaded = meta.len();
                            dl.total = meta.len();
                        }
                    }
                    Ok(status) => {
                        // Exit code 33 means range request not supported (resume failed), not necessarily an error
                        let code = status.code().unwrap_or(-1);
                        if code == 33 {
                            // Try without resume
                            dl.status = DlStatus::Failed("Resume not supported. Try again.".to_string());
                        } else {
                            dl.status = DlStatus::Failed(format!("curl exited with code {}", code));
                        }
                        dl.speed = String::new();
                    }
                    Err(e) => {
                        dl.status = DlStatus::Failed(format!("Process error: {}", e));
                        dl.speed = String::new();
                    }
                }
            }
        }
    });
}

/// Update the Slint UI from shared state.
fn update_ui(ui: &App, state: &SharedState, filter: i32) {
    let s = state.lock().unwrap();

    // Compute stats
    let active_count = s.iter().filter(|d| d.status == DlStatus::Downloading).count();
    let completed_today = s.iter().filter(|d| d.status == DlStatus::Completed).count();

    // Sum up speed text from all active downloads
    let total_speed = if active_count > 0 {
        // Just show aggregate speed text from active downloads
        let speeds: Vec<&str> = s
            .iter()
            .filter(|d| d.status == DlStatus::Downloading && !d.speed.is_empty())
            .map(|d| d.speed.as_str())
            .collect();
        if speeds.is_empty() {
            "calculating...".to_string()
        } else if speeds.len() == 1 {
            speeds[0].to_string()
        } else {
            format!("{} streams", speeds.len())
        }
    } else {
        "0 B/s".to_string()
    };

    // Apply filter
    let filtered: Vec<&DownloadState> = match filter {
        1 => s.iter().filter(|d| d.status == DlStatus::Downloading || d.status == DlStatus::Paused || d.status == DlStatus::Queued).collect(),
        2 => s.iter().filter(|d| d.status == DlStatus::Completed).collect(),
        3 => s.iter().filter(|d| matches!(d.status, DlStatus::Failed(_))).collect(),
        _ => s.iter().collect(),
    };

    let items: Vec<DownloadItem> = filtered
        .iter()
        .map(|d| DownloadItem {
            id: d.id,
            filename: d.filename.clone().into(),
            url: d.url.clone().into(),
            progress: d.progress,
            speed_text: d.speed.clone().into(),
            size_text: format_size_text(d.downloaded, d.total).into(),
            status: d.status.as_str().into(),
            eta_text: d.eta.clone().into(),
            is_selected: false,
        })
        .collect();

    ui.set_dl_downloads(ModelRc::new(VecModel::from(items)));
    ui.set_dl_active_count(active_count as i32);
    ui.set_dl_total_speed(total_speed.into());
    ui.set_dl_completed_today(completed_today as i32);
}

/// Extract filename from URL.
fn extract_filename(url: &str) -> String {
    // Strip query string and fragment
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split('#').next().unwrap_or(path);

    // Get last path segment
    let name = path.rsplit('/').next().unwrap_or("download");
    let name = name.trim();

    if name.is_empty() {
        "download".to_string()
    } else {
        // Simple percent-decode for common characters
        percent_decode(name)
    }
}

/// Simple percent-decoding for URL filenames.
fn percent_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse curl progress percentage from stderr line.
///
/// curl --progress-bar writes lines like:
/// `###                                               5.8%`
/// or `####################################################100.0%`
fn parse_curl_progress(line: &str) -> Option<f32> {
    let trimmed = line.trim();

    // Look for percentage at end of line
    if let Some(pct_pos) = trimmed.rfind('%') {
        // Get the number before %
        let before = &trimmed[..pct_pos];
        // Find the start of the number (last space or #)
        let num_start = before
            .rfind(|c: char| !c.is_ascii_digit() && c != '.')
            .map(|i| i + 1)
            .unwrap_or(0);
        let num_str = &before[num_start..];
        if let Ok(pct) = num_str.parse::<f32>() {
            return Some(pct.clamp(0.0, 100.0));
        }
    }

    None
}

/// Parse speed from curl output (if present).
fn parse_curl_speed(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // curl sometimes outputs speed info like "1234k" or "5.2M"
    // In --progress-bar mode, the speed isn't always shown inline.
    // We'll try to detect common patterns.
    for suffix in &["k/s", "M/s", "G/s", "B/s"] {
        if let Some(pos) = trimmed.find(suffix) {
            // Walk backwards to find start of number
            let before = &trimmed[..pos];
            let num_start = before
                .rfind(|c: char| !c.is_ascii_digit() && c != '.')
                .map(|i| i + 1)
                .unwrap_or(0);
            let speed_str = &trimmed[num_start..pos + suffix.len()];
            if !speed_str.is_empty() {
                return Some(speed_str.to_string());
            }
        }
    }

    None
}

/// Format download size text like "45.2 / 128.0 MB".
fn format_size_text(downloaded: u64, total: u64) -> String {
    if total == 0 && downloaded == 0 {
        return String::new();
    }

    let dl_str = format_bytes(downloaded);
    if total > 0 {
        let total_str = format_bytes(total);
        format!("{} / {}", dl_str, total_str)
    } else {
        dl_str
    }
}

/// Format bytes as human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Kill a process by PID.
fn kill_process(pid: u32) {
    // On Unix/Linux, send SIGTERM
    let _ = Command::new("kill")
        .arg(pid.to_string())
        .output();
}

/// Get the download directory path.
fn download_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join("Downloads")
    } else {
        PathBuf::from("/tmp/downloads")
    }
}

/// History file path.
fn history_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("yantrik")
            .join("downloads.json")
    } else {
        PathBuf::from("/tmp/yantrik-downloads.json")
    }
}

/// Simple JSON history entry for serialization.
#[derive(serde::Serialize, serde::Deserialize)]
struct HistoryEntry {
    id: i32,
    filename: String,
    url: String,
    progress: f32,
    status: String,
    downloaded: u64,
    total: u64,
    output_path: String,
}

/// Save download history to JSON file.
fn save_history(downloads: &[DownloadState]) -> std::io::Result<()> {
    let path = history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Only save completed and failed downloads (not in-progress)
    let entries: Vec<HistoryEntry> = downloads
        .iter()
        .filter(|d| d.status == DlStatus::Completed || matches!(d.status, DlStatus::Failed(_)))
        .map(|d| HistoryEntry {
            id: d.id,
            filename: d.filename.clone(),
            url: d.url.clone(),
            progress: d.progress,
            status: d.status.as_str().to_string(),
            downloaded: d.downloaded,
            total: d.total,
            output_path: d.output_path.to_string_lossy().to_string(),
        })
        .collect();

    let json = serde_json::to_string_pretty(&entries)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, json)
}

/// Load download history from JSON file.
fn load_history() -> std::io::Result<Vec<DownloadState>> {
    let path = history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let json = std::fs::read_to_string(&path)?;
    let entries: Vec<HistoryEntry> = serde_json::from_str(&json)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    Ok(entries
        .into_iter()
        .map(|e| DownloadState {
            id: e.id,
            filename: e.filename,
            url: e.url,
            progress: e.progress,
            speed: String::new(),
            downloaded: e.downloaded,
            total: e.total,
            status: match e.status.as_str() {
                "completed" => DlStatus::Completed,
                "failed" => DlStatus::Failed(String::new()),
                _ => DlStatus::Completed,
            },
            eta: String::new(),
            output_path: PathBuf::from(e.output_path),
        })
        .collect())
}
