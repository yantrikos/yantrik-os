//! Media Player wiring — mpv audio backend via IPC.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::App;

/// Handle to a running mpv instance.
pub struct MpvHandle {
    child: Child,
    socket_path: String,
}

impl MpvHandle {
    /// Spawn mpv in audio-only mode with IPC socket.
    pub fn spawn(file_path: &str) -> Option<Self> {
        let pid = std::process::id();
        let socket_path = format!("/tmp/yantrik-mpv-{}", pid);

        // Clean up old socket
        let _ = std::fs::remove_file(&socket_path);

        match Command::new("mpv")
            .arg("--no-video")
            .arg("--no-terminal")
            .arg(format!("--input-ipc-server={}", socket_path))
            .arg(file_path)
            .spawn()
        {
            Ok(child) => {
                tracing::info!(path = file_path, socket = %socket_path, "mpv spawned");
                Some(Self {
                    child,
                    socket_path,
                })
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to spawn mpv");
                None
            }
        }
    }

    /// Send a JSON IPC command to mpv.
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

    /// Get a property value from mpv.
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

    pub fn toggle_pause(&self) {
        self.send_command(&["cycle", "pause"]);
    }

    pub fn stop(&mut self) {
        self.send_command(&["quit"]);
        let _ = self.child.wait();
    }

    pub fn seek_percent(&self, percent: f64) {
        let pct_str = format!("{:.1}", percent * 100.0);
        self.send_command(&["seek", &pct_str, "absolute-percent"]);
    }

    pub fn set_volume(&self, volume: f64) {
        let vol_str = format!("{:.0}", volume * 100.0);
        self.send_command(&["set_property", "volume", &vol_str]);
    }

    pub fn set_mute(&self, mute: bool) {
        self.send_command(&["set_property", "mute", if mute { "yes" } else { "no" }]);
    }

    pub fn set_speed(&self, speed: f64) {
        let speed_str = format!("{:.2}", speed);
        self.send_command(&["set_property", "speed", &speed_str]);
    }

    /// Get current playback position as 0.0–1.0.
    pub fn percent_pos(&self) -> f64 {
        self.get_property("percent-pos")
            .and_then(|v| v.as_f64())
            .map(|p| p / 100.0)
            .unwrap_or(0.0)
    }

    /// Get current time position in seconds.
    pub fn time_pos(&self) -> f64 {
        self.get_property("time-pos")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    /// Get total duration in seconds.
    pub fn duration(&self) -> f64 {
        self.get_property("duration")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    /// Check if mpv is paused.
    pub fn is_paused(&self) -> bool {
        self.get_property("pause")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    /// Check if mpv process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Set a property on mpv (JSON value variant).
    pub fn set_property_json(&self, prop: &str, value: serde_json::Value) -> Option<String> {
        let mut stream = UnixStream::connect(&self.socket_path).ok()?;
        let cmd = serde_json::json!({ "command": ["set_property", prop, value] });
        let mut msg = serde_json::to_string(&cmd).ok()?;
        msg.push('\n');
        stream.write_all(msg.as_bytes()).ok()?;
        stream.flush().ok()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response).ok()?;
        Some(response)
    }

    /// Get the number of subtitle tracks.
    pub fn subtitle_track_count(&self) -> i64 {
        self.get_property("track-list/count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    }

    /// Get subtitle track list (titles).
    pub fn subtitle_tracks(&self) -> Vec<String> {
        let count = self.subtitle_track_count();
        let mut subs = Vec::new();
        for i in 0..count {
            let type_prop = format!("track-list/{}/type", i);
            let is_sub = self
                .get_property(&type_prop)
                .and_then(|v| v.as_str().map(|s| s == "sub"))
                .unwrap_or(false);
            if is_sub {
                let title_prop = format!("track-list/{}/title", i);
                let lang_prop = format!("track-list/{}/lang", i);
                let title = self
                    .get_property(&title_prop)
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let lang = self
                    .get_property(&lang_prop)
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let label = if !title.is_empty() && !lang.is_empty() {
                    format!("{} ({})", title, lang)
                } else if !title.is_empty() {
                    title
                } else if !lang.is_empty() {
                    lang
                } else {
                    format!("Track {}", subs.len() + 1)
                };
                subs.push(label);
            }
        }
        subs
    }

    /// Select a subtitle track. -1 = disable subtitles.
    pub fn select_subtitle(&self, track_idx: i32) {
        if track_idx < 0 {
            self.set_property_json("sid", serde_json::json!("no"));
        } else {
            // mpv subtitle IDs are 1-based; we map our 0-based index
            self.set_property_json("sid", serde_json::json!(track_idx + 1));
        }
    }
}

impl Drop for MpvHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Format seconds as M:SS.
fn format_time(secs: f64) -> String {
    let total = secs as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{}:{:02}", m, s)
}

/// A-B repeat state.
struct AbRepeatState {
    active: bool,
    start: f64, // 0.0-1.0 progress
    end: f64,   // 0.0-1.0 progress
}

impl Default for AbRepeatState {
    fn default() -> Self {
        Self {
            active: false,
            start: 0.0,
            end: 1.0,
        }
    }
}

/// Wire media player callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let player = ctx.media_player.clone();
    let ab_state = Rc::new(RefCell::new(AbRepeatState::default()));

    // Play/Pause
    let pl = player.clone();
    ui.on_player_play_pause(move || {
        if let Some(ref handle) = *pl.borrow() {
            handle.toggle_pause();
        }
    });

    // Stop
    let pl = player.clone();
    let ui_weak = ui.as_weak();
    ui.on_player_stop(move || {
        if let Some(ref mut handle) = *pl.borrow_mut() {
            handle.stop();
        }
        *pl.borrow_mut() = None;
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_player_is_playing(false);
            ui.set_player_progress(0.0);
            ui.set_player_time_current("0:00".into());
        }
    });

    // Seek
    let pl = player.clone();
    ui.on_player_seek(move |pos| {
        if let Some(ref handle) = *pl.borrow() {
            handle.seek_percent(pos as f64);
        }
    });

    // Volume
    let pl = player.clone();
    ui.on_player_set_volume(move |vol| {
        if let Some(ref handle) = *pl.borrow() {
            handle.set_volume(vol as f64);
        }
    });

    // Mute toggle
    let pl = player.clone();
    let ui_weak_mute = ui.as_weak();
    ui.on_player_toggle_mute(move || {
        if let Some(ui) = ui_weak_mute.upgrade() {
            let muted = !ui.get_player_is_muted();
            ui.set_player_is_muted(muted);
            if let Some(ref handle) = *pl.borrow() {
                handle.set_mute(muted);
            }
        }
    });

    // Speed
    let pl = player.clone();
    let ui_weak_speed = ui.as_weak();
    ui.on_player_set_speed(move |idx| {
        let speeds = [0.5, 0.75, 0.9, 1.0, 1.25, 1.5, 2.0];
        let labels = ["0.5x", "0.75x", "0.9x", "1.0x", "1.25x", "1.5x", "2.0x"];
        let idx = (idx as usize).min(speeds.len() - 1);
        if let Some(ref handle) = *pl.borrow() {
            handle.set_speed(speeds[idx]);
        }
        if let Some(ui) = ui_weak_speed.upgrade() {
            ui.set_player_speed_text(labels[idx].into());
        }
    });

    // Next/prev track (stub — no playlist yet in media player)
    ui.on_player_next_track(move || {
        tracing::debug!("Next track — not implemented for media player");
    });
    ui.on_player_prev_track(move || {
        tracing::debug!("Prev track — not implemented for media player");
    });

    // Repeat toggle
    let ui_weak_rep = ui.as_weak();
    ui.on_player_toggle_repeat(move || {
        if let Some(ui) = ui_weak_rep.upgrade() {
            let mode = (ui.get_player_repeat_mode() + 1) % 3;
            ui.set_player_repeat_mode(mode);
        }
    });

    // Shuffle toggle
    let ui_weak_shuf = ui.as_weak();
    ui.on_player_toggle_shuffle(move || {
        if let Some(ui) = ui_weak_shuf.upgrade() {
            ui.set_player_shuffle_on(!ui.get_player_shuffle_on());
        }
    });

    // Subtitle selection
    let pl = player.clone();
    let ui_weak_sub = ui.as_weak();
    ui.on_player_select_subtitle(move |track_idx| {
        if let Some(ref handle) = *pl.borrow() {
            handle.select_subtitle(track_idx);
        }
        if let Some(ui) = ui_weak_sub.upgrade() {
            ui.set_player_subtitle_active(track_idx);
        }
    });

    // A-B repeat: set A point
    let ab = ab_state.clone();
    let ui_weak_ab_a = ui.as_weak();
    ui.on_player_set_ab_start(move || {
        if let Some(ui) = ui_weak_ab_a.upgrade() {
            let pos = ui.get_player_progress();
            let mut st = ab.borrow_mut();
            st.start = pos as f64;
            ui.set_player_ab_start(pos);
        }
    });

    // A-B repeat: set B point
    let ab = ab_state.clone();
    let ui_weak_ab_b = ui.as_weak();
    ui.on_player_set_ab_end(move || {
        if let Some(ui) = ui_weak_ab_b.upgrade() {
            let pos = ui.get_player_progress();
            let mut st = ab.borrow_mut();
            st.end = pos as f64;
            ui.set_player_ab_end(pos);
        }
    });

    // A-B repeat: toggle
    let ab = ab_state.clone();
    let ui_weak_ab_t = ui.as_weak();
    ui.on_player_toggle_ab_repeat(move || {
        if let Some(ui) = ui_weak_ab_t.upgrade() {
            let mut st = ab.borrow_mut();
            st.active = !st.active;
            ui.set_player_ab_repeat_active(st.active);
            if !st.active {
                // Reset markers
                st.start = 0.0;
                st.end = 1.0;
                ui.set_player_ab_start(0.0);
                ui.set_player_ab_end(1.0);
            }
        }
    });

    // AI explain (stub)
    ui.on_player_ai_explain(move || {
        tracing::debug!("Media player AI explain — not implemented");
    });
    ui.on_player_ai_dismiss(move || {
        tracing::debug!("Media player AI dismiss");
    });

    // Progress polling timer (500ms)
    let pl = player.clone();
    let ab = ab_state.clone();
    let ui_weak = ui.as_weak();
    let subtitle_fetched = Rc::new(RefCell::new(false));
    let sub_fetched = subtitle_fetched.clone();
    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, std::time::Duration::from_millis(500), move || {
        let mut handle = pl.borrow_mut();
        if let Some(ref mut mpv) = *handle {
            if !mpv.is_alive() {
                // Playback ended naturally
                drop(handle);
                *pl.borrow_mut() = None;
                *sub_fetched.borrow_mut() = false;
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_player_is_playing(false);
                    ui.set_player_progress(0.0);
                    ui.set_player_time_current("0:00".into());
                    ui.set_player_subtitle_tracks(ModelRc::new(VecModel::<SharedString>::default()));
                    ui.set_player_subtitle_active(-1);
                }
                return;
            }

            let pos = mpv.percent_pos();
            let time = mpv.time_pos();
            let dur = mpv.duration();
            let paused = mpv.is_paused();

            // Reset subtitle fetch flag when a new file starts (time near 0)
            if *sub_fetched.borrow() && time < 1.0 && dur > 0.0 {
                *sub_fetched.borrow_mut() = false;
            }

            // Fetch subtitle tracks once after playback starts
            if !*sub_fetched.borrow() && dur > 0.0 {
                *sub_fetched.borrow_mut() = true;
                let subs = mpv.subtitle_tracks();
                if let Some(ui) = ui_weak.upgrade() {
                    let items: Vec<SharedString> = subs.into_iter().map(|s| s.into()).collect();
                    ui.set_player_subtitle_tracks(ModelRc::new(VecModel::from(items)));
                    ui.set_player_subtitle_active(-1);
                }
            }

            // A-B repeat: if active and position > B, seek to A
            {
                let ab_st = ab.borrow();
                if ab_st.active && pos > ab_st.end {
                    mpv.seek_percent(ab_st.start);
                }
            }

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_player_progress(pos as f32);
                ui.set_player_time_current(format_time(time).into());
                ui.set_player_time_total(format_time(dur).into());
                ui.set_player_is_playing(!paused);
            }
        }
    });

    // Keep the timer alive by storing it (it will be dropped when the app exits)
    std::mem::forget(poll_timer);
}

/// Start playback of a media file. Call when navigating to screen 13.
pub fn start_playback(
    ui: &App,
    path: &PathBuf,
    player: &Rc<RefCell<Option<MpvHandle>>>,
) {
    if !super::dep_check::has_command("mpv") {
        ui.set_player_track_name("mpv not installed".into());
        ui.set_player_is_playing(false);
        tracing::warn!("mpv not installed — media playback unavailable (apk add mpv)");
        return;
    }

    // Stop existing playback
    if let Some(ref mut old) = *player.borrow_mut() {
        old.stop();
    }

    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    ui.set_player_track_name(name.into());
    ui.set_player_progress(0.0);
    ui.set_player_time_current("0:00".into());
    ui.set_player_time_total("0:00".into());
    ui.set_player_is_playing(true);

    let path_str = path.display().to_string();
    match MpvHandle::spawn(&path_str) {
        Some(handle) => {
            *player.borrow_mut() = Some(handle);
        }
        None => {
            ui.set_player_is_playing(false);
            tracing::error!("Failed to start mpv playback");
        }
    }
}
