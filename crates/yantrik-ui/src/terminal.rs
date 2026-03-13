//! Terminal backend — PTY spawn + vt100 screen parsing + ANSI color extraction.
//!
//! Spawns a shell via `pty-process`, reads output on a background thread,
//! and maintains a vt100 screen buffer with full color segment data.
//! The UI polls `get_segments()` for colored output and `get_scrollback_text()`
//! for scroll history.

use std::io::{Read, Write};
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A colored text segment for rendering in Slint.
#[derive(Clone, Debug)]
pub struct ColorSegment {
    pub text: String,
    pub fg_r: u8,
    pub fg_g: u8,
    pub fg_b: u8,
    pub bold: bool,
    pub row: u16,
    pub col_start: u16,
}

/// Handle to a running terminal session.
///
/// Cheaply cloneable (all fields are Arc). Drop-safe: the reader thread
/// exits when the PTY child dies or `alive` is set to false.
#[derive(Clone)]
pub struct TerminalHandle {
    /// Write end of the PTY master — send keystrokes here.
    pty_write: Arc<Mutex<std::fs::File>>,
    /// Raw PTY fd for resize ioctl.
    pty_raw_fd: Arc<AtomicI32>,
    /// Current screen as colored segments.
    segments: Arc<Mutex<Vec<ColorSegment>>>,
    /// Current screen rows (plain text fallback).
    rows: Arc<Mutex<Vec<String>>>,
    /// Scrollback text (lines above visible area).
    scrollback_text: Arc<Mutex<String>>,
    /// Number of scrollback lines available.
    scrollback_count: Arc<AtomicUsize>,
    /// Current cursor position.
    cursor_row: Arc<AtomicUsize>,
    cursor_col: Arc<AtomicUsize>,
    /// Whether the shell process is still alive.
    alive: Arc<AtomicBool>,
    /// Last output chunk (for AI error scanning).
    last_output: Arc<Mutex<String>>,
    /// Whether application cursor mode is active (for arrow key encoding).
    application_cursor: Arc<AtomicBool>,
    /// Dirty flag — set when screen content changes, cleared after UI reads.
    dirty: Arc<AtomicBool>,
    /// Current terminal dimensions (packed: high 16 = rows, low 16 = cols).
    dimensions: Arc<AtomicU32>,
    /// Resize channel — send (rows, cols) to reader thread.
    resize_tx: crossbeam_channel::Sender<(u16, u16)>,
    /// Detected working directory from prompt.
    cwd: Arc<Mutex<String>>,
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
                if std::path::Path::new("/bin/ash").exists() {
                    "/bin/ash".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });
        let mut cmd = pty_process::blocking::Command::new(&shell);
        let _child = cmd.spawn(pts)?;

        // Store raw fd for resize before consuming pty
        let raw_fd = pty.as_raw_fd();

        // Clone fd for reader thread
        let read_fd = pty.as_fd().try_clone_to_owned()
            .map_err(|e| anyhow::anyhow!("Failed to dup PTY fd for reader: {}", e))?;
        let read_file = std::fs::File::from(read_fd);

        // Convert original PTY to a File for writing
        let write_fd: std::os::fd::OwnedFd = pty.into();
        let write_file = std::fs::File::from(write_fd);

        let (resize_tx, resize_rx) = crossbeam_channel::unbounded();

        let pty_write = Arc::new(Mutex::new(write_file));
        let screen_rows = Arc::new(Mutex::new(vec![String::new(); rows as usize]));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let scrollback_text = Arc::new(Mutex::new(String::new()));
        let scrollback_count = Arc::new(AtomicUsize::new(0));
        let cursor_row = Arc::new(AtomicUsize::new(0));
        let cursor_col = Arc::new(AtomicUsize::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let last_output = Arc::new(Mutex::new(String::new()));
        let application_cursor = Arc::new(AtomicBool::new(false));
        let dirty = Arc::new(AtomicBool::new(true));
        let dimensions = Arc::new(AtomicU32::new(((rows as u32) << 16) | (cols as u32)));
        let cwd = Arc::new(Mutex::new(String::new()));

        let handle = Self {
            pty_write,
            pty_raw_fd: Arc::new(AtomicI32::new(raw_fd)),
            segments,
            rows: screen_rows,
            scrollback_text,
            scrollback_count,
            cursor_row,
            cursor_col,
            alive,
            last_output,
            application_cursor,
            dirty,
            dimensions,
            resize_tx,
            cwd,
        };

        // Start reader thread
        let h = handle.clone();
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                reader_loop(read_file, rows, cols, h, resize_rx);
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

    /// Resize the PTY and terminal parser.
    pub fn resize(&self, rows: u16, cols: u16) {
        let fd = self.pty_raw_fd.load(Ordering::Relaxed);
        if fd >= 0 {
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            unsafe {
                libc::ioctl(fd, libc::TIOCSWINSZ, &ws);
            }
        }
        // Tell reader thread to resize the parser
        let _ = self.resize_tx.send((rows, cols));
        self.dimensions.store(((rows as u32) << 16) | (cols as u32), Ordering::Relaxed);
    }

    /// Get current dimensions (rows, cols).
    pub fn dimensions(&self) -> (u16, u16) {
        let d = self.dimensions.load(Ordering::Relaxed);
        ((d >> 16) as u16, (d & 0xFFFF) as u16)
    }

    /// Get current screen rows as a Vec (plain text fallback).
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

    /// Get colored segments for the current screen.
    pub fn get_segments(&self) -> Vec<ColorSegment> {
        self.segments.lock().unwrap().clone()
    }

    /// Get scrollback text (lines above the visible screen).
    pub fn get_scrollback_text(&self) -> String {
        self.scrollback_text.lock().unwrap().clone()
    }

    /// Get number of scrollback lines.
    pub fn scrollback_count(&self) -> usize {
        self.scrollback_count.load(Ordering::Relaxed)
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

    /// Check if screen content changed since last clear.
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    /// Clear the dirty flag after UI has consumed the update.
    pub fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    /// Get detected working directory.
    pub fn get_cwd(&self) -> String {
        self.cwd.lock().unwrap().clone()
    }
}

// ─── Xterm 256-color palette ─────────────────────────────────────────────

/// Standard 16-color ANSI palette (dark theme friendly).
const ANSI_COLORS: [(u8, u8, u8); 16] = [
    (0x1c, 0x1c, 0x28),  // 0: Black (dark bg)
    (0xef, 0x9a, 0x9a),  // 1: Red
    (0x26, 0xa6, 0x9a),  // 2: Green
    (0xff, 0xb7, 0x4d),  // 3: Yellow
    (0x4e, 0xcd, 0xc4),  // 4: Blue (accent)
    (0xb0, 0x84, 0xe0),  // 5: Magenta
    (0x4e, 0xcd, 0xc4),  // 6: Cyan
    (0xe8, 0xea, 0xef),  // 7: White
    (0x4a, 0x50, 0x60),  // 8: Bright black (dim)
    (0xef, 0x9a, 0x9a),  // 9: Bright red
    (0x26, 0xa6, 0x9a),  // 10: Bright green
    (0xff, 0xb7, 0x4d),  // 11: Bright yellow
    (0x7e, 0xe0, 0xd8),  // 12: Bright blue
    (0xcc, 0xa0, 0xf0),  // 13: Bright magenta
    (0x7e, 0xe0, 0xd8),  // 14: Bright cyan
    (0xe8, 0xea, 0xef),  // 15: Bright white
];

/// Convert a vt100 color to RGB.
fn vt100_color_to_rgb(color: vt100::Color) -> (u8, u8, u8) {
    match color {
        vt100::Color::Default => (0xe8, 0xea, 0xef), // Theme.text-primary
        vt100::Color::Idx(n) => {
            if n < 16 {
                ANSI_COLORS[n as usize]
            } else if n < 232 {
                // 6x6x6 color cube (indices 16-231)
                let n = n - 16;
                let r = (n / 36) % 6;
                let g = (n / 6) % 6;
                let b = n % 6;
                let to_val = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                (to_val(r), to_val(g), to_val(b))
            } else {
                // Grayscale ramp (indices 232-255)
                let v = 8 + (n - 232) * 10;
                (v, v, v)
            }
        }
        vt100::Color::Rgb(r, g, b) => (r, g, b),
    }
}

/// Extract colored segments from the vt100 screen.
fn extract_segments(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<ColorSegment> {
    let mut segments = Vec::with_capacity(rows as usize * 5); // ~5 segments per row avg

    for row in 0..rows {
        let mut col = 0u16;
        while col < cols {
            let cell = screen.cell(row, col);
            if cell.is_none() {
                col += 1;
                continue;
            }
            let cell = cell.unwrap();
            let ch = cell.contents();
            if ch.is_empty() {
                col += 1;
                continue;
            }

            let fg = vt100_color_to_rgb(cell.fgcolor());
            let bold = cell.bold();
            let seg_start = col;
            let mut text = ch.to_string();

            // Group consecutive cells with same attributes
            col += 1;
            while col < cols {
                let next = screen.cell(row, col);
                if next.is_none() {
                    break;
                }
                let next = next.unwrap();
                let next_ch = next.contents();
                if next_ch.is_empty() {
                    break;
                }
                let next_fg = vt100_color_to_rgb(next.fgcolor());
                let next_bold = next.bold();
                if next_fg != fg || next_bold != bold {
                    break;
                }
                text.push_str(next_ch);
                col += 1;
            }

            // Trim trailing spaces from segment (but not if mid-row with more segments)
            let trimmed = if col >= cols {
                text.trim_end().to_string()
            } else {
                text
            };

            if !trimmed.is_empty() {
                segments.push(ColorSegment {
                    text: trimmed,
                    fg_r: fg.0,
                    fg_g: fg.1,
                    fg_b: fg.2,
                    bold,
                    row,
                    col_start: seg_start,
                });
            }
        }
    }

    segments
}

/// Detect working directory from the last prompt line.
fn detect_cwd(rows: &[String]) -> Option<String> {
    // Look at the last few non-empty lines for prompt patterns
    for row in rows.iter().rev().take(3) {
        let trimmed = row.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Pattern: user@host:path$ or user@host:path#
        if let Some(colon_pos) = trimmed.find(':') {
            if let Some(at_pos) = trimmed[..colon_pos].find('@') {
                if at_pos > 0 {
                    let after_colon = &trimmed[colon_pos + 1..];
                    // Find the prompt char ($ or #)
                    if let Some(prompt_pos) = after_colon.rfind(|c| c == '$' || c == '#') {
                        let path = after_colon[..prompt_pos].trim();
                        if !path.is_empty() {
                            return Some(path.to_string());
                        }
                    }
                }
            }
        }

        // Pattern: path $ or path # (simple prompt)
        if trimmed.ends_with("$ ") || trimmed.ends_with("# ") || trimmed.ends_with('$') || trimmed.ends_with('#') {
            let path = trimmed.trim_end_matches(|c| c == '$' || c == '#' || c == ' ');
            if path.starts_with('/') || path.starts_with('~') {
                return Some(path.to_string());
            }
        }
    }
    None
}

/// Background reader loop: reads from PTY, feeds vt100 parser, updates shared state.
fn reader_loop(
    mut read_file: std::fs::File,
    rows: u16,
    cols: u16,
    handle: TerminalHandle,
    resize_rx: crossbeam_channel::Receiver<(u16, u16)>,
) {
    let mut parser = vt100::Parser::new(rows, cols, 1000);
    let mut buf = [0u8; 4096];
    let mut current_rows = rows;
    let mut current_cols = cols;

    loop {
        // Check for resize requests (non-blocking)
        while let Ok((new_rows, new_cols)) = resize_rx.try_recv() {
            if new_rows != current_rows || new_cols != current_cols {
                parser.screen_mut().set_size(new_rows, new_cols);
                current_rows = new_rows;
                current_cols = new_cols;
            }
        }

        match read_file.read(&mut buf) {
            Ok(0) => {
                handle.alive.store(false, Ordering::Relaxed);
                tracing::info!("PTY reader: shell exited (EOF)");
                break;
            }
            Ok(n) => {
                parser.process(&buf[..n]);

                let screen = parser.screen();

                // Extract colored segments
                let new_segments = extract_segments(screen, current_rows, current_cols);

                // Extract plain text rows (for search, AI, etc.)
                let new_rows: Vec<String> = screen
                    .rows(0, current_cols)
                    .map(|row| row.trim_end().to_string())
                    .collect();

                // Extract scrollback
                let scrollback_len = screen.scrollback();
                if scrollback_len > 0 {
                    let mut sb_text = String::new();
                    // Access scrollback rows by setting scrollback offset
                    // vt100 provides scrollback via contents_between or by
                    // iterating rows with negative offsets
                    // For simplicity, store a rolling scrollback buffer
                    let sb_count = scrollback_len;
                    handle.scrollback_count.store(sb_count, Ordering::Relaxed);

                    // Build scrollback text from stored buffer
                    // (vt100's scrollback is accessed via screen.rows_formatted
                    //  with scrollback set, but this is complex. For now, track
                    //  scrollback count and use the existing last_output for context)
                }

                // Update cursor
                let (crow, ccol) = screen.cursor_position();
                handle.cursor_row.store(crow as usize, Ordering::Relaxed);
                handle.cursor_col.store(ccol as usize, Ordering::Relaxed);

                // Update application cursor mode
                handle.application_cursor.store(screen.application_cursor(), Ordering::Relaxed);

                // Store last output chunk for AI error detection
                {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    let mut last = handle.last_output.lock().unwrap();
                    if last.len() > 2048 {
                        let start = last.len() - 1024;
                        *last = last[start..].to_string();
                    }
                    last.push_str(&chunk);
                }

                // Detect CWD from prompt
                if let Some(cwd) = detect_cwd(&new_rows) {
                    *handle.cwd.lock().unwrap() = cwd;
                }

                // Update shared state
                *handle.segments.lock().unwrap() = new_segments;
                *handle.rows.lock().unwrap() = new_rows;

                // Mark dirty
                handle.dirty.store(true, Ordering::Relaxed);
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
pub fn key_to_pty_bytes(key_text: &str, _modifiers_shift: bool, modifiers_control: bool, application_cursor: bool) -> Option<Vec<u8>> {
    match key_text {
        "\r" | "\n" => Some(b"\r".to_vec()),
        "\u{0008}" => Some(vec![0x7f]),
        "\t" => Some(b"\t".to_vec()),
        "\u{001b}" => Some(vec![0x1b]),

        // Arrow keys
        "\u{F700}" => if application_cursor { Some(b"\x1bOA".to_vec()) } else { Some(b"\x1b[A".to_vec()) },
        "\u{F701}" => if application_cursor { Some(b"\x1bOB".to_vec()) } else { Some(b"\x1b[B".to_vec()) },
        "\u{F703}" => if application_cursor { Some(b"\x1bOC".to_vec()) } else { Some(b"\x1b[C".to_vec()) },
        "\u{F702}" => if application_cursor { Some(b"\x1bOD".to_vec()) } else { Some(b"\x1b[D".to_vec()) },

        // Navigation
        "\u{F729}" => Some(b"\x1b[H".to_vec()),
        "\u{F72B}" => Some(b"\x1b[F".to_vec()),
        "\u{F72C}" => Some(b"\x1b[5~".to_vec()),
        "\u{F72D}" => Some(b"\x1b[6~".to_vec()),
        "\u{F728}" | "\u{007f}" => Some(b"\x1b[3~".to_vec()),
        "\u{F727}" => Some(b"\x1b[2~".to_vec()),

        // Function keys
        "\u{F704}" => Some(b"\x1bOP".to_vec()),
        "\u{F705}" => Some(b"\x1bOQ".to_vec()),
        "\u{F706}" => Some(b"\x1bOR".to_vec()),
        "\u{F707}" => Some(b"\x1bOS".to_vec()),
        "\u{F708}" => Some(b"\x1b[15~".to_vec()),
        "\u{F709}" => Some(b"\x1b[17~".to_vec()),
        "\u{F70A}" => Some(b"\x1b[18~".to_vec()),
        "\u{F70B}" => Some(b"\x1b[19~".to_vec()),
        "\u{F70C}" => Some(b"\x1b[20~".to_vec()),
        "\u{F70D}" => Some(b"\x1b[21~".to_vec()),
        "\u{F70E}" => Some(b"\x1b[23~".to_vec()),
        "\u{F70F}" => Some(b"\x1b[24~".to_vec()),

        text => {
            if modifiers_control && text.len() == 1 {
                let ch = text.chars().next().unwrap();
                if ch.is_ascii_alphabetic() {
                    let code = (ch.to_ascii_lowercase() as u8) - b'a' + 1;
                    return Some(vec![code]);
                }
            }
            if !text.is_empty() {
                let first = text.chars().next().unwrap();
                if (first as u32) >= 0xF700 && (first as u32) <= 0xF8FF {
                    return None;
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
            // skip
        } else {
            out.push(c);
        }
    }
    out
}

/// Check if terminal output contains error patterns.
pub fn detect_errors(output: &str) -> Option<String> {
    let error_patterns = [
        "error:", "Error:", "ERROR:", "FATAL:", "fatal:",
        "panic:", "PANIC:",
        "Traceback (most recent call last)",
        "command not found", ": not found",
        "No such file or directory", "Permission denied",
        "segmentation fault", "Segmentation fault", "core dumped",
        "Cannot allocate memory", "Connection refused", "Connection timed out",
        "syntax error", "unrecognized option", "invalid option",
    ];

    let cleaned = strip_ansi(output);
    let lines: Vec<&str> = cleaned.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        for pattern in &error_patterns {
            if line.contains(pattern) {
                let start = i.saturating_sub(3);
                let end = (i + 5).min(lines.len());
                let context: Vec<&str> = lines[start..end].to_vec();
                return Some(context.join("\n"));
            }
        }
    }

    None
}

/// List of dangerous command patterns for safety warnings.
pub const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf *",
    "dd if=",
    "mkfs.",
    "chmod -R 777 /",
    "> /dev/sd",
    ":(){ :|:&};:",
    "wget*|*sh",
    "curl*|*sh",
    "curl*|*bash",
];

/// Check if a command line contains dangerous patterns.
pub fn is_dangerous_command(cmd: &str) -> Option<&'static str> {
    let cmd_lower = cmd.to_lowercase().trim().to_string();

    if cmd_lower.starts_with("rm -rf /") && !cmd_lower.starts_with("rm -rf /tmp") {
        return Some("This will recursively delete from root. Are you sure?");
    }
    if cmd_lower.starts_with("rm -rf ~") || cmd_lower.starts_with("rm -rf $home") {
        return Some("This will delete your entire home directory.");
    }
    if cmd_lower.starts_with("dd if=") && cmd_lower.contains("/dev/") {
        return Some("dd to a device can destroy data. Double-check the target.");
    }
    if cmd_lower.starts_with("mkfs.") {
        return Some("This will format a filesystem, erasing all data.");
    }
    if cmd_lower.contains("chmod -r 777 /") && !cmd_lower.contains("/tmp") {
        return Some("Setting 777 permissions recursively on system dirs is dangerous.");
    }
    if cmd_lower.contains("> /dev/sd") || cmd_lower.contains("> /dev/nvm") {
        return Some("Writing directly to a block device will destroy the filesystem.");
    }
    if cmd_lower.contains(":(){ :|:&};:") {
        return Some("This is a fork bomb that will crash your system.");
    }
    if (cmd_lower.contains("curl ") || cmd_lower.contains("wget "))
        && (cmd_lower.contains("| sh") || cmd_lower.contains("| bash") || cmd_lower.contains("|sh") || cmd_lower.contains("|bash"))
    {
        return Some("Piping remote content to a shell is risky. Review the script first.");
    }
    None
}
