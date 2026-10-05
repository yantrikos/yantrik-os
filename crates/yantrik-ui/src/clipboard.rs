//! Clipboard History — monitors wl-paste, stores last 50 entries with timestamps.
//!
//! Polls `wl-paste` every second on a background thread.
//! Provides search and time-based retrieval for Intent Lens integration.
//!
//! The history is a ring in this process's memory and nowhere else: it is never written to disk,
//! synced or sent, and it is gone when the shell restarts. The minds' clipboard tools read only
//! the current clipboard (`wl-paste`), not this. That is what lets the panel's footer say "Kept on
//! this machine"; a change that stores or sends it must change those words too.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Pause, or resume, history while a key is on its way from a provider's page to the vault (the
/// free AI card): the person copies it in the browser, and it must not be kept here before, or
/// after, they press Paste. The flag is the companion tools' own, so the model's clipboard tools
/// are held by the same switch (`provider_keys::hold_clipboard`).
pub fn hold_for_a_key(on: bool) {
    yantrik_companion::tools::provider_keys::hold_clipboard(on);
}

/// What history never keeps, held or not: anything shaped like a provider's API key. The same
/// shapes the setup card accepts, so what one takes the other never records.
fn never_kept(content: &str) -> bool {
    yantrik_ml::provider::pool::signup::looks_like_a_provider_key(content)
}

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

    /// Add a new entry. Deduplicates against the most recent entry. A provider's key is never
    /// added (`never_kept`).
    pub fn push(&mut self, content: String) {
        if never_kept(&content) {
            return;
        }
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

    /// Drop the entries that are exactly `content`: a key taken into the vault leaves no copy
    /// here. Exact, so a paste of something short cannot empty the history.
    pub fn forget(&mut self, content: &str) {
        let content = content.trim();
        if !content.is_empty() {
            self.entries.retain(|e| e.content.trim() != content);
        }
    }

    /// Empty the history (the panel's "Clear history"). What is on the clipboard now stays there:
    /// the watcher has already seen it, so it does not come straight back as a new entry.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get the N most recent entries.
    pub fn recent(&self, n: usize) -> Vec<&ClipEntry> {
        self.entries.iter().take(n).collect()
    }

    /// Search entries by substring (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<(usize, &ClipEntry)> {
        let lower = query.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.content.to_lowercase().contains(&lower))
            .take(20)
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
    // Private mode records nothing: the clipboard is not read while it is on, and what was copied
    // during it is taken as already seen when it ends, so it is never captured afterwards.
    let mut was_private = false;

    loop {
        let private = crate::private_mode::is_on() || yantrik_companion::tools::provider_keys::clipboard_held();
        if private {
            was_private = true;
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        let just_left_private = std::mem::take(&mut was_private);
        match std::process::Command::new("wl-paste")
            .arg("--no-newline")
            .output()
        {
            Ok(output) if output.status.success() => {
                let content = String::from_utf8_lossy(&output.stdout).to_string();
                if just_left_private {
                    last_content = content;
                } else if !content.is_empty()
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

#[cfg(test)]
mod key_tests {
    use super::*;

    /// A provider's key copied from its page is never kept in history, and one taken into the
    /// vault is dropped from it (free AI setup, 1 Oct 2026).
    #[test]
    fn a_providers_key_is_never_kept_in_history() {
        let mut h = ClipHistory::new();
        h.push(format!("gsk_{}", "Ab12".repeat(13)));
        h.push(format!("sk-or-v1-{}", "cd34".repeat(16)));
        assert_eq!(h.len(), 0, "a key went into history");
        h.push("an ordinary line".to_string());
        h.push("mistralsecretabcdef0123456789xyz".to_string());
        h.forget(" mistralsecretabcdef0123456789xyz\n");
        assert_eq!(h.len(), 1, "a key taken into the vault is dropped from history");
        assert_eq!(h.get(0).unwrap().content, "an ordinary line");
        h.forget("a");
        assert_eq!(h.len(), 1, "forgetting is exact: a short paste does not empty the history");
    }

    #[test]
    fn clearing_empties_the_history_and_it_fills_again_from_new_copies() {
        let mut h = ClipHistory::new();
        h.push("one".to_string());
        h.push("two".to_string());
        h.clear();
        assert_eq!(h.len(), 0);
        assert!(h.recent(20).is_empty() && h.search("o").is_empty());
        h.push("three".to_string());
        assert_eq!(h.get(0).unwrap().content, "three");
    }

    /// The panel says "History stored on this device". That stays true only while nothing here writes the
    /// history out or hands it to anything that could.
    #[test]
    fn the_history_is_kept_in_memory_only() {
        let src = include_str!("clipboard.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for out in ["std::fs", "File::", "write(", "reqwest", "TcpStream", "serde_json"] {
            assert!(!code.contains(out), "clipboard.rs uses `{out}`: the footer's \"History stored on this device\" needs a look");
        }
    }
}
