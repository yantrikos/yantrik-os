//! Music player service contract — library, playlists, playback control.

use serde::{Deserialize, Serialize};
use crate::email::ServiceError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub duration_secs: f64,
    pub path: String,
    pub format: String,
    pub bitrate: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub track_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    pub is_playing: bool,
    pub current_track: Option<Track>,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub volume: f64,
    pub is_muted: bool,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

/// Music player service operations.
pub trait MusicService: Send + Sync {
    // Library
    fn scan_library(&self, folders: Vec<String>) -> Result<Vec<Track>, ServiceError>;
    fn list_tracks(&self, filter: Option<&str>) -> Result<Vec<Track>, ServiceError>;

    // Playlists
    fn list_playlists(&self) -> Result<Vec<Playlist>, ServiceError>;
    fn create_playlist(&self, name: &str) -> Result<Playlist, ServiceError>;
    fn delete_playlist(&self, playlist_id: &str) -> Result<(), ServiceError>;
    fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>, ServiceError>;
    fn add_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<(), ServiceError>;
    fn remove_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<(), ServiceError>;

    // Playback
    fn play(&self, track_id: &str) -> Result<(), ServiceError>;
    fn pause(&self) -> Result<(), ServiceError>;
    fn resume(&self) -> Result<(), ServiceError>;
    fn stop(&self) -> Result<(), ServiceError>;
    fn next(&self) -> Result<(), ServiceError>;
    fn previous(&self) -> Result<(), ServiceError>;
    fn seek(&self, position_secs: f64) -> Result<(), ServiceError>;
    fn set_volume(&self, volume: f64) -> Result<(), ServiceError>;
    fn set_shuffle(&self, enabled: bool) -> Result<(), ServiceError>;
    fn set_repeat(&self, mode: RepeatMode) -> Result<(), ServiceError>;
    fn playback_state(&self) -> Result<PlaybackState, ServiceError>;
}
