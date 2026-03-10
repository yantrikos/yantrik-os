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
    is_selected: bool,
    // Checksum
    checksum_expected: String,  // user-supplied expected hash
    checksum_status: ChecksumStatus,
    // Per-download save location
    save_dir: String,
    // Detail fields
    start_time: String,
    end_time: String,
    file_hash: String,          // computed SHA-256 after download
    file_type: String,          // MIME type
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

    fn sort_order(&self) -> i32 {
        match self {
            DlStatus::Downloading => 0,
            DlStatus::Queued => 1,
            DlStatus::Paused => 2,
            DlStatus::Failed(_) => 3,
            DlStatus::Completed => 4,
            DlStatus::Cancelled => 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ChecksumStatus {
    None,
    Verifying,
    Pass,
    Fail,
}

impl ChecksumStatus {
    fn as_str(&self) -> &str {
        match self {
            ChecksumStatus::None => "none",
            ChecksumStatus::Verifying => "verifying",
            ChecksumStatus::Pass => "pass",
            ChecksumStatus::Fail => "fail",
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
pub fn wire(ui: &App, ctx: &AppContext) {
    let state: SharedState = Arc::new(Mutex::new(Vec::new()));
    let process_map: ProcessMap = Arc::new(Mutex::new(HashMap::new()));
    let filter: Rc<RefCell<i32>> = Rc::new(RefCell::new(0));
    let search_query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let sort_mode: Rc<RefCell<i32>> = Rc::new(RefCell::new(0));

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
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();
        ui.on_dl_add(move |url, checksum, save_dir| {
            let url_str = url.to_string().trim().to_string();
            if url_str.is_empty() {
                return;
            }

            let checksum_str = checksum.to_string().trim().to_string();
            let save_dir_str = save_dir.to_string().trim().to_string();

            let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let filename = extract_filename(&url_str);

            // Use custom save dir or default
            let download_dir = if save_dir_str.is_empty() {
                download_dir()
            } else {
                PathBuf::from(&save_dir_str)
            };
            let output_path = download_dir.join(&filename);

            // Ensure download directory exists
            let _ = std::fs::create_dir_all(&download_dir);

            let now = format_timestamp();

            // Guess file type from extension
            let file_type = guess_file_type(&filename);

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
                is_selected: false,
                checksum_expected: checksum_str,
                checksum_status: ChecksumStatus::None,
                save_dir: save_dir_str,
                start_time: now,
                end_time: String::new(),
                file_hash: String::new(),
                file_type,
            };

            {
                let mut s = state.lock().unwrap();
                s.push(dl);
            }

            // Start download in background thread
            start_download(id, url_str, output_path, state.clone(), process_map.clone());

            // Immediately refresh UI
            if let Some(ui) = ui_weak.upgrade() {
                update_ui(&ui, &state, *filter.borrow(), &search_query.borrow(), *sort_mode.borrow());
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
                    dl.start_time = format_timestamp();
                    dl.end_time = String::new();
                    dl.file_hash = String::new();
                    dl.checksum_status = ChecksumStatus::None;
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

    // ── Verify Checksum callback ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_dl_verify_checksum(move |id| {
            let (output_path, expected) = {
                let mut s = state.lock().unwrap();
                if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                    if dl.status != DlStatus::Completed {
                        return;
                    }
                    dl.checksum_status = ChecksumStatus::Verifying;
                    (dl.output_path.clone(), dl.checksum_expected.clone())
                } else {
                    return;
                }
            };

            let state = state.clone();
            let ui_weak = ui_weak.clone();
            std::thread::spawn(move || {
                let hash = compute_sha256(&output_path);
                let mut s = state.lock().unwrap();
                if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                    match hash {
                        Some(h) => {
                            dl.file_hash = h.clone();
                            if expected.is_empty() {
                                // No expected checksum — just show the computed hash
                                dl.checksum_status = ChecksumStatus::Pass;
                                let hash_display = h.clone();
                                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                                    ui.set_dl_checksum_result(hash_display.into());
                                });
                            } else {
                                // Compare case-insensitively
                                if h.to_lowercase() == expected.to_lowercase() {
                                    dl.checksum_status = ChecksumStatus::Pass;
                                } else {
                                    dl.checksum_status = ChecksumStatus::Fail;
                                }
                            }
                        }
                        None => {
                            dl.checksum_status = ChecksumStatus::Fail;
                        }
                    }
                }
            });
        });
    }

    // ── Toggle Select callback ──
    {
        let state = state.clone();
        ui.on_dl_toggle_select(move |id| {
            let mut s = state.lock().unwrap();
            if let Some(dl) = s.iter_mut().find(|d| d.id == id) {
                dl.is_selected = !dl.is_selected;
            }
        });
    }

    // ── Select All callback ──
    {
        let state = state.clone();
        ui.on_dl_select_all(move |val| {
            let mut s = state.lock().unwrap();
            for dl in s.iter_mut() {
                dl.is_selected = val;
            }
        });
    }

    // ── Pause All callback ──
    {
        let state = state.clone();
        let process_map = process_map.clone();
        ui.on_dl_pause_all(move || {
            let mut s = state.lock().unwrap();
            let mut pmap = process_map.lock().unwrap();
            for dl in s.iter_mut() {
                if dl.status == DlStatus::Downloading {
                    if let Some(pid) = pmap.remove(&dl.id) {
                        kill_process(pid);
                    }
                    dl.status = DlStatus::Paused;
                    dl.speed = String::new();
                    dl.eta = String::new();
                }
            }
        });
    }

    // ── Resume All callback ──
    {
        let state = state.clone();
        let process_map = process_map.clone();
        ui.on_dl_resume_all(move || {
            let ids_to_resume: Vec<(i32, String, PathBuf)> = {
                let mut s = state.lock().unwrap();
                s.iter_mut()
                    .filter(|d| d.status == DlStatus::Paused)
                    .map(|d| {
                        d.status = DlStatus::Downloading;
                        (d.id, d.url.clone(), d.output_path.clone())
                    })
                    .collect()
            };
            for (id, url, output_path) in ids_to_resume {
                start_download(id, url, output_path, state.clone(), process_map.clone());
            }
        });
    }

    // ── Search callback ──
    {
        let search_query = search_query.clone();
        ui.on_dl_search(move |query| {
            *search_query.borrow_mut() = query.to_string();
        });
    }

    // ── Sort callback ──
    {
        let sort_mode = sort_mode.clone();
        ui.on_dl_sort(move |mode| {
            *sort_mode.borrow_mut() = mode;
        });
    }

    // ── 500ms progress poll timer ──
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let filter = filter.clone();
        let search_query = search_query.clone();
        let sort_mode = sort_mode.clone();

        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            if let Some(ui) = ui_weak.upgrade() {
                if ui.get_current_screen() == 24 {
                    update_ui(&ui, &state, *filter.borrow(), &search_query.borrow(), *sort_mode.borrow());
                }
            }
            // Save history periodically
            if let Ok(s) = state.lock() {
                let _ = save_history(&s);
            }
        });
        std::mem::forget(timer);
    }

    // ── AI Explain callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_dl_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let active = ui.get_dl_active_count();
        let speed = ui.get_dl_total_speed().to_string();
        let completed = ui.get_dl_completed_today();
        let error = ui.get_dl_error_text().to_string();

        let context = format!(
            "Active downloads: {}\nTotal speed: {}\nCompleted today: {}\nErrors: {}",
            active,
            if speed.is_empty() { "none" } else { &speed },
            completed,
            if error.is_empty() { "none" } else { &error }
        );
        let prompt = super::ai_assist::download_analysis_prompt(&context);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_dl_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_dl_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_dl_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_dl_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_dl_ai_panel_open(false);
        }
    });
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
                    dl.end_time = format_timestamp();
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
                        dl.end_time = format_timestamp();
                        // Update final file size
                        if let Ok(meta) = std::fs::metadata(&dl.output_path) {
                            dl.downloaded = meta.len();
                            dl.total = meta.len();
                        }
                        // Compute file hash in background after completion
                        let output_path = dl.output_path.clone();
                        let state_clone = state.clone();
                        let dl_id = dl.id;
                        // Auto-verify if checksum expected is set
                        let has_expected = !dl.checksum_expected.is_empty();
                        let expected = dl.checksum_expected.clone();
                        if has_expected {
                            dl.checksum_status = ChecksumStatus::Verifying;
                        }
                        drop(s); // release lock before spawning
                        std::thread::spawn(move || {
                            if let Some(hash) = compute_sha256(&output_path) {
                                let mut s2 = state_clone.lock().unwrap();
                                if let Some(dl2) = s2.iter_mut().find(|d| d.id == dl_id) {
                                    dl2.file_hash = hash.clone();
                                    if has_expected {
                                        if hash.to_lowercase() == expected.to_lowercase() {
                                            dl2.checksum_status = ChecksumStatus::Pass;
                                        } else {
                                            dl2.checksum_status = ChecksumStatus::Fail;
                                        }
                                    }
                                }
                            }
                        });
                        return; // already dropped s
                    }
                    Ok(status) => {
                        // Exit code 33 means range request not supported (resume failed), not necessarily an error
                        let code = status.code().unwrap_or(-1);
                        dl.end_time = format_timestamp();
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
                        dl.end_time = format_timestamp();
                    }
                }
            }
        }
    });
}

/// Update the Slint UI from shared state.
fn update_ui(ui: &App, state: &SharedState, filter: i32, search_query: &str, sort_mode: i32) {
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

    // Apply status filter
    let status_filtered: Vec<&DownloadState> = match filter {
        1 => s.iter().filter(|d| d.status == DlStatus::Downloading || d.status == DlStatus::Paused || d.status == DlStatus::Queued).collect(),
        2 => s.iter().filter(|d| d.status == DlStatus::Completed).collect(),
        3 => s.iter().filter(|d| matches!(d.status, DlStatus::Failed(_))).collect(),
        _ => s.iter().collect(),
    };

    // Apply search filter
    let search_lower = search_query.to_lowercase();
    let filtered: Vec<&DownloadState> = if search_lower.is_empty() {
        status_filtered
    } else {
        status_filtered
            .into_iter()
            .filter(|d| {
                d.filename.to_lowercase().contains(&search_lower)
                    || d.url.to_lowercase().contains(&search_lower)
            })
            .collect()
    };

    let match_count = filtered.len() as i32;

    // Apply sort
    let mut sorted: Vec<&DownloadState> = filtered;
    match sort_mode {
        1 => {
            // Sort by name
            sorted.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase()));
        }
        2 => {
            // Sort by size (largest first)
            sorted.sort_by(|a, b| b.total.cmp(&a.total));
        }
        3 => {
            // Sort by status
            sorted.sort_by(|a, b| a.status.sort_order().cmp(&b.status.sort_order()));
        }
        _ => {
            // Sort by date (newest first) — higher ID = newer
            sorted.sort_by(|a, b| b.id.cmp(&a.id));
        }
    }

    let items: Vec<DownloadItem> = sorted
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
            is_selected: d.is_selected,
            checksum_expected: d.checksum_expected.clone().into(),
            checksum_status: d.checksum_status.as_str().into(),
            save_dir: d.save_dir.clone().into(),
            start_time: d.start_time.clone().into(),
            end_time: d.end_time.clone().into(),
            file_hash: d.file_hash.clone().into(),
            file_type: d.file_type.clone().into(),
        })
        .collect();

    ui.set_dl_downloads(ModelRc::new(VecModel::from(items)));
    ui.set_dl_active_count(active_count as i32);
    ui.set_dl_total_speed(total_speed.into());
    ui.set_dl_completed_today(completed_today as i32);
    ui.set_dl_search_match_count(match_count);
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

/// Format current timestamp as "YYYY-MM-DD HH:MM:SS".
fn format_timestamp() -> String {
    // Use std::process to call date for a simple timestamp (no chrono dependency)
    if let Ok(output) = Command::new("date")
        .args(["+%Y-%m-%d %H:%M:%S"])
        .output()
    {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        String::new()
    }
}

/// Compute SHA-256 hash of a file using sha256sum command.
fn compute_sha256(path: &PathBuf) -> Option<String> {
    let output = Command::new("sha256sum")
        .arg(path.to_str()?)
        .output()
        .ok()?;

    if output.status.success() {
        let out = String::from_utf8_lossy(&output.stdout);
        // sha256sum output: "hash  filename"
        out.split_whitespace().next().map(|s| s.to_string())
    } else {
        None
    }
}

/// Guess MIME type from file extension.
fn guess_file_type(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "xz" => "application/x-xz",
        "bz2" => "application/x-bzip2",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/x-rar-compressed",
        "deb" => "application/x-debian-package",
        "rpm" => "application/x-rpm",
        "apk" => "application/vnd.android.package-archive",
        "dmg" => "application/x-apple-diskimage",
        "iso" => "application/x-iso9660-image",
        "exe" | "msi" => "application/x-executable",
        "pdf" => "application/pdf",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        "ppt" | "pptx" => "application/vnd.ms-powerpoint",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "md" => "text/markdown",
        "sh" => "application/x-sh",
        "py" => "text/x-python",
        "rs" => "text/x-rust",
        "c" | "h" => "text/x-c",
        "cpp" | "hpp" => "text/x-c++",
        "java" => "text/x-java",
        "go" => "text/x-go",
        "wasm" => "application/wasm",
        "ttf" | "otf" => "font/opentype",
        "woff" | "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
    .to_string()
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
    #[serde(default)]
    checksum_expected: String,
    #[serde(default)]
    checksum_status: String,
    #[serde(default)]
    save_dir: String,
    #[serde(default)]
    start_time: String,
    #[serde(default)]
    end_time: String,
    #[serde(default)]
    file_hash: String,
    #[serde(default)]
    file_type: String,
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
            checksum_expected: d.checksum_expected.clone(),
            checksum_status: d.checksum_status.as_str().to_string(),
            save_dir: d.save_dir.clone(),
            start_time: d.start_time.clone(),
            end_time: d.end_time.clone(),
            file_hash: d.file_hash.clone(),
            file_type: d.file_type.clone(),
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
        .map(|e| {
            let checksum_status = match e.checksum_status.as_str() {
                "pass" => ChecksumStatus::Pass,
                "fail" => ChecksumStatus::Fail,
                "verifying" => ChecksumStatus::None, // reset on reload
                _ => ChecksumStatus::None,
            };
            DownloadState {
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
                is_selected: false,
                checksum_expected: e.checksum_expected,
                checksum_status,
                save_dir: e.save_dir,
                start_time: e.start_time,
                end_time: e.end_time,
                file_hash: e.file_hash,
                file_type: e.file_type,
            }
        })
        .collect())
}
