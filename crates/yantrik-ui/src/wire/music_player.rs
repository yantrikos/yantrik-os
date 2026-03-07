//! Music Player wiring — library scanner, queue management, mpv playback.
//!
//! Scans ~/Music for audio files, manages a playlist queue, and uses mpv
//! subprocess for playback via IPC socket (same pattern as media_player.rs).

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::App;

// Re-export the Slint struct
use crate::MusicTrackData;

/// A scanned music track.
#[derive(Clone, Debug)]
pub struct MusicTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: f64,
    pub path: PathBuf,
}

impl MusicTrack {
    /// Parse metadata from filename: "Artist - Title.ext" or just "Title.ext".
    fn from_path(path: PathBuf) -> Self {
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let (artist, title) = if let Some((a, t)) = stem.split_once(" - ") {
            (a.trim().to_string(), t.trim().to_string())
        } else {
            ("Unknown Artist".to_string(), stem)
        };

        // Try to derive album from parent directory name
        let album = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Don't use "Music" as album name
        let album = if album == "Music" || album == "music" {
            String::new()
        } else {
            album
        };

        Self {
            title,
            artist,
            album,
            duration_secs: 0.0,
            path,
        }
    }

    fn to_slint(&self, is_current: bool) -> MusicTrackData {
        MusicTrackData {
            title: self.title.clone().into(),
            artist: self.artist.clone().into(),
            album: self.album.clone().into(),
            duration_text: format_time(self.duration_secs).into(),
            path: self.path.display().to_string().into(),
            is_current,
        }
    }
}

/// Scan directories for audio files.
fn scan_music_dirs() -> Vec<MusicTrack> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let music_dirs = vec![
        PathBuf::from(&home).join("Music"),
        PathBuf::from(&home).join("music"),
        PathBuf::from("/home/yantrik/Music"),
    ];

    let audio_exts: HashSet<&str> =
        ["mp3", "flac", "ogg", "wav", "m4a", "aac", "wma", "opus"]
            .iter()
            .copied()
            .collect();

    let mut tracks = Vec::new();

    for dir in &music_dirs {
        if dir.is_dir() {
            scan_dir_recursive(dir, &audio_exts, &mut tracks);
        }
    }

    // Sort by artist, then album, then title
    tracks.sort_by(|a, b| {
        a.artist
            .to_lowercase()
            .cmp(&b.artist.to_lowercase())
            .then(a.album.to_lowercase().cmp(&b.album.to_lowercase()))
            .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    tracing::info!(count = tracks.len(), "Music library scanned");
    tracks
}

fn scan_dir_recursive(dir: &Path, audio_exts: &HashSet<&str>, tracks: &mut Vec<MusicTrack>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_recursive(&path, audio_exts, tracks);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if audio_exts.contains(ext.to_lowercase().as_str()) {
                tracks.push(MusicTrack::from_path(path));
            }
        }
    }
}

/// Handle to mpv running in music-player mode.
struct MusicMpvHandle {
    child: Child,
    socket_path: String,
}

impl MusicMpvHandle {
    fn spawn(file_path: &str) -> Option<Self> {
        let socket_path = "/tmp/yantrik-mpv-music.sock".to_string();
        let _ = std::fs::remove_file(&socket_path);

        match Command::new("mpv")
            .arg("--no-video")
            .arg("--no-terminal")
            .arg(format!("--input-ipc-server={}", socket_path))
            .arg(file_path)
            .spawn()
        {
            Ok(child) => {
                tracing::info!(path = file_path, "Music mpv spawned");
                Some(Self { child, socket_path })
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to spawn mpv for music");
                None
            }
        }
    }

    fn send_command(&self, command: &[&str]) -> Option<String> {
        let mut stream = UnixStream::connect(&self.socket_path).ok()?;
        let cmd = serde_json::json!({ "command": command });
        let mut msg = serde_json::to_string(&cmd).ok()?;
        msg.push('\n');
        stream.write_all(msg.as_bytes()).ok()?;
        stream.flush().ok()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response).ok()?;
        Some(response)
    }

    fn get_property(&self, prop: &str) -> Option<serde_json::Value> {
        let mut stream = UnixStream::connect(&self.socket_path).ok()?;
        let cmd = serde_json::json!({ "command": ["get_property", prop] });
        let mut msg = serde_json::to_string(&cmd).ok()?;
        msg.push('\n');
        stream.write_all(msg.as_bytes()).ok()?;
        stream.flush().ok()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&response).ok()?;
        parsed.get("data").cloned()
    }

    fn toggle_pause(&self) {
        self.send_command(&["cycle", "pause"]);
    }

    fn stop(&mut self) {
        self.send_command(&["quit"]);
        let _ = self.child.wait();
    }

    fn seek_percent(&self, percent: f64) {
        let pct_str = format!("{:.1}", percent * 100.0);
        self.send_command(&["seek", &pct_str, "absolute-percent"]);
    }

    fn set_volume(&self, volume: f64) {
        let vol_str = format!("{:.0}", volume * 100.0);
        self.send_command(&["set_property_string", "volume", &vol_str]);
    }

    fn percent_pos(&self) -> f64 {
        self.get_property("percent-pos")
            .and_then(|v| v.as_f64())
            .map(|p| p / 100.0)
            .unwrap_or(0.0)
    }

    fn time_pos(&self) -> f64 {
        self.get_property("time-pos")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    fn duration(&self) -> f64 {
        self.get_property("duration")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    fn is_paused(&self) -> bool {
        self.get_property("pause")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for MusicMpvHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Music player state, shared between callbacks.
struct MusicPlayerState {
    library: Vec<MusicTrack>,
    queue: Vec<MusicTrack>,
    current_index: i32, // -1 = nothing playing
    mpv: Option<MusicMpvHandle>,
    shuffle: bool,
    repeat: bool,
    volume: f64,
    /// Current browse filter (artist or album name)
    browse_sub: String,
}

impl MusicPlayerState {
    fn new(library: Vec<MusicTrack>) -> Self {
        Self {
            library,
            queue: Vec::new(),
            current_index: -1,
            mpv: None,
            shuffle: false,
            repeat: false,
            volume: 0.8,
            browse_sub: String::new(),
        }
    }

    /// Start playing a track by queue index.
    fn play_queue_index(&mut self, idx: usize) {
        if idx >= self.queue.len() {
            return;
        }
        if !super::dep_check::has_command("mpv") {
            tracing::warn!("mpv not installed — music playback unavailable (apk add mpv)");
            return;
        }

        // Stop current playback
        if let Some(ref mut mpv) = self.mpv {
            mpv.stop();
        }

        self.current_index = idx as i32;
        let path = self.queue[idx].path.display().to_string();
        self.mpv = MusicMpvHandle::spawn(&path);

        // Set volume on new instance
        if let Some(ref mpv) = self.mpv {
            // Small delay for mpv to initialize IPC
            std::thread::sleep(std::time::Duration::from_millis(200));
            mpv.set_volume(self.volume);
        }
    }

    /// Add all library tracks to queue and start playing from the given index.
    fn play_from_library(&mut self, library_idx: usize) {
        self.queue = self.library.clone();
        if self.shuffle {
            self.shuffle_queue_except(library_idx);
        }
        let play_idx = if self.shuffle { 0 } else { library_idx };
        self.play_queue_index(play_idx);
    }

    /// Play a filtered set of tracks (e.g., artist/album subset).
    fn play_from_filtered(&mut self, filtered: &[MusicTrack], idx: usize) {
        self.queue = filtered.to_vec();
        if self.shuffle {
            self.shuffle_queue_except(idx);
        }
        let play_idx = if self.shuffle { 0 } else { idx };
        self.play_queue_index(play_idx);
    }

    /// Next track in queue.
    fn next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let next_idx = if self.current_index < 0 {
            0
        } else {
            let n = self.current_index as usize + 1;
            if n >= self.queue.len() {
                if self.repeat {
                    0
                } else {
                    return; // End of queue
                }
            } else {
                n
            }
        };
        self.play_queue_index(next_idx);
    }

    /// Previous track in queue.
    fn prev(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let prev_idx = if self.current_index <= 0 {
            if self.repeat {
                self.queue.len() - 1
            } else {
                0
            }
        } else {
            self.current_index as usize - 1
        };
        self.play_queue_index(prev_idx);
    }

    /// Shuffle queue, putting the given index first.
    fn shuffle_queue_except(&mut self, first_idx: usize) {
        if self.queue.is_empty() {
            return;
        }
        let first = self.queue.remove(first_idx);
        // Simple shuffle using timestamp as seed
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let len = self.queue.len();
        if len > 1 {
            let mut rng = seed;
            for i in (1..len).rev() {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let j = (rng % (i as u128 + 1)) as usize;
                self.queue.swap(i, j);
            }
        }
        self.queue.insert(0, first);
    }

    /// Get unique artists from library.
    fn artists(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut artists = Vec::new();
        for t in &self.library {
            if t.artist != "Unknown Artist" && seen.insert(t.artist.clone()) {
                artists.push(t.artist.clone());
            }
        }
        artists.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        artists
    }

    /// Get unique albums from library.
    fn albums(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut albums = Vec::new();
        for t in &self.library {
            if !t.album.is_empty() && seen.insert(t.album.clone()) {
                albums.push(t.album.clone());
            }
        }
        albums.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        albums
    }

    /// Filter library by search query.
    fn filter_library(&self, query: &str) -> Vec<MusicTrack> {
        if query.is_empty() && self.browse_sub.is_empty() {
            return self.library.clone();
        }

        let q = query.to_lowercase();
        self.library
            .iter()
            .filter(|t| {
                // Apply sub-filter (artist or album drill-down)
                if !self.browse_sub.is_empty() {
                    if t.artist != self.browse_sub && t.album != self.browse_sub {
                        return false;
                    }
                }
                if q.is_empty() {
                    return true;
                }
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }

    /// Current playing track path (for marking is_current in UI).
    fn current_path(&self) -> Option<String> {
        if self.current_index >= 0 && (self.current_index as usize) < self.queue.len() {
            Some(self.queue[self.current_index as usize].path.display().to_string())
        } else {
            None
        }
    }
}

/// Format seconds as M:SS.
fn format_time(secs: f64) -> String {
    if secs <= 0.0 {
        return "0:00".to_string();
    }
    let total = secs as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{}:{:02}", m, s)
}

/// Sync the library tracks model to the UI.
fn sync_library_to_ui(state: &MusicPlayerState, ui: &App, category: i32, filter: &str) {
    let current_path = state.current_path();
    let filtered = state.filter_library(filter);

    let items: Vec<MusicTrackData> = filtered
        .iter()
        .map(|t| {
            let is_current = current_path
                .as_ref()
                .map(|cp| cp == &t.path.display().to_string())
                .unwrap_or(false);
            t.to_slint(is_current)
        })
        .collect();

    ui.set_music_library_tracks(ModelRc::new(VecModel::from(items)));
    ui.set_music_library_track_count(filtered.len() as i32);

    // Update artists and albums lists
    let artists: Vec<SharedString> = state.artists().into_iter().map(|a| a.into()).collect();
    ui.set_music_library_artists(ModelRc::new(VecModel::from(artists)));

    let albums: Vec<SharedString> = state.albums().into_iter().map(|a| a.into()).collect();
    ui.set_music_library_albums(ModelRc::new(VecModel::from(albums)));
}

/// Sync the queue model to the UI.
fn sync_queue_to_ui(state: &MusicPlayerState, ui: &App) {
    let current_path = state.current_path();
    let items: Vec<MusicTrackData> = state
        .queue
        .iter()
        .map(|t| {
            let is_current = current_path
                .as_ref()
                .map(|cp| cp == &t.path.display().to_string())
                .unwrap_or(false);
            t.to_slint(is_current)
        })
        .collect();

    ui.set_music_queue_tracks(ModelRc::new(VecModel::from(items)));
    ui.set_music_queue_current_index(state.current_index);
}

/// Update now-playing metadata on the UI.
fn sync_now_playing(state: &MusicPlayerState, ui: &App) {
    if state.current_index >= 0 && (state.current_index as usize) < state.queue.len() {
        let track = &state.queue[state.current_index as usize];
        ui.set_music_now_title(track.title.clone().into());
        ui.set_music_now_artist(track.artist.clone().into());
        ui.set_music_now_album(track.album.clone().into());
    } else {
        ui.set_music_now_title("".into());
        ui.set_music_now_artist("".into());
        ui.set_music_now_album("".into());
    }
    ui.set_music_shuffle_on(state.shuffle);
    ui.set_music_repeat_on(state.repeat);
    ui.set_music_volume(state.volume as f32);
}

/// Wire all music player callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // Scan library
    let library = scan_music_dirs();

    let state = Rc::new(RefCell::new(MusicPlayerState::new(library)));

    // Initial sync
    {
        let s = state.borrow();
        sync_library_to_ui(&s, ui, 0, "");
        sync_queue_to_ui(&s, ui);
        sync_now_playing(&s, ui);
    }

    // Browse filter / category
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_browse_filter_changed(move |filter| {
        if let Some(ui) = ui_weak.upgrade() {
            let s = st.borrow();
            let cat = ui.get_music_browse_category();
            sync_library_to_ui(&s, &ui, cat, filter.as_str());
        }
    });

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_browse_category_changed(move |cat| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.browse_sub.clear();
            ui.set_music_browse_sub_header("".into());
            sync_library_to_ui(&s, &ui, cat, "");
        }
    });

    // Browse item selected (artist/album drill-down)
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_browse_item_selected(move |name| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let name_str = name.to_string();
            s.browse_sub = name_str.clone();
            ui.set_music_browse_sub_header(name.clone());
            let filter = ui.get_music_browse_filter();
            sync_library_to_ui(&s, &ui, ui.get_music_browse_category(), filter.as_str());
        }
    });

    // Play from library
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_play_track_index(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();

            // Get the filtered view to know which tracks to queue
            let filter = ui.get_music_browse_filter();
            let filtered = s.filter_library(filter.as_str());

            if (idx as usize) < filtered.len() {
                s.play_from_filtered(&filtered, idx as usize);
                sync_queue_to_ui(&s, &ui);
                sync_now_playing(&s, &ui);
                ui.set_music_is_playing(true);
                // Also refresh library to show current highlight
                let cat = ui.get_music_browse_category();
                sync_library_to_ui(&s, &ui, cat, filter.as_str());
            }
        }
    });

    // Queue play index
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_queue_play_index(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.play_queue_index(idx as usize);
            sync_queue_to_ui(&s, &ui);
            sync_now_playing(&s, &ui);
            ui.set_music_is_playing(true);
            let filter = ui.get_music_browse_filter();
            let cat = ui.get_music_browse_category();
            sync_library_to_ui(&s, &ui, cat, filter.as_str());
        }
    });

    // Play/Pause
    let st = state.clone();
    ui.on_music_play_pause(move || {
        let s = st.borrow();
        if let Some(ref mpv) = s.mpv {
            mpv.toggle_pause();
        }
    });

    // Stop
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_stop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            if let Some(ref mut mpv) = s.mpv {
                mpv.stop();
            }
            s.mpv = None;
            s.current_index = -1;
            ui.set_music_is_playing(false);
            ui.set_music_progress(0.0);
            ui.set_music_time_current("0:00".into());
            sync_now_playing(&s, &ui);
            sync_queue_to_ui(&s, &ui);
        }
    });

    // Next
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_next_track(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.next();
            sync_queue_to_ui(&s, &ui);
            sync_now_playing(&s, &ui);
            ui.set_music_is_playing(s.mpv.is_some());
            let filter = ui.get_music_browse_filter();
            let cat = ui.get_music_browse_category();
            sync_library_to_ui(&s, &ui, cat, filter.as_str());
        }
    });

    // Previous
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_prev_track(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.prev();
            sync_queue_to_ui(&s, &ui);
            sync_now_playing(&s, &ui);
            ui.set_music_is_playing(s.mpv.is_some());
            let filter = ui.get_music_browse_filter();
            let cat = ui.get_music_browse_category();
            sync_library_to_ui(&s, &ui, cat, filter.as_str());
        }
    });

    // Seek
    let st = state.clone();
    ui.on_music_seek(move |pos| {
        let s = st.borrow();
        if let Some(ref mpv) = s.mpv {
            mpv.seek_percent(pos as f64);
        }
    });

    // Volume
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_volume_changed(move |vol| {
        let mut s = st.borrow_mut();
        s.volume = vol as f64;
        if let Some(ref mpv) = s.mpv {
            mpv.set_volume(vol as f64);
        }
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_music_volume(vol);
        }
    });

    // Shuffle toggle
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_toggle_shuffle(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.shuffle = !s.shuffle;
            ui.set_music_shuffle_on(s.shuffle);
        }
    });

    // Repeat toggle
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_toggle_repeat(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.repeat = !s.repeat;
            ui.set_music_repeat_on(s.repeat);
        }
    });

    // Add to queue
    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_add_to_queue(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let filter = ui.get_music_browse_filter();
            let filtered = s.filter_library(filter.as_str());
            if (idx as usize) < filtered.len() {
                let track = filtered[idx as usize].clone();
                s.queue.push(track);
                sync_queue_to_ui(&s, &ui);
            }
        }
    });

    // Progress polling timer (500ms)
    let st = state.clone();
    let ui_weak = ui.as_weak();
    let poll_timer = Timer::default();
    poll_timer.start(
        TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            // First pass: check if alive and get playback info
            let poll_result = {
                let mut s = st.borrow_mut();
                if s.mpv.is_none() {
                    return;
                }

                let mpv = s.mpv.as_mut().unwrap();
                if !mpv.is_alive() {
                    // Track ended — mark for auto-advance
                    s.mpv = None;
                    None // signals "track ended"
                } else {
                    let pos = mpv.percent_pos();
                    let time = mpv.time_pos();
                    let dur = mpv.duration();
                    let paused = mpv.is_paused();

                    // Update track duration in queue if we got it from mpv
                    if dur > 0.0 && s.current_index >= 0 {
                        let idx = s.current_index as usize;
                        if idx < s.queue.len() && s.queue[idx].duration_secs == 0.0 {
                            s.queue[idx].duration_secs = dur;
                        }
                    }

                    Some((pos, time, dur, paused))
                }
            };
            // borrow is dropped here

            match poll_result {
                None => {
                    // Track ended — auto-advance
                    let mut s = st.borrow_mut();
                    let should_next = s.current_index >= 0;
                    if should_next {
                        s.next();
                    }

                    if let Some(ui) = ui_weak.upgrade() {
                        let playing = s.mpv.is_some();
                        ui.set_music_is_playing(playing);
                        if !playing {
                            ui.set_music_progress(0.0);
                            ui.set_music_time_current("0:00".into());
                        }
                        sync_queue_to_ui(&s, &ui);
                        sync_now_playing(&s, &ui);
                    }
                }
                Some((pos, time, dur, paused)) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_music_progress(pos as f32);
                        ui.set_music_time_current(format_time(time).into());
                        ui.set_music_time_total(format_time(dur).into());
                        ui.set_music_is_playing(!paused);
                    }
                }
            }
        },
    );

    // Keep timer alive
    std::mem::forget(poll_timer);
}
