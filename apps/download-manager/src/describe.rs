//! The Mind's view of Download Manager: a projection of the downloads as the engine holds them, in
//! the same keys and words the app has always answered, so `yos check download-manager` reads the
//! same.
//!
//! The engine is read live, not copied into the model: the workers move bytes between the
//! window's looks, and a caller asking how far a download has got wants the engine's word.

use serde_json::Value;
use yantrik_app::control::View;

use crate::engine::{self, Download, Status, Totals};
use crate::model::Model;
use crate::update::calls::row_json;

/// How many rows `describe` carries. The state is a glance, not a transcript: the true count
/// travels beside it, so a caller is never misled about how many there are.
pub const LISTING_CAP: usize = 20;

pub fn view(m: &Model) -> View {
    let items = m.items();
    let totals = Totals::of(&items);
    let shown: Vec<Value> = items.iter().rev().take(LISTING_CAP).map(row_json).collect();
    View::new(summary(&items, totals))
        .with("downloads", Value::Array(shown))
        .with("shown", items.len().min(LISTING_CAP) as i64)
        .with("total", items.len() as i64)
        .with("active", totals.active as i64)
        .with("paused", totals.paused as i64)
        .with("completed", totals.completed as i64)
        .with("failed", totals.failed as i64)
        .with("missing", totals.missing as i64)
        // How much of this list survived a restart, and where it survived in. A caller that finds
        // `restored` rows knows the window is not starting from nothing, and `state_file` is where
        // to look when it disagrees with what it expected.
        .with("restored", totals.restored as i64)
        .with("state_file", m.engine.state_path().to_string_lossy().to_string())
        .with("notice", m.engine.notice())
        .with("speed", format!("{}/s", engine::format_bytes(totals.speed_bps as u64)))
        .with("save_dir", m.engine.default_dir().to_string_lossy().to_string())
}

/// One line a person could read, with whatever is worth knowing first.
pub fn summary(items: &[Download], totals: Totals) -> String {
    if items.is_empty() {
        return "Downloads \u{2014} nothing queued".to_string();
    }
    // Trouble leads, the same way a failed service leads the shell's summary.
    if let Some(failed) = items.iter().rev().find(|d| d.status == Status::Failed) {
        let reason = if failed.error.is_empty() { "failed".into() } else { failed.error.clone() };
        return format!(
            "Downloads \u{2014} {} failed ({}: {}), {} active",
            totals.failed, failed.filename, reason, totals.active
        );
    }
    // A file that was fetched and is now gone outranks everything below: it is the one state the
    // person cannot discover by looking at the window's progress bars.
    if totals.missing > 0 {
        let gone = items.iter().find(|d| d.status == Status::Missing);
        return format!(
            "Downloads \u{2014} {} finished {} no longer on disk{}",
            totals.missing,
            engine::plural(totals.missing, "download is", "downloads are"),
            gone.map(|d| format!(" ({})", d.filename)).unwrap_or_default()
        );
    }
    if let Some(mismatch) = items.iter().find(|d| d.checksum_status == "fail") {
        return format!("Downloads \u{2014} {} downloaded but its checksum does not match", mismatch.filename);
    }
    if totals.active > 0 {
        let leader = items
            .iter()
            .find(|d| d.status == Status::Downloading)
            .map(|d| {
                if d.total.is_some() {
                    format!(", {} at {}%", d.filename, (d.progress() * 100.0).round() as i32)
                } else {
                    format!(", {} ({} so far)", d.filename, engine::format_bytes(d.downloaded))
                }
            })
            .unwrap_or_default();
        return format!(
            "Downloads \u{2014} {} active at {}/s{leader}",
            totals.active,
            engine::format_bytes(totals.speed_bps as u64)
        );
    }
    if totals.paused > 0 {
        // "Paused" and "was running when the app died" are the same row to the engine and very
        // different news to whoever left it running.
        let interrupted = items.iter().filter(|d| d.interrupted).count();
        let held = if interrupted > 0 {
            format!("{} paused ({interrupted} interrupted by a restart)", totals.paused)
        } else {
            format!("{} paused", totals.paused)
        };
        return format!("Downloads \u{2014} {held}, {} finished", totals.completed);
    }
    format!("Downloads \u{2014} {} finished, nothing running", totals.completed)
}
