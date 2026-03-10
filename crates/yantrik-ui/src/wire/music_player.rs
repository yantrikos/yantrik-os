//! Music Player wiring — library scanner, playlist management, queue, mpv playback.
//!
//! Enterprise-grade music player with:
//! - Multi-folder library scanning with genre/format detection
//! - Playlist CRUD (create, rename, delete, add/remove tracks)
//! - Queue management (add, remove, reorder, play next, clear)
//! - Playback: shuffle, repeat (off/one/all), speed control, mute
//! - Track metadata: genre, format, bitrate via mpv property queries

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

// Re-export the Slint structs
use crate::{MusicAlbumData, MusicArtistData, MusicGenreData, MusicPlaylistData, MusicScanFolderData, MusicTrackData};

/// A scanned music track.
#[derive(Clone, Debug)]
pub struct MusicTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub duration_secs: f64,
    pub path: PathBuf,
    pub format_info: String,
    pub bitrate: String,
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

        // Derive format from extension
        let format_info = path
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .unwrap_or_default();

        Self {
            title,
            artist,
            album,
            genre: String::new(),
            duration_secs: 0.0,
            path,
            format_info,
            bitrate: String::new(),
        }
    }

    fn to_slint(&self, is_current: bool) -> MusicTrackData {
        MusicTrackData {
            title: self.title.clone().into(),
            artist: self.artist.clone().into(),
            album: self.album.clone().into(),
            genre: self.genre.clone().into(),
            duration_text: format_time(self.duration_secs).into(),
            duration_secs: self.duration_secs as f32,
            path: self.path.display().to_string().into(),
            is_current,
            format_info: self.format_info.clone().into(),
            bitrate: self.bitrate.clone().into(),
        }
    }
}

/// A user-created playlist.
#[derive(Clone, Debug)]
struct Playlist {
    name: String,
    tracks: Vec<MusicTrack>,
}

impl Playlist {
    fn new(name: String) -> Self {
        Self {
            name,
            tracks: Vec::new(),
        }
    }

    fn total_duration_secs(&self) -> f64 {
        self.tracks.iter().map(|t| t.duration_secs).sum()
    }

    fn to_slint(&self, is_active: bool) -> MusicPlaylistData {
        MusicPlaylistData {
            name: self.name.clone().into(),
            track_count: self.tracks.len() as i32,
            total_duration: format_duration_long(self.total_duration_secs()).into(),
            is_active,
        }
    }
}

/// A scan folder entry.
#[derive(Clone, Debug)]
struct ScanFolder {
    path: PathBuf,
    track_count: usize,
}

impl ScanFolder {
    fn to_slint(&self, is_scanning: bool) -> MusicScanFolderData {
        MusicScanFolderData {
            path: self.path.display().to_string().into(),
            track_count: self.track_count as i32,
            is_scanning,
        }
    }
}

/// Scan directories for audio files.
fn scan_music_dirs(folders: &[ScanFolder]) -> Vec<MusicTrack> {
    let audio_exts: HashSet<&str> =
        ["mp3", "flac", "ogg", "wav", "m4a", "aac", "wma", "opus"]
            .iter()
            .copied()
            .collect();

    let mut tracks = Vec::new();

    for folder in folders {
        if folder.path.is_dir() {
            scan_dir_recursive(&folder.path, &audio_exts, &mut tracks);
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

/// Get default scan folders.
fn default_scan_folders() -> Vec<ScanFolder> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let mut folders = Vec::new();

    for dir_name in &["Music", "music"] {
        let p = PathBuf::from(&home).join(dir_name);
        if p.is_dir() {
            folders.push(ScanFolder {
                path: p,
                track_count: 0,
            });
        }
    }

    let yantrik_music = PathBuf::from("/home/yantrik/Music");
    if yantrik_music.is_dir() {
        folders.push(ScanFolder {
            path: yantrik_music,
            track_count: 0,
        });
    }

    // If no folders found, add ~/Music anyway so user sees it
    if folders.is_empty() {
        folders.push(ScanFolder {
            path: PathBuf::from(&home).join("Music"),
            track_count: 0,
        });
    }

    folders
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

    fn set_speed(&self, speed: f64) {
        let speed_str = format!("{:.2}", speed);
        self.send_command(&["set_property_string", "speed", &speed_str]);
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

    /// Get audio bitrate from mpv (returns bps or 0).
    fn audio_bitrate(&self) -> f64 {
        self.get_property("audio-params/samplerate")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    /// Get audio codec name.
    fn audio_codec(&self) -> String {
        self.get_property("audio-codec-name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    /// Get file format info string.
    fn audio_format_info(&self) -> (String, String) {
        let codec = self.audio_codec();
        let samplerate = self.audio_bitrate();

        let format_str = if !codec.is_empty() && samplerate > 0.0 {
            format!("{} {:.1}kHz", codec.to_uppercase(), samplerate / 1000.0)
        } else if !codec.is_empty() {
            codec.to_uppercase()
        } else {
            String::new()
        };

        let bitrate = self
            .get_property("audio-bitrate")
            .and_then(|v| v.as_f64())
            .map(|b| {
                if b > 0.0 {
                    format!("{:.0} kbps", b / 1000.0)
                } else {
                    String::new()
                }
            })
            .unwrap_or_default();

        (format_str, bitrate)
    }
}

impl Drop for MusicMpvHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Repeat mode enum matching Slint's MusicRepeatMode.
#[derive(Clone, Copy, PartialEq, Debug)]
enum RepeatMode {
    Off = 0,
    One = 1,
    All = 2,
}

impl RepeatMode {
    fn from_int(v: i32) -> Self {
        match v {
            1 => Self::One,
            2 => Self::All,
            _ => Self::Off,
        }
    }

    fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// Music player state, shared between callbacks.
struct MusicPlayerState {
    library: Vec<MusicTrack>,
    queue: Vec<MusicTrack>,
    current_index: i32, // -1 = nothing playing
    mpv: Option<MusicMpvHandle>,
    shuffle: bool,
    repeat: RepeatMode,
    volume: f64,
    volume_muted: bool,
    volume_before_mute: f64,
    playback_speed: f64,
    /// Current browse filter (artist, album, or genre name)
    browse_sub: String,
    /// Playlists
    playlists: Vec<Playlist>,
    /// Active playlist index (-1 = all music)
    active_playlist: i32,
    /// Scan folders
    scan_folders: Vec<ScanFolder>,
    /// Whether metadata has been fetched for current track
    metadata_fetched: bool,
}

impl MusicPlayerState {
    fn new(library: Vec<MusicTrack>, scan_folders: Vec<ScanFolder>) -> Self {
        Self {
            library,
            queue: Vec::new(),
            current_index: -1,
            mpv: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            volume: 0.8,
            volume_muted: false,
            volume_before_mute: 0.8,
            playback_speed: 1.0,
            browse_sub: String::new(),
            playlists: Vec::new(),
            active_playlist: -1,
            scan_folders,
            metadata_fetched: false,
        }
    }

    /// Get the effective library (all music or active playlist).
    fn effective_library(&self) -> Vec<MusicTrack> {
        if self.active_playlist >= 0 && (self.active_playlist as usize) < self.playlists.len() {
            self.playlists[self.active_playlist as usize].tracks.clone()
        } else {
            self.library.clone()
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
        self.metadata_fetched = false;
        let path = self.queue[idx].path.display().to_string();
        self.mpv = MusicMpvHandle::spawn(&path);

        // Set volume and speed on new instance
        if let Some(ref mpv) = self.mpv {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let effective_vol = if self.volume_muted { 0.0 } else { self.volume };
            mpv.set_volume(effective_vol);
            if (self.playback_speed - 1.0).abs() > 0.01 {
                mpv.set_speed(self.playback_speed);
            }
        }
    }

    /// Add all library tracks to queue and start playing from the given index.
    fn play_from_library(&mut self, library_idx: usize) {
        self.queue = self.effective_library();
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

        // Repeat one: replay current track
        if self.repeat == RepeatMode::One && self.current_index >= 0 {
            self.play_queue_index(self.current_index as usize);
            return;
        }

        let next_idx = if self.current_index < 0 {
            0
        } else {
            let n = self.current_index as usize + 1;
            if n >= self.queue.len() {
                if self.repeat == RepeatMode::All {
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
            if self.repeat == RepeatMode::All {
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

    /// Get unique artists from library with counts.
    fn artists(&self) -> Vec<(String, usize, usize)> {
        let lib = self.effective_library();
        let mut artist_map: std::collections::HashMap<String, (HashSet<String>, usize)> =
            std::collections::HashMap::new();

        for t in &lib {
            if t.artist != "Unknown Artist" {
                let entry = artist_map
                    .entry(t.artist.clone())
                    .or_insert((HashSet::new(), 0));
                if !t.album.is_empty() {
                    entry.0.insert(t.album.clone());
                }
                entry.1 += 1;
            }
        }

        let mut artists: Vec<(String, usize, usize)> = artist_map
            .into_iter()
            .map(|(name, (albums, tracks))| (name, albums.len(), tracks))
            .collect();
        artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        artists
    }

    /// Get unique albums from library with metadata.
    fn albums(&self) -> Vec<(String, String, usize)> {
        let lib = self.effective_library();
        let mut album_map: std::collections::HashMap<String, (String, usize)> =
            std::collections::HashMap::new();

        for t in &lib {
            if !t.album.is_empty() {
                let entry = album_map
                    .entry(t.album.clone())
                    .or_insert((t.artist.clone(), 0));
                entry.1 += 1;
            }
        }

        let mut albums: Vec<(String, String, usize)> = album_map
            .into_iter()
            .map(|(name, (artist, count))| (name, artist, count))
            .collect();
        albums.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        albums
    }

    /// Get unique genres from library with counts.
    fn genres(&self) -> Vec<(String, usize)> {
        let lib = self.effective_library();
        let mut genre_map: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for t in &lib {
            if !t.genre.is_empty() {
                *genre_map.entry(t.genre.clone()).or_insert(0) += 1;
            }
        }

        // If no genres, derive from folder structure (grandparent dir)
        if genre_map.is_empty() {
            for t in &lib {
                let genre = t
                    .path
                    .parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !genre.is_empty()
                    && genre != "Music"
                    && genre != "music"
                    && genre != "home"
                    && genre != "yantrik"
                    && genre != "root"
                {
                    *genre_map.entry(genre).or_insert(0) += 1;
                }
            }
        }

        let mut genres: Vec<(String, usize)> = genre_map.into_iter().collect();
        genres.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        genres
    }

    /// Filter library by search query.
    fn filter_library(&self, query: &str) -> Vec<MusicTrack> {
        let lib = self.effective_library();

        if query.is_empty() && self.browse_sub.is_empty() {
            return lib;
        }

        let q = query.to_lowercase();
        lib.iter()
            .filter(|t| {
                // Apply sub-filter (artist, album, or genre drill-down)
                if !self.browse_sub.is_empty() {
                    if t.artist != self.browse_sub
                        && t.album != self.browse_sub
                        && t.genre != self.browse_sub
                    {
                        // Also check derived genre from grandparent dir
                        let derived_genre = t
                            .path
                            .parent()
                            .and_then(|p| p.parent())
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if derived_genre != self.browse_sub {
                            return false;
                        }
                    }
                }
                if q.is_empty() {
                    return true;
                }
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
                    || t.genre.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }

    /// Current playing track path (for marking is_current in UI).
    fn current_path(&self) -> Option<String> {
        if self.current_index >= 0 && (self.current_index as usize) < self.queue.len() {
            Some(
                self.queue[self.current_index as usize]
                    .path
                    .display()
                    .to_string(),
            )
        } else {
            None
        }
    }

    /// Rescan all folders and rebuild library.
    fn rescan(&mut self) {
        // Update track counts per folder
        let audio_exts: HashSet<&str> = ["mp3", "flac", "ogg", "wav", "m4a", "aac", "wma", "opus"]
            .iter()
            .copied()
            .collect();

        for folder in &mut self.scan_folders {
            let mut count = 0;
            if folder.path.is_dir() {
                count_audio_files(&folder.path, &audio_exts, &mut count);
            }
            folder.track_count = count;
        }

        self.library = scan_music_dirs(&self.scan_folders);
    }
}

/// Count audio files in a directory (for folder track counts).
fn count_audio_files(dir: &Path, audio_exts: &HashSet<&str>, count: &mut usize) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count_audio_files(&path, audio_exts, count);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if audio_exts.contains(ext.to_lowercase().as_str()) {
                *count += 1;
            }
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

/// Format duration as "Xh Ym" or "Ym" for playlist totals.
fn format_duration_long(secs: f64) -> String {
    if secs <= 0.0 {
        return "0m".to_string();
    }
    let total = secs as u64;
    let hours = total / 3600;
    let mins = (total % 3600) / 60;
    if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

/// Sync the library tracks model to the UI.
fn sync_library_to_ui(state: &MusicPlayerState, ui: &App, _category: i32, filter: &str) {
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

    // Update artists list with counts
    let artists_data: Vec<MusicArtistData> = state
        .artists()
        .into_iter()
        .map(|(name, album_count, track_count)| MusicArtistData {
            name: name.into(),
            album_count: album_count as i32,
            track_count: track_count as i32,
        })
        .collect();
    ui.set_music_library_artists(ModelRc::new(VecModel::from(artists_data)));

    // Update albums list with metadata
    let albums_data: Vec<MusicAlbumData> = state
        .albums()
        .into_iter()
        .map(|(name, artist, track_count)| MusicAlbumData {
            name: name.into(),
            artist: artist.into(),
            track_count: track_count as i32,
            year: SharedString::new(),
        })
        .collect();
    ui.set_music_library_albums(ModelRc::new(VecModel::from(albums_data)));

    // Update genres list with counts
    let genres_data: Vec<MusicGenreData> = state
        .genres()
        .into_iter()
        .map(|(name, count)| MusicGenreData {
            name: name.into(),
            track_count: count as i32,
        })
        .collect();
    ui.set_music_library_genres(ModelRc::new(VecModel::from(genres_data)));
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
        ui.set_music_now_genre(track.genre.clone().into());
        ui.set_music_now_format_info(track.format_info.clone().into());
        ui.set_music_now_bitrate(track.bitrate.clone().into());
    } else {
        ui.set_music_now_title("".into());
        ui.set_music_now_artist("".into());
        ui.set_music_now_album("".into());
        ui.set_music_now_genre("".into());
        ui.set_music_now_format_info("".into());
        ui.set_music_now_bitrate("".into());
    }
    ui.set_music_shuffle_on(state.shuffle);
    ui.set_music_repeat_mode(state.repeat as i32);
    ui.set_music_volume(state.volume as f32);
    ui.set_music_volume_muted(state.volume_muted);
    ui.set_music_playback_speed(state.playback_speed as f32);
}

/// Sync playlist list to UI.
fn sync_playlists_to_ui(state: &MusicPlayerState, ui: &App) {
    let items: Vec<MusicPlaylistData> = state
        .playlists
        .iter()
        .enumerate()
        .map(|(i, pl)| pl.to_slint(i as i32 == state.active_playlist))
        .collect();
    ui.set_music_playlists(ModelRc::new(VecModel::from(items)));
    ui.set_music_active_playlist_index(state.active_playlist);
}

/// Sync scan folders to UI.
fn sync_scan_folders_to_ui(state: &MusicPlayerState, ui: &App) {
    let items: Vec<MusicScanFolderData> = state
        .scan_folders
        .iter()
        .map(|f| f.to_slint(false))
        .collect();
    ui.set_music_scan_folders(ModelRc::new(VecModel::from(items)));
}

/// Wire all music player callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    // Set up scan folders and scan library
    let scan_folders = default_scan_folders();
    let library = scan_music_dirs(&scan_folders);

    let state = Rc::new(RefCell::new(MusicPlayerState::new(library, scan_folders)));

    // Initial sync
    {
        let s = state.borrow();
        sync_library_to_ui(&s, ui, 0, "");
        sync_queue_to_ui(&s, ui);
        sync_now_playing(&s, ui);
        sync_playlists_to_ui(&s, ui);
        sync_scan_folders_to_ui(&s, ui);
    }

    // ── Browse filter / category ──

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

    // Browse item selected (artist/album/genre drill-down)
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

    // ── Play from library ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_play_track_index(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let filter = ui.get_music_browse_filter();
            let filtered = s.filter_library(filter.as_str());

            if (idx as usize) < filtered.len() {
                s.play_from_filtered(&filtered, idx as usize);
                sync_queue_to_ui(&s, &ui);
                sync_now_playing(&s, &ui);
                ui.set_music_is_playing(true);
                let cat = ui.get_music_browse_category();
                sync_library_to_ui(&s, &ui, cat, filter.as_str());
            }
        }
    });

    // ── Queue play index ──

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

    // ── Play/Pause ──

    let st = state.clone();
    ui.on_music_play_pause(move || {
        let s = st.borrow();
        if let Some(ref mpv) = s.mpv {
            mpv.toggle_pause();
        }
    });

    // ── Stop ──

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

    // ── Next / Previous ──

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

    // ── Seek ──

    let st = state.clone();
    ui.on_music_seek(move |pos| {
        let s = st.borrow();
        if let Some(ref mpv) = s.mpv {
            mpv.seek_percent(pos as f64);
        }
    });

    // ── Volume ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_volume_changed(move |vol| {
        let mut s = st.borrow_mut();
        s.volume = vol as f64;
        s.volume_muted = false;
        if let Some(ref mpv) = s.mpv {
            mpv.set_volume(vol as f64);
        }
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_music_volume(vol);
            ui.set_music_volume_muted(false);
        }
    });

    // ── Mute toggle ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_toggle_mute(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.volume_muted = !s.volume_muted;
            if s.volume_muted {
                s.volume_before_mute = s.volume;
                if let Some(ref mpv) = s.mpv {
                    mpv.set_volume(0.0);
                }
            } else {
                s.volume = s.volume_before_mute;
                if let Some(ref mpv) = s.mpv {
                    mpv.set_volume(s.volume);
                }
            }
            ui.set_music_volume_muted(s.volume_muted);
            ui.set_music_volume(s.volume as f32);
        }
    });

    // ── Shuffle toggle ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_toggle_shuffle(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.shuffle = !s.shuffle;
            ui.set_music_shuffle_on(s.shuffle);
        }
    });

    // ── Repeat cycle ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_cycle_repeat(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.repeat = s.repeat.cycle();
            ui.set_music_repeat_mode(s.repeat as i32);
        }
    });

    // ── Playback speed ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_set_playback_speed(move |speed| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.playback_speed = speed as f64;
            if let Some(ref mpv) = s.mpv {
                mpv.set_speed(speed as f64);
            }
            ui.set_music_playback_speed(speed);
        }
    });

    // ── Add to queue ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_add_to_queue(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let filter = ui.get_music_browse_filter();
            let filtered = s.filter_library(filter.as_str());

            if idx == -1 {
                // Add all filtered tracks
                for track in &filtered {
                    s.queue.push(track.clone());
                }
            } else if (idx as usize) < filtered.len() {
                let track = filtered[idx as usize].clone();
                s.queue.push(track);
            }
            sync_queue_to_ui(&s, &ui);
        }
    });

    // ── Play next (insert after current) ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_play_next(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let filter = ui.get_music_browse_filter();
            let filtered = s.filter_library(filter.as_str());

            if (idx as usize) < filtered.len() {
                let track = filtered[idx as usize].clone();
                let insert_pos = if s.current_index >= 0 {
                    (s.current_index as usize + 1).min(s.queue.len())
                } else {
                    0
                };
                s.queue.insert(insert_pos, track);
                sync_queue_to_ui(&s, &ui);
            }
        }
    });

    // ── Queue remove ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_queue_remove(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let idx = idx as usize;
            if idx < s.queue.len() {
                s.queue.remove(idx);
                // Adjust current index if needed
                if s.current_index >= 0 {
                    let ci = s.current_index as usize;
                    if idx < ci {
                        s.current_index -= 1;
                    } else if idx == ci {
                        // Current track removed; stop or play next
                        s.current_index = -1;
                    }
                }
                sync_queue_to_ui(&s, &ui);
            }
        }
    });

    // ── Queue move up ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_queue_move_up(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let idx = idx as usize;
            if idx > 0 && idx < s.queue.len() {
                s.queue.swap(idx, idx - 1);
                // Adjust current index
                if s.current_index == idx as i32 {
                    s.current_index -= 1;
                } else if s.current_index == (idx as i32 - 1) {
                    s.current_index += 1;
                }
                sync_queue_to_ui(&s, &ui);
            }
        }
    });

    // ── Queue move down ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_queue_move_down(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let idx = idx as usize;
            if idx + 1 < s.queue.len() {
                s.queue.swap(idx, idx + 1);
                // Adjust current index
                if s.current_index == idx as i32 {
                    s.current_index += 1;
                } else if s.current_index == (idx as i32 + 1) {
                    s.current_index -= 1;
                }
                sync_queue_to_ui(&s, &ui);
            }
        }
    });

    // ── Queue clear ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_queue_clear(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            // Stop playback
            if let Some(ref mut mpv) = s.mpv {
                mpv.stop();
            }
            s.mpv = None;
            s.queue.clear();
            s.current_index = -1;
            ui.set_music_is_playing(false);
            ui.set_music_progress(0.0);
            ui.set_music_time_current("0:00".into());
            sync_queue_to_ui(&s, &ui);
            sync_now_playing(&s, &ui);
        }
    });

    // ── Playlist: create ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_playlist_create(move |name| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.playlists.push(Playlist::new(name.to_string()));
            sync_playlists_to_ui(&s, &ui);
        }
    });

    // ── Playlist: rename ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_playlist_rename(move |idx, new_name| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let idx = idx as usize;
            if idx < s.playlists.len() && !new_name.is_empty() {
                s.playlists[idx].name = new_name.to_string();
                sync_playlists_to_ui(&s, &ui);
            }
        }
    });

    // ── Playlist: delete ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_playlist_delete(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let idx = idx as usize;
            if idx < s.playlists.len() {
                s.playlists.remove(idx);
                if s.active_playlist == idx as i32 {
                    s.active_playlist = -1;
                } else if s.active_playlist > idx as i32 {
                    s.active_playlist -= 1;
                }
                sync_playlists_to_ui(&s, &ui);
                sync_library_to_ui(&s, &ui, 0, "");
            }
        }
    });

    // ── Playlist: select ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_playlist_select(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.active_playlist = idx;
            s.browse_sub.clear();
            ui.set_music_browse_sub_header("".into());
            sync_playlists_to_ui(&s, &ui);
            let filter = ui.get_music_browse_filter();
            let cat = ui.get_music_browse_category();
            sync_library_to_ui(&s, &ui, cat, filter.as_str());
        }
    });

    // ── Playlist: add track ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_playlist_add_track(move |pl_idx, track_idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let pl_idx = pl_idx as usize;
            let track_idx = track_idx as usize;
            if pl_idx < s.playlists.len() && track_idx < s.library.len() {
                let track = s.library[track_idx].clone();
                s.playlists[pl_idx].tracks.push(track);
                sync_playlists_to_ui(&s, &ui);
            }
        }
    });

    // ── Playlist: remove track ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_playlist_remove_track(move |pl_idx, track_idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let pl_idx = pl_idx as usize;
            let track_idx = track_idx as usize;
            if pl_idx < s.playlists.len() && track_idx < s.playlists[pl_idx].tracks.len() {
                s.playlists[pl_idx].tracks.remove(track_idx);
                sync_playlists_to_ui(&s, &ui);
                if s.active_playlist == pl_idx as i32 {
                    let filter = ui.get_music_browse_filter();
                    let cat = ui.get_music_browse_category();
                    sync_library_to_ui(&s, &ui, cat, filter.as_str());
                }
            }
        }
    });

    // ── Scan: add folder ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_scan_add_folder(move |path| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let p = PathBuf::from(path.to_string());
            // Don't add duplicates
            if !s.scan_folders.iter().any(|f| f.path == p) {
                s.scan_folders.push(ScanFolder {
                    path: p,
                    track_count: 0,
                });
                s.rescan();
                sync_scan_folders_to_ui(&s, &ui);
                sync_library_to_ui(&s, &ui, 0, "");
            }
        }
    });

    // ── Scan: remove folder ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_scan_remove_folder(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            let idx = idx as usize;
            if idx < s.scan_folders.len() {
                s.scan_folders.remove(idx);
                s.rescan();
                sync_scan_folders_to_ui(&s, &ui);
                sync_library_to_ui(&s, &ui, 0, "");
            }
        }
    });

    // ── Scan: rescan ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    ui.on_music_scan_rescan(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut s = st.borrow_mut();
            s.rescan();
            sync_scan_folders_to_ui(&s, &ui);
            let filter = ui.get_music_browse_filter();
            let cat = ui.get_music_browse_category();
            sync_library_to_ui(&s, &ui, cat, filter.as_str());
        }
    });

    // ── Progress polling timer (500ms) ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    let poll_timer = Timer::default();
    poll_timer.start(
        TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            let poll_result = {
                let mut s = st.borrow_mut();
                if s.mpv.is_none() {
                    return;
                }

                let need_meta = !s.metadata_fetched;
                let cur_idx = s.current_index;
                let mpv = s.mpv.as_mut().unwrap();
                if !mpv.is_alive() {
                    s.mpv = None;
                    None // signals "track ended"
                } else {
                    let pos = mpv.percent_pos();
                    let time = mpv.time_pos();
                    let dur = mpv.duration();
                    let paused = mpv.is_paused();
                    let meta = if need_meta {
                        let (format_info, bitrate) = mpv.audio_format_info();
                        Some((format_info, bitrate))
                    } else {
                        None
                    };
                    // Drop mpv borrow before accessing other fields
                    drop(mpv);

                    if need_meta {
                        s.metadata_fetched = true;
                    }

                    // Update track duration in queue if we got it from mpv
                    if dur > 0.0 && cur_idx >= 0 {
                        let idx = cur_idx as usize;
                        if idx < s.queue.len() && s.queue[idx].duration_secs == 0.0 {
                            s.queue[idx].duration_secs = dur;
                        }
                    }

                    Some((pos, time, dur, paused, meta))
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
                Some((pos, time, dur, paused, meta)) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_music_progress(pos as f32);
                        ui.set_music_time_current(format_time(time).into());
                        ui.set_music_time_total(format_time(dur).into());
                        ui.set_music_is_playing(!paused);

                        // Update format/bitrate metadata
                        if let Some((format_info, bitrate)) = meta {
                            if !format_info.is_empty() {
                                ui.set_music_now_format_info(format_info.into());
                                // Also update in state
                                let mut s = st.borrow_mut();
                                if s.current_index >= 0 {
                                    let idx = s.current_index as usize;
                                    if idx < s.queue.len() {
                                        s.queue[idx].format_info = ui.get_music_now_format_info().to_string();
                                    }
                                }
                            }
                            if !bitrate.is_empty() {
                                ui.set_music_now_bitrate(bitrate.into());
                                let mut s = st.borrow_mut();
                                if s.current_index >= 0 {
                                    let idx = s.current_index as usize;
                                    if idx < s.queue.len() {
                                        s.queue[idx].bitrate = ui.get_music_now_bitrate().to_string();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    );

    // Keep timer alive
    std::mem::forget(poll_timer);

    // ── Folder watch toggle ──

    let st = state.clone();
    let ui_weak = ui.as_weak();
    let folder_watch_active = Rc::new(RefCell::new(false));
    let fw_active = folder_watch_active.clone();
    ui.on_music_toggle_folder_watch(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut active = fw_active.borrow_mut();
            *active = !*active;
            ui.set_music_folder_watch_active(*active);
            if *active {
                ui.set_music_folder_watch_status("Watching for changes...".into());
                tracing::info!("Music folder watch enabled");
            } else {
                ui.set_music_folder_watch_status("".into());
                tracing::info!("Music folder watch disabled");
            }
        }
    });

    // Folder watch polling timer (60s) — checks for new files when active
    let st_fw = state.clone();
    let ui_weak_fw = ui.as_weak();
    let fw_active2 = folder_watch_active.clone();
    let fw_timer = Timer::default();
    fw_timer.start(
        TimerMode::Repeated,
        std::time::Duration::from_secs(60),
        move || {
            if !*fw_active2.borrow() {
                return;
            }
            let mut s = st_fw.borrow_mut();
            let old_count = s.library.len();
            s.rescan();
            let new_count = s.library.len();
            if new_count != old_count {
                if let Some(ui) = ui_weak_fw.upgrade() {
                    let diff = new_count as i64 - old_count as i64;
                    let status = if diff > 0 {
                        format!("Found {} new tracks", diff)
                    } else {
                        format!("{} tracks removed", -diff)
                    };
                    ui.set_music_folder_watch_status(status.into());
                    sync_scan_folders_to_ui(&s, &ui);
                    let filter = ui.get_music_browse_filter();
                    let cat = ui.get_music_browse_category();
                    sync_library_to_ui(&s, &ui, cat, filter.as_str());
                }
            }
        },
    );
    std::mem::forget(fw_timer);

    // ── Equalizer preset ──

    let ui_weak = ui.as_weak();
    ui.on_music_set_equalizer(move |preset| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_music_equalizer_preset(preset);
            let label = match preset {
                0 => "Flat",
                1 => "Bass Boost",
                2 => "Vocal",
                3 => "Treble Boost",
                _ => "Flat",
            };
            tracing::info!(preset = label, "Equalizer preset selected (UI-only)");
        }
    });

    // ── AI Explain callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_music_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let title = ui.get_music_now_title().to_string();
        let artist = ui.get_music_now_artist().to_string();
        let album = ui.get_music_now_album().to_string();

        if title.is_empty() {
            return;
        }

        let context = format!(
            "Now playing: {} by {}\nAlbum: {}",
            title,
            if artist.is_empty() {
                "Unknown"
            } else {
                &artist
            },
            if album.is_empty() {
                "Unknown"
            } else {
                &album
            }
        );
        let prompt = super::ai_assist::music_info_prompt(&context);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_music_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_music_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_music_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_music_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_music_ai_panel_open(false);
        }
    });
}
