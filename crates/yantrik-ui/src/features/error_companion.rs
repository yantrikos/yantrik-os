//! Error Companion — watches shell command failures, surfaces Whisper Cards.
//!
//! Monitors `~/.yantrik/cmd_log` (written by a PROMPT_COMMAND bash hook).
//! On error (exit != 0): emits an Urge with the failed command.
//! Solution recall happens through the LLM conversation when the user
//! clicks "Help" and opens the Intent Lens.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Instant;

use yantrik_os::SystemEvent;

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};

pub struct ErrorCompanion {
    /// Path to the command log file.
    cmd_log_path: PathBuf,
    /// File offset — read only bytes after this position.
    last_offset: u64,
    /// Whether we've initialized the offset (skip existing entries at boot).
    initialized: bool,
    /// Cooldown: command text → last fire time.
    cooldowns: HashMap<String, Instant>,
    /// Seconds between duplicate urges for the same command.
    cooldown_secs: u64,
}

impl ErrorCompanion {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        Self {
            cmd_log_path: PathBuf::from(home).join(".yantrik/cmd_log"),
            last_offset: 0,
            initialized: false,
            cooldowns: HashMap::new(),
            cooldown_secs: 60,
        }
    }

    /// Check if cooldown has expired for a given command.
    fn should_fire(&mut self, cmd: &str) -> bool {
        let now = Instant::now();
        if let Some(last) = self.cooldowns.get(cmd) {
            if now.duration_since(*last).as_secs() < self.cooldown_secs {
                return false;
            }
        }
        self.cooldowns.insert(cmd.to_string(), now);
        true
    }

    /// Read new lines from cmd_log since last_offset.
    fn read_new_entries(&mut self) -> Vec<CmdLogEntry> {
        let mut file = match std::fs::File::open(&self.cmd_log_path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };

        // Get current file size
        let file_len = match file.metadata() {
            Ok(m) => m.len(),
            Err(_) => return Vec::new(),
        };

        // Nothing new
        if file_len <= self.last_offset {
            // Handle file truncation (log rotation)
            if file_len < self.last_offset {
                self.last_offset = 0;
            } else {
                return Vec::new();
            }
        }

        // Seek to last known position
        if file.seek(SeekFrom::Start(self.last_offset)).is_err() {
            return Vec::new();
        }

        // Read new bytes
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            return Vec::new();
        }

        self.last_offset = file_len;

        // Parse lines
        buf.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() >= 3 {
                    let timestamp = parts[0].parse::<u64>().ok()?;
                    let command = parts[1].to_string();
                    let exit_code = parts[2].parse::<i32>().ok()?;
                    Some(CmdLogEntry {
                        timestamp,
                        command,
                        exit_code,
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

struct CmdLogEntry {
    timestamp: u64,
    command: String,
    exit_code: i32,
}

impl ProactiveFeature for ErrorCompanion {
    fn name(&self) -> &str {
        "error_companion"
    }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        let path = match event {
            SystemEvent::FileChanged {
                path,
                kind: yantrik_os::FileChangeKind::Modified,
            } => path,
            _ => return Vec::new(),
        };

        // Only react to cmd_log changes
        if !path.ends_with("cmd_log") {
            return Vec::new();
        }

        let entries = self.read_new_entries();
        let mut urges = Vec::new();

        for entry in entries {
            // Skip successful commands (shouldn't be in the log, but defensive)
            if entry.exit_code == 0 {
                continue;
            }

            // Cooldown check
            if !self.should_fire(&entry.command) {
                continue;
            }

            // Truncate command for title
            let cmd_display = if entry.command.len() > 50 {
                format!("{}...", &entry.command[..47])
            } else {
                entry.command.clone()
            };

            tracing::info!(
                cmd = %entry.command,
                exit_code = entry.exit_code,
                "Shell command failed"
            );

            urges.push(Urge {
                id: format!("err-{}", entry.timestamp),
                source: "error_companion".to_string(),
                title: format!("Command failed: {}", cmd_display),
                body: format!(
                    "Exit code {}. Tap Help to troubleshoot.",
                    entry.exit_code
                ),
                urgency: 0.6,
                confidence: 0.9,
                category: UrgeCategory::Shell,
            });
        }

        urges
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        // Initialize offset on first tick — skip any existing entries
        if !self.initialized {
            self.initialized = true;
            if let Ok(meta) = std::fs::metadata(&self.cmd_log_path) {
                self.last_offset = meta.len();
                tracing::debug!(
                    offset = self.last_offset,
                    "ErrorCompanion initialized — skipping existing entries"
                );
            }
        }

        // Clean expired cooldowns (older than 5 minutes)
        let now = Instant::now();
        self.cooldowns
            .retain(|_, t| now.duration_since(*t).as_secs() < 300);

        Vec::new()
    }

    fn on_feedback(&mut self, _urge_id: &str, _outcome: Outcome) {
        // Future: track which errors the user acted on vs dismissed
    }
}
