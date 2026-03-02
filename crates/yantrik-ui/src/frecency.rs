//! Frecency store — tracks which Lens actions the user picks most often.
//!
//! Uses the Firefox Awesome Bar algorithm: each use gets a weight based on
//! how recently it happened.  The sum of weights is the frecency score.
//!
//! Storage: in-memory `HashMap`, persisted to `~/.yantrik/lens_frecency.json`
//! every 30 seconds (never blocks UI thread).

use std::collections::HashMap;
use std::path::PathBuf;

/// Maximum timestamps kept per entry (ring buffer).
const MAX_TIMESTAMPS: usize = 20;

/// A single frecency record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FrecencyEntry {
    pub action_id: String,
    pub title: String,
    pub result_type: String,
    pub icon_char: String,
    pub timestamps: Vec<f64>,
    pub use_count: u32,
}

impl FrecencyEntry {
    /// Compute frecency score from timestamps.
    pub fn score(&self, now: f64) -> f64 {
        self.timestamps.iter().map(|&t| weight(now - t)).sum()
    }
}

/// Weight based on age in seconds.
fn weight(age_secs: f64) -> f64 {
    if age_secs < 300.0 {
        10.0 // last 5 minutes
    } else if age_secs < 3600.0 {
        5.0 // last hour
    } else if age_secs < 86400.0 {
        2.0 // last day
    } else if age_secs < 604800.0 {
        1.0 // last week
    } else {
        0.5 // older
    }
}

/// In-memory frecency store, periodically flushed to disk.
pub struct FrecencyStore {
    entries: HashMap<String, FrecencyEntry>,
    dirty: bool,
}

impl FrecencyStore {
    /// Load from `~/.yantrik/lens_frecency.json`, or create empty.
    pub fn load() -> Self {
        let path = store_path();
        let entries = match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        tracing::info!(entries = entries.len(), "Frecency store loaded");
        Self {
            entries,
            dirty: false,
        }
    }

    /// Record a selection. Call after every Lens action dispatch.
    pub fn record(&mut self, action_id: &str, title: &str, result_type: &str, icon_char: &str) {
        let now = unix_now();
        let entry = self
            .entries
            .entry(action_id.to_string())
            .or_insert_with(|| FrecencyEntry {
                action_id: action_id.to_string(),
                title: title.to_string(),
                result_type: result_type.to_string(),
                icon_char: icon_char.to_string(),
                timestamps: Vec::new(),
                use_count: 0,
            });
        // Update metadata (title may change for same action_id)
        entry.title = title.to_string();
        entry.result_type = result_type.to_string();
        entry.icon_char = icon_char.to_string();
        entry.use_count += 1;
        entry.timestamps.push(now);
        // Ring buffer: keep last MAX_TIMESTAMPS
        if entry.timestamps.len() > MAX_TIMESTAMPS {
            let excess = entry.timestamps.len() - MAX_TIMESTAMPS;
            entry.timestamps.drain(..excess);
        }
        self.dirty = true;
    }

    /// Get the frecency score for a specific action_id.
    pub fn score(&self, action_id: &str) -> f64 {
        let now = unix_now();
        self.entries
            .get(action_id)
            .map(|e| e.score(now))
            .unwrap_or(0.0)
    }

    /// Return the top N entries by frecency score (highest first).
    pub fn top_n(&self, n: usize) -> Vec<&FrecencyEntry> {
        let now = unix_now();
        let mut sorted: Vec<_> = self.entries.values().collect();
        sorted.sort_by(|a, b| {
            b.score(now)
                .partial_cmp(&a.score(now))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(n);
        sorted
    }

    /// Persist to disk if dirty.
    pub fn persist(&mut self) {
        if !self.dirty {
            return;
        }
        let path = store_path();
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string(&self.entries) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(()) => {
                    self.dirty = false;
                    tracing::debug!(entries = self.entries.len(), "Frecency store persisted");
                }
                Err(e) => tracing::warn!(error = %e, "Failed to persist frecency store"),
            },
            Err(e) => tracing::warn!(error = %e, "Failed to serialize frecency store"),
        }
    }
}

/// Path to the frecency JSON file.
fn store_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".yantrik").join("lens_frecency.json")
}

/// Current Unix timestamp as f64 seconds.
fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
