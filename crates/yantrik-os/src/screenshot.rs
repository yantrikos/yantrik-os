//! Screenshot capture — grim/slurp integration for Wayland.
//!
//! Captures full-screen or region screenshots and saves them to
//! ~/Pictures/Screenshots/ with timestamp filenames.
//!
//! Keybinds (configured in labwc rc.xml via `keybinds.rs`):
//! - Print       -> full screen capture
//! - Shift+Print -> region selection (via slurp)

use std::path::PathBuf;
use std::process::Command;

/// How to capture the screenshot.
pub enum CaptureMode {
    /// Capture the entire screen via `grim`.
    FullScreen,
    /// Let the user select a region via `slurp`, then capture it with `grim -g`.
    Region,
}

/// Capture a screenshot and save it to ~/Pictures/Screenshots/.
///
/// Returns the absolute path of the saved file on success.
pub fn capture(mode: CaptureMode) -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let dir = PathBuf::from(&home).join("Pictures/Screenshots");

    // Ensure the directory exists
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;

    // Generate timestamp filename: screenshot-2026-03-04-143022.png
    let filename = format!("screenshot-{}.png", timestamp());
    let dest = dir.join(&filename);
    let dest_str = dest.to_string_lossy().to_string();

    match mode {
        CaptureMode::FullScreen => {
            let output = Command::new("grim")
                .args(["-t", "png", &dest_str])
                .output()
                .map_err(|e| format!("grim not found: {e}"))?;

            if !output.status.success() {
                return Err(format!(
                    "grim failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        CaptureMode::Region => {
            // First, get the region from slurp
            let slurp_output = Command::new("slurp")
                .output()
                .map_err(|e| format!("slurp not found: {e}"))?;

            if !slurp_output.status.success() {
                // slurp exits non-zero when the user cancels (presses Escape)
                return Err("Region selection cancelled".to_string());
            }

            let geometry = String::from_utf8_lossy(&slurp_output.stdout)
                .trim()
                .to_string();

            if geometry.is_empty() {
                return Err("slurp returned empty geometry".to_string());
            }

            // Capture the selected region
            let output = Command::new("grim")
                .args(["-t", "png", "-g", &geometry, &dest_str])
                .output()
                .map_err(|e| format!("grim not found: {e}"))?;

            if !output.status.success() {
                return Err(format!(
                    "grim failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
    }

    tracing::info!(path = %dest_str, "Screenshot saved");
    Ok(dest_str)
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
