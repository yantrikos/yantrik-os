//! Screenshot capture — grim/slurp integration for Wayland.
//!
//! Captures full-screen or region screenshots and saves them to
//! ~/Pictures/Screenshots/ with timestamp filenames, or pipes directly
//! to the Wayland clipboard via `wl-copy`.
//!
//! Keybinds (configured in labwc rc.xml via `keybinds.rs`):
//! - Print         -> full screen capture (save to file)
//! - Shift+Print   -> region selection (save to file)
//! - Ctrl+Print    -> full screen capture (copy to clipboard)
//! - Ctrl+S+Print  -> region selection (copy to clipboard)

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// How to capture the screenshot.
pub enum CaptureMode {
    /// Capture the entire screen via `grim` and save to file.
    FullScreen,
    /// Let the user select a region via `slurp`, then capture and save to file.
    Region,
    /// Capture the entire screen and pipe directly to clipboard via `wl-copy`.
    ClipboardFull,
    /// Let the user select a region, then pipe to clipboard via `wl-copy`.
    ClipboardRegion,
}

/// Capture a screenshot — either saving to file or copying to clipboard.
///
/// For file modes, returns the absolute path of the saved file.
/// For clipboard modes, returns "Copied to clipboard".
pub fn capture(mode: CaptureMode) -> Result<String, String> {
    match mode {
        CaptureMode::FullScreen => capture_to_file(None),
        CaptureMode::Region => {
            let geometry = get_region()?;
            capture_to_file(Some(&geometry))
        }
        CaptureMode::ClipboardFull => capture_to_clipboard(None),
        CaptureMode::ClipboardRegion => {
            let geometry = get_region()?;
            capture_to_clipboard(Some(&geometry))
        }
    }
}

/// Run `slurp` to let the user select a screen region.
/// Returns the geometry string (e.g. "100,200 400x300").
fn get_region() -> Result<String, String> {
    let output = Command::new("slurp")
        .output()
        .map_err(|e| format!("slurp not found: {e}"))?;

    if !output.status.success() {
        return Err("Region selection cancelled".to_string());
    }

    let geometry = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();

    if geometry.is_empty() {
        return Err("slurp returned empty geometry".to_string());
    }

    Ok(geometry)
}

/// Capture a screenshot to ~/Pictures/Screenshots/.
/// If `geometry` is Some, captures that region; otherwise captures full screen.
fn capture_to_file(geometry: Option<&str>) -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let dir = PathBuf::from(&home).join("Pictures/Screenshots");

    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;

    let filename = format!("screenshot-{}.png", timestamp());
    let dest = dir.join(&filename);
    let dest_str = dest.to_string_lossy().to_string();

    let mut cmd = Command::new("grim");
    cmd.args(["-t", "png"]);
    if let Some(g) = geometry {
        cmd.args(["-g", g]);
    }
    cmd.arg(&dest_str);

    let output = cmd.output().map_err(|e| format!("grim not found: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "grim failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    tracing::info!(path = %dest_str, "Screenshot saved");
    Ok(dest_str)
}

/// Capture a screenshot and pipe it directly to the clipboard via `wl-copy`.
/// Uses `grim -t png - | wl-copy --type image/png` (no intermediate file).
fn capture_to_clipboard(geometry: Option<&str>) -> Result<String, String> {
    let mut grim_cmd = Command::new("grim");
    grim_cmd.args(["-t", "png"]);
    if let Some(g) = geometry {
        grim_cmd.args(["-g", g]);
    }
    grim_cmd.arg("-"); // write PNG to stdout
    grim_cmd.stdout(Stdio::piped());

    let mut grim_child = grim_cmd
        .spawn()
        .map_err(|e| format!("grim not found: {e}"))?;

    let grim_stdout = grim_child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture grim stdout".to_string())?;

    let wl_copy_child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(grim_stdout)
        .spawn()
        .map_err(|e| format!("wl-copy not found: {e}"))?;

    // Wait for both processes to finish
    let grim_status = grim_child
        .wait()
        .map_err(|e| format!("grim process error: {e}"))?;
    if !grim_status.success() {
        return Err("grim failed during clipboard capture".to_string());
    }

    let wl_copy_output = wl_copy_child
        .wait_with_output()
        .map_err(|e| format!("wl-copy process error: {e}"))?;
    if !wl_copy_output.status.success() {
        return Err(format!(
            "wl-copy failed: {}",
            String::from_utf8_lossy(&wl_copy_output.stderr)
        ));
    }

    tracing::info!("Screenshot copied to clipboard");
    Ok("Copied to clipboard".to_string())
}

/// Generate a timestamp string like `2026-03-04-143022`.
fn timestamp() -> String {
    // Parse seconds since epoch into date/time components.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Simple UTC breakdown (no chrono dependency).
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01 to Y-M-D (civil calendar).
    let (year, month, day) = days_to_date(days);

    format!(
        "{:04}-{:02}-{:02}-{:02}{:02}{:02}",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from Howard Hinnant's `civil_from_days`.
fn days_to_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // day of era
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}
