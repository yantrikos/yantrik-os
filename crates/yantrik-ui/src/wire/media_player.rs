//! Media Player wiring — mpv audio backend via IPC.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::rc::Rc;

use slint::{ComponentHandle, Timer, TimerMode};

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

/// Wire media player callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let player = ctx.media_player.clone();

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

    // Progress polling timer (500ms)
    let pl = player.clone();
    let ui_weak = ui.as_weak();
    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, std::time::Duration::from_millis(500), move || {
        let mut handle = pl.borrow_mut();
        if let Some(ref mut mpv) = *handle {
            if !mpv.is_alive() {
                // Playback ended naturally
                drop(handle);
                *pl.borrow_mut() = None;
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_player_is_playing(false);
                    ui.set_player_progress(0.0);
                    ui.set_player_time_current("0:00".into());
                }
                return;
            }

            let pos = mpv.percent_pos();
            let time = mpv.time_pos();
            let dur = mpv.duration();
            let paused = mpv.is_paused();

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

/// Start playback of an audio file. Call when navigating to screen 13.
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
