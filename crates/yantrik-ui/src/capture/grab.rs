//! Taking the pixels off a display: the one place `grim` is run from.
//!
//! Two features look at the screen. `control_screen` reads it as text for a mind; the capture
//! bar and the `screenshot_*` actions save it as a picture. They used to be two copies of the
//! same grim call with different deadlines and different ideas of which Wayland display was
//! meant, and the copy without a deadline was the one that hung (Mind View, Sep 2026). Both ask
//! this module now: which display, which rectangle, bounded by the same limit.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long a capture may take. A grim on the nested display once hung for good (Mind View,
/// Sep 2026), and a read that never answers is worse than one that says it could not.
pub const CAPTURE_LIMIT: Duration = Duration::from_secs(10);

/// The largest region a caller may ask for, in pixels: a 4K display. grim allocates the whole
/// rectangle and the reader holds it several times over; a 16384² ask was a gigabyte before a
/// single character was read.
pub const MAX_REGION_PIXELS: i64 = 3840 * 2160;

/// The sentence that says a look at the person's desktop cannot be taken back. A macro so that
/// `read_screen` and `screenshot_screen` say it word for word: the gate's `unrecoverable` reads
/// it, and it is what makes `auto` ask and keeps an "allow for this session" from covering the
/// next look. Two hand-typed copies would drift, and the one that lost the words would run
/// unasked.
macro_rules! seen_for_good {
    () => {
        "It shows everything they have open, and a read cannot be undone — what was on screen \
         has been seen — so it asks them every time"
    };
}
pub(crate) use seen_for_good;

/// Which display to look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    /// The person's desktop: every window they have open.
    Desktop,
    /// Mind View, the nested display minds' own apps draw on.
    MindView,
}

impl Display {
    pub fn name(self) -> &'static str {
        match self {
            Display::Desktop => "desktop",
            Display::MindView => "mind_view",
        }
    }

    /// The Wayland display to capture.
    pub fn wayland(self) -> Result<String, String> {
        match self {
            Display::Desktop => std::env::var("WAYLAND_DISPLAY")
                .ok()
                .filter(|d| !d.is_empty())
                .ok_or_else(|| "the desktop has no Wayland display to read".to_string()),
            Display::MindView => crate::mind_view::display_now().ok_or_else(|| {
                "Mind View is not running, so there is nothing on it to read. A mind's app opens \
                 it: shell.open_app"
                    .to_string()
            }),
        }
    }
}

/// `x,y,w,h` in the display's pixels, as grim's `-g` wants it: `x,y wxh`.
pub fn region_arg(given: &str) -> Result<String, String> {
    let parts: Vec<i64> = given
        .split(',')
        .map(|p| p.trim().parse::<i64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("`region` is x,y,w,h in pixels, e.g. 0,0,640,400 — not `{given}`"))?;
    match parts.as_slice() {
        [x, y, w, h]
            if (0..=65535).contains(x)
                && (0..=65535).contains(y)
                && (8..=16384).contains(w)
                && (8..=16384).contains(h)
                && w * h <= MAX_REGION_PIXELS =>
        {
            Ok(format!("{x},{y} {w}x{h}"))
        }
        _ => Err(format!(
            "`region` is x,y,w,h in pixels with a width and height of at least 8, and no larger \
             than a 4K display — not `{given}`"
        )),
    }
}

/// Run a command to its end or its deadline, whichever comes first; stdout on success.
///
/// The output is drained while the command runs, not after: a pipe holds 64 KiB, and a child
/// that fills it waits for a reader forever — which, read only at exit, was every page of small
/// text, killed at the deadline.
pub fn run_bounded(mut cmd: Command, limit: Duration, what: &str) -> Result<Vec<u8>, String> {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {what}: {e}"))?;
    let pid = child.id() as libc::pid_t;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(out)) if out.status.success() => Ok(out.stdout),
        Ok(Ok(out)) => Err(format!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Ok(Err(e)) => Err(format!("{what}: {e}")),
        Err(_) => {
            // The waiting thread reaps it once it dies.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            Err(format!("{what} did not finish in {} s", limit.as_secs()))
        }
    }
}

/// How a capture is written: `ppm` for the text reader, `png` for a picture somebody keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Ppm,
    Png,
}

/// Capture `display` (or a rectangle of it, in grim's geometry) into `out`, bounded.
///
/// `pointer` draws the pointer into the picture (grim `-c`); off, it is left out, which is
/// grim's own default and what the text reader wants.
pub fn grab(display: Display, region: Option<&str>, format: Format, pointer: bool, out: &Path) -> Result<(), String> {
    run_bounded(grim(display, region, format, pointer, out.as_os_str())?, CAPTURE_LIMIT, "the capture (grim)").map(|_| ())
}

/// The same, the picture handed back in memory (grim writes to its stdout) rather than written
/// anywhere by grim: for a look that keeps nothing (the screen watcher's), and for a screenshot,
/// which is written into a file the shell made itself (`names::create_fresh`) and never into a
/// path grim would open and follow.
pub fn grab_bytes(display: Display, region: Option<&str>, format: Format, pointer: bool) -> Result<Vec<u8>, String> {
    run_bounded(grim(display, region, format, pointer, std::ffi::OsStr::new("-"))?, CAPTURE_LIMIT, "the capture (grim)")
}

fn grim(display: Display, region: Option<&str>, format: Format, pointer: bool, out: &std::ffi::OsStr) -> Result<Command, String> {
    let wayland = display.wayland()?;
    let mut grim = Command::new("grim");
    // WAYLAND_SOCKET would win over WAYLAND_DISPLAY, and name whatever socket the shell was
    // handed rather than the display asked for.
    grim.env("WAYLAND_DISPLAY", &wayland).env_remove("WAYLAND_SOCKET");
    grim.args(["-t", match format { Format::Ppm => "ppm", Format::Png => "png" }]);
    if pointer {
        grim.arg("-c");
    }
    if let Some(g) = region {
        grim.args(["-g", g]);
    }
    grim.arg(out);
    Ok(grim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_region_is_four_numbers_and_becomes_grims_geometry() {
        assert_eq!(region_arg("0,0,640,400").unwrap(), "0,0 640x400");
        assert_eq!(region_arg(" 10, 20 ,30,40").unwrap(), "10,20 30x40");
        assert_eq!(region_arg("0,0,3840,2160").unwrap(), "0,0 3840x2160");
        for bad in [
            "", "1,2,3", "a,b,c,d", "-1,0,100,100", "0,0,4,100", "0,0,100,99999", "0,0,640,400,1",
            "0,0,16384,16384", "99999999999,0,100,100", "0,70000,100,100",
        ] {
            assert!(region_arg(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_command_that_says_a_lot_is_not_stuck_behind_its_own_pipe() {
        // 1 MiB, sixteen times what a pipe holds: read only at exit, this never exited.
        let mut cmd = Command::new("head");
        cmd.args(["-c", "1048576", "/dev/zero"]);
        let out = run_bounded(cmd, Duration::from_secs(10), "head").unwrap();
        assert_eq!(out.len(), 1 << 20);
    }

    #[test]
    fn a_command_that_hangs_is_ended_at_its_deadline() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let started = Instant::now();
        let err = run_bounded(cmd, Duration::from_millis(300), "sleep").unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn grim_is_run_from_here_and_nowhere_else_in_the_shell() {
        // One shared module, never copies: a second grim call is a second deadline to forget.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        fn walk(dir: &std::path::Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, found);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&p).unwrap_or_default();
                    if text.contains("Command::new(\"grim\")") {
                        found.push(p.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        walk(&src, &mut found);
        assert_eq!(found.len(), 1, "grim is run from more than one file: {found:?}");
        assert!(found[0].ends_with("capture/grab.rs"), "{found:?}");
    }
}
