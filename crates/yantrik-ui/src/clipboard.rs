//! Clipboard History — monitors wl-paste, stores last 50 entries with timestamps.
//!
//! Polls `wl-paste` every second on a background thread.
//! Provides search and time-based retrieval for Intent Lens integration.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Max entries to retain.
const MAX_ENTRIES: usize = 50;

/// Max bytes per single clipboard entry (skip images/huge content).
const MAX_ENTRY_BYTES: usize = 10_000;

/// A single clipboard history entry.
#[derive(Debug, Clone)]
pub struct ClipEntry {
    /// Full clipboard text (up to MAX_ENTRY_BYTES).
    pub content: String,
    /// Unix epoch seconds when captured.
    pub timestamp: f64,
}

impl ClipEntry {
    /// Short preview: first line, max 80 chars.
    pub fn preview(&self) -> String {
        let first_line = self.content.lines().next().unwrap_or("");
        if first_line.len() > 80 {
            format!("{}...", &first_line[..77])
        } else {
            first_line.to_string()
        }
    }

    /// Human-readable "time ago" string.
    pub fn time_ago(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let delta = (now - self.timestamp).max(0.0);
        if delta < 60.0 {
            "just now".to_string()
        } else if delta < 3600.0 {
            format!("{:.0}m ago", delta / 60.0)
        } else if delta < 86400.0 {
            format!("{:.0}h ago", delta / 3600.0)
        } else {
            format!("{:.0}d ago", delta / 86400.0)
        }
    }
}

/// Thread-safe clipboard history handle.
pub type SharedHistory = Arc<Mutex<ClipHistory>>;

/// Ring buffer of clipboard entries.
pub struct ClipHistory {
    entries: VecDeque<ClipEntry>,
}

impl ClipHistory {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_ENTRIES),
        }
    }

    /// Add a new entry. Deduplicates against the most recent entry.
    pub fn push(&mut self, content: String) {
        if let Some(last) = self.entries.front() {
            if last.content == content {
                return;
            }
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        self.entries.push_front(ClipEntry { content, timestamp });

        while self.entries.len() > MAX_ENTRIES {
            self.entries.pop_back();
        }
    }

    /// Get the N most recent entries.
    pub fn recent(&self, n: usize) -> Vec<&ClipEntry> {
        self.entries.iter().take(n).collect()
    }

    /// Search entries by substring.
    #[allow(dead_code)]
    pub fn search(&self, query: &str) -> Vec<(usize, &ClipEntry)> {
        let lower = query.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.content.to_lowercase().contains(&lower))
            .take(6)
            .collect()
    }

    /// Get entry by index (0 = most recent).
    pub fn get(&self, index: usize) -> Option<&ClipEntry> {
        self.entries.get(index)
    }

    /// Total entries stored.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Start the clipboard watcher thread. Returns a shared history handle.
pub fn start_watcher() -> SharedHistory {
    let history: SharedHistory = Arc::new(Mutex::new(ClipHistory::new()));
    let history_thread = history.clone();

    std::thread::Builder::new()
        .name("yantrik-clipboard".into())
        .spawn(move || {
            run_watcher(history_thread);
        })
        .expect("failed to spawn clipboard watcher");

    history
}

/// Polls `wl-paste` every second and stores new clipboard content.
fn run_watcher(history: SharedHistory) {
    tracing::info!("Clipboard watcher started");
    let mut last_content = String::new();

    loop {
        match std::process::Command::new("wl-paste")
            .arg("--no-newline")
            .output()
        {
            Ok(output) if output.status.success() => {
                let content = String::from_utf8_lossy(&output.stdout).to_string();
                if !content.is_empty()
                    && content != last_content
                    && content.len() <= MAX_ENTRY_BYTES
                {
                    last_content = content.clone();
                    if let Ok(mut h) = history.lock() {
                        h.push(content);
                    }
                    tracing::debug!("Clipboard entry captured");
                }
            }
            Ok(_) => {} // wl-paste returned non-zero (empty clipboard)
            Err(e) => {
                // wl-paste not available — stop watching (non-Wayland environment)
                tracing::debug!(error = %e, "wl-paste unavailable — clipboard watcher stopping");
                return;
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
