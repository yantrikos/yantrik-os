//! Terminal backend — PTY spawn + vt100 screen parsing.
//!
//! Spawns a shell via `pty-process`, reads output on a background thread,
//! and maintains a vt100 screen buffer. The UI polls `get_screen_text()`
//! to render the current terminal state.

use std::io::{Read, Write};
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Handle to a running terminal session.
///
/// Cheaply cloneable (all fields are Arc). Drop-safe: the reader thread
/// exits when the PTY child dies or `alive` is set to false.
#[derive(Clone)]
pub struct TerminalHandle {
    /// Write end of the PTY master — send keystrokes here.
    pty_write: Arc<Mutex<std::fs::File>>,
    /// Current screen rows (plain text, one String per row).
    rows: Arc<Mutex<Vec<String>>>,
    /// Current cursor position.
    cursor_row: Arc<AtomicUsize>,
    cursor_col: Arc<AtomicUsize>,
    /// Whether the shell process is still alive.
    alive: Arc<AtomicBool>,
    /// Last output chunk (for AI error scanning).
    last_output: Arc<Mutex<String>>,
    /// Whether application cursor mode is active (for arrow key encoding).
    application_cursor: Arc<AtomicBool>,
}

impl TerminalHandle {
    /// Spawn a new terminal session with the given dimensions.
    ///
    /// Returns the handle immediately; the reader thread runs in the background.
    pub fn spawn(rows: u16, cols: u16) -> anyhow::Result<Self> {
        use pty_process::Size;

        let (pty, pts) = pty_process::blocking::open()?;
        pty.resize(Size::new(rows, cols))?;

        // Spawn shell (prefer ash on Alpine for better interactive experience)
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty() && !s.contains(".exe") && std::path::Path::new(s).exists())
            .unwrap_or_else(|| {
                // Prefer ash (busybox interactive shell) over plain sh
                if std::path::Path::new("/bin/ash").exists() {
                    "/bin/ash".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });
        let mut cmd = pty_process::blocking::Command::new(&shell);
        let _child = cmd.spawn(pts)?;

        // Duplicate the PTY fd: one for reading (background thread), one for writing (main thread).
        // The PTY fd supports concurrent read/write on separate fds.
        let pty_raw_fd = pty.as_raw_fd();

        // Clone fd for reader thread
        let read_fd = pty.as_fd().try_clone_to_owned()
            .map_err(|e| anyhow::anyhow!("Failed to dup PTY fd for reader: {}", e))?;
        let read_file = std::fs::File::from(read_fd);

        // Convert original PTY to a File for writing (avoids ownership issues with Pty struct)
        let write_fd: std::os::fd::OwnedFd = pty.into();
        let write_file = std::fs::File::from(write_fd);

        let pty_write = Arc::new(Mutex::new(write_file));
        let screen_rows = Arc::new(Mutex::new(vec![String::new(); rows as usize]));
        let cursor_row = Arc::new(AtomicUsize::new(0));
        let cursor_col = Arc::new(AtomicUsize::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let last_output = Arc::new(Mutex::new(String::new()));
        let application_cursor = Arc::new(AtomicBool::new(false));

        let handle = Self {
            pty_write,
            rows: screen_rows,
            cursor_row,
            cursor_col,
            alive,
            last_output,
            application_cursor,
        };

        // Start reader thread
        let h = handle.clone();
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                reader_loop(read_file, rows, cols, h);
            })?;

        Ok(handle)
    }

    /// Send raw bytes to the PTY (keystrokes, escape sequences).
    pub fn write_bytes(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.pty_write.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    /// Get current screen rows as a Vec.
    pub fn get_rows(&self) -> Vec<String> {
        self.rows.lock().unwrap().clone()
    }

    /// Get current screen as a single string for display.
    pub fn get_full_text(&self) -> String {
        let rows = self.rows.lock().unwrap();
        let mut text = String::new();
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            text.push_str(row);
        }
        text
    }

    /// Get cursor position (row, col) within the visible screen.
    pub fn cursor_position(&self) -> (usize, usize) {
        (
            self.cursor_row.load(Ordering::Relaxed),
            self.cursor_col.load(Ordering::Relaxed),
        )
    }

    /// Check if the shell process is still alive.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Get the last output chunk (for AI error scanning).
    pub fn take_last_output(&self) -> String {
        let mut last = self.last_output.lock().unwrap();
        std::mem::take(&mut *last)
    }

    /// Check if application cursor mode is active.
    pub fn application_cursor_mode(&self) -> bool {
        self.application_cursor.load(Ordering::Relaxed)
    }
}

/// Background reader loop: reads from PTY, feeds vt100 parser, updates shared state.
fn reader_loop(mut read_file: std::fs::File, rows: u16, cols: u16, handle: TerminalHandle) {
    let mut parser = vt100::Parser::new(rows, cols, 1000); // 1000 lines scrollback
    let mut buf = [0u8; 4096];

    loop {
        match read_file.read(&mut buf) {
            Ok(0) => {
                // EOF — shell exited
                handle.alive.store(false, Ordering::Relaxed);
                tracing::info!("PTY reader: shell exited (EOF)");
                break;
            }
            Ok(n) => {
                parser.process(&buf[..n]);

                // Extract screen state using vt100's rows() iterator
                let screen = parser.screen();
                let new_rows: Vec<String> = screen
                    .rows(0, cols)
                    .map(|row| row.trim_end().to_string())
                    .collect();

                // Update cursor
                let (crow, ccol) = screen.cursor_position();
                handle.cursor_row.store(crow as usize, Ordering::Relaxed);
                handle.cursor_col.store(ccol as usize, Ordering::Relaxed);

                // Update application cursor mode
                handle
                    .application_cursor
                    .store(screen.application_cursor(), Ordering::Relaxed);

                // Store last output chunk for AI error detection
                {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    let mut last = handle.last_output.lock().unwrap();
                    // Keep last ~2KB for context
                    if last.len() > 2048 {
                        let start = last.len() - 1024;
                        *last = last[start..].to_string();
                    }
                    last.push_str(&chunk);
                }

                // Update shared rows
                *handle.rows.lock().unwrap() = new_rows;
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::Interrupted {
                    handle.alive.store(false, Ordering::Relaxed);
                    tracing::warn!(error = %e, "PTY reader error");
                    break;
                }
            }
        }
    }
}

/// Translate a Slint key event into PTY bytes.
///
/// Returns `Some(bytes)` if the key should be sent to the PTY,
/// or `None` if it should be ignored.
pub fn key_to_pty_bytes(key_text: &str, _modifiers_shift: bool, modifiers_control: bool, application_cursor: bool) -> Option<Vec<u8>> {
    // Slint special key constants (from slint::platform::Key)
    // These are Unicode private-use-area characters that Slint uses for special keys.
    match key_text {
        // Enter / Return
        "\r" | "\n" => Some(b"\r".to_vec()),

        // Backspace
        "\u{0008}" => Some(vec![0x7f]),

        // Tab
        "\t" => Some(b"\t".to_vec()),

        // Escape
        "\u{001b}" => Some(vec![0x1b]),

        // Arrow keys (Slint uses Unicode private-use chars)
        "\u{F700}" => { // Up
            if application_cursor { Some(b"\x1bOA".to_vec()) }
            else { Some(b"\x1b[A".to_vec()) }
        }
        "\u{F701}" => { // Down
            if application_cursor { Some(b"\x1bOB".to_vec()) }
            else { Some(b"\x1b[B".to_vec()) }
        }
        "\u{F703}" => { // Right
            if application_cursor { Some(b"\x1bOC".to_vec()) }
            else { Some(b"\x1b[C".to_vec()) }
        }
        "\u{F702}" => { // Left
            if application_cursor { Some(b"\x1bOD".to_vec()) }
            else { Some(b"\x1b[D".to_vec()) }
        }

        // Home / End
        "\u{F729}" => Some(b"\x1b[H".to_vec()),  // Home
        "\u{F72B}" => Some(b"\x1b[F".to_vec()),  // End

        // Page Up / Page Down
        "\u{F72C}" => Some(b"\x1b[5~".to_vec()), // Page Up
        "\u{F72D}" => Some(b"\x1b[6~".to_vec()), // Page Down

        // Delete
        "\u{F728}" | "\u{007f}" => Some(b"\x1b[3~".to_vec()),

        // Insert
        "\u{F727}" => Some(b"\x1b[2~".to_vec()),

        // Function keys (F1-F12)
        "\u{F704}" => Some(b"\x1bOP".to_vec()),   // F1
        "\u{F705}" => Some(b"\x1bOQ".to_vec()),   // F2
        "\u{F706}" => Some(b"\x1bOR".to_vec()),   // F3
        "\u{F707}" => Some(b"\x1bOS".to_vec()),   // F4
        "\u{F708}" => Some(b"\x1b[15~".to_vec()), // F5
        "\u{F709}" => Some(b"\x1b[17~".to_vec()), // F6
        "\u{F70A}" => Some(b"\x1b[18~".to_vec()), // F7
        "\u{F70B}" => Some(b"\x1b[19~".to_vec()), // F8
        "\u{F70C}" => Some(b"\x1b[20~".to_vec()), // F9
        "\u{F70D}" => Some(b"\x1b[21~".to_vec()), // F10
        "\u{F70E}" => Some(b"\x1b[23~".to_vec()), // F11
        "\u{F70F}" => Some(b"\x1b[24~".to_vec()), // F12

        // Regular text
        text => {
            if modifiers_control && text.len() == 1 {
                // Ctrl+letter -> control code
                let ch = text.chars().next().unwrap();
                if ch.is_ascii_alphabetic() {
                    let code = (ch.to_ascii_lowercase() as u8) - b'a' + 1;
                    return Some(vec![code]);
                }
            }

            // Regular text — send as UTF-8
            if !text.is_empty() {
                // Filter out any remaining Slint special keys we don't handle
                let first = text.chars().next().unwrap();
                if (first as u32) >= 0xF700 && (first as u32) <= 0xF8FF {
                    return None; // Unknown special key
                }
                Some(text.as_bytes().to_vec())
            } else {
                None
            }
        }
    }
}

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC[ ... m and ESC[ ... other sequences
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc.is_ascii_alphabetic() || nc == '~' {
                        break;
                    }
                }
            }
        } else if c.is_control() && c != '\n' && c != '\r' && c != '\t' {
            // Skip other control characters
        } else {
            out.push(c);
        }
    }
    out
}

/// Check if terminal output contains error patterns.
/// Returns the error context if found.
pub fn detect_errors(output: &str) -> Option<String> {
    let error_patterns = [
        "error:",
        "Error:",
        "ERROR:",
        "FATAL:",
        "fatal:",
        "panic:",
        "PANIC:",
        "Traceback (most recent call last)",
        "command not found",
        ": not found",
        "No such file or directory",
        "Permission denied",
        "segmentation fault",
        "Segmentation fault",
        "core dumped",
        "Cannot allocate memory",
        "Connection refused",
        "Connection timed out",
        "syntax error",
        "unrecognized option",
        "invalid option",
    ];

    // Strip ANSI escape codes so patterns match cleanly
    let cleaned = strip_ansi(output);
    let lines: Vec<&str> = cleaned.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        for pattern in &error_patterns {
            if line.contains(pattern) {
                // Extract context: up to 3 lines before and 5 after the error
                let start = i.saturating_sub(3);
                let end = (i + 5).min(lines.len());
                let context: Vec<&str> = lines[start..end].to_vec();
                return Some(context.join("\n"));
            }
        }
    }

    None
}
