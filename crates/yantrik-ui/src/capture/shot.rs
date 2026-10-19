//! A screenshot, from the moment the frame is clean to the file and the clipboard.
//!
//! The bar and the toasts are never in the picture. Whoever asks hides them first
//! ([`super::hide_furniture`]), and the capture waits [`SETTLE`] for the compositor to draw the
//! frame after they are gone: a Wayland client cannot ask when its unmap reached the screen, so
//! the wait is a number of frames, not a signal. The same wait follows `slurp`, whose selection
//! overlay is a surface of its own.
//!
//! grim writes the picture to its stdout, and the shell writes it into a file it made itself,
//! exclusively and 0600 in a 0700 folder (`names::create_fresh`): grim given a path opens it with
//! O_TRUNC and follows a link planted at the predictable next name (security review of #828,
//! finding 3). The clipboard gets the same bytes on wl-copy's stdin, with no file in between.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use chrono::Local;

use super::grab::{self, Display, Format};
use super::names::{self, Kind};
use super::Who;

/// Six frames at 60 Hz, three at 30: the bar has no fade, so its last frame is gone well inside
/// this.
pub const SETTLE: Duration = Duration::from_millis(100);

/// How long the person may take to drag a rectangle before the selection is given up.
const SELECT_LIMIT: Duration = Duration::from_secs(300);

/// What part of the display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Area {
    /// The whole display.
    Screen,
    /// The person drags a rectangle with `slurp`.
    Drag,
    /// A rectangle already known, in grim's geometry (a mind's `region`, checked).
    Given(String),
}

/// Where the picture goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Saved to ~/Pictures/Screenshots and copied to the clipboard: the person's screenshot.
    FileAndClipboard,
    /// The clipboard only (Ctrl+Print). Nothing is written to the home.
    Clipboard,
    /// Saved only: a mind's screenshot, which must not put the person's screen on the person's
    /// clipboard, where the next paste would carry it somewhere they did not mean.
    File,
}

pub struct Shot {
    pub display: Display,
    pub area: Area,
    pub delay: Duration,
    pub pointer: bool,
    pub keep: Keep,
    pub who: Who,
}

/// What came of it.
#[derive(Debug)]
pub enum Taken {
    Saved(PathBuf),
    Copied,
}

/// The person pressed Esc in `slurp`. Not an error anybody needs to be told about.
pub const CANCELLED: &str = "cancelled";

/// Ask the person for a rectangle. `Err(CANCELLED)` when they pressed Esc.
pub fn drag() -> Result<String, String> {
    let mut slurp = Command::new("slurp");
    slurp.env_remove("WAYLAND_SOCKET");
    let out = grab::run_bounded(slurp, SELECT_LIMIT, "the region selector (slurp)").map_err(|e| {
        // slurp exits 1 on Esc, which run_bounded reports as a failure with no words.
        if e.contains("failed") { CANCELLED.to_string() } else { e }
    })?;
    let geometry = String::from_utf8_lossy(&out).trim().to_string();
    if geometry.is_empty() {
        return Err(CANCELLED.into());
    }
    Ok(geometry)
}

/// Take it. Blocking — slurp waits for the person, then the delay, then grim: a worker.
/// The furniture is already hidden by the caller.
pub fn take(shot: Shot) -> Result<Taken, String> {
    let region = match &shot.area {
        Area::Screen => None,
        Area::Given(g) => Some(g.clone()),
        Area::Drag => {
            let g = drag()?;
            std::thread::sleep(SETTLE);
            Some(g)
        }
    };
    std::thread::sleep(shot.delay.max(SETTLE));
    // Nothing is captured while the screen is locked: a timer that ran out after Super+L must
    // not photograph whatever the lock covers.
    if super::locked_now() {
        return Err("the screen locked before the picture was taken, so none was".into());
    }
    let at = Local::now();
    let png = grab::grab_bytes(shot.display, region.as_deref(), Format::Png, shot.pointer)?;
    // And again after: a lock that landed while grim ran (up to its 10 s limit) may be in the
    // picture, or what it was about to cover. The picture is only in memory here, so dropping it
    // keeps nothing: no file, no clipboard (security review of #828, finding 6).
    if super::locked_now() {
        return Err("the screen locked while the picture was being taken, so it was thrown away".into());
    }
    match shot.keep {
        Keep::Clipboard => copy_to_clipboard(&png).map(|()| Taken::Copied),
        Keep::File | Keep::FileAndClipboard => {
            let path = save(&png, &at)?;
            if shot.keep == Keep::FileAndClipboard {
                // Saved is what matters; a clipboard that could not be reached is logged, and the
                // notification still offers the file.
                if let Err(e) = copy_to_clipboard(&png) {
                    tracing::warn!(error = %e, "The screenshot was saved but not copied to the clipboard");
                }
            }
            tracing::info!(path = %path.display(), by = %shot.who.words(), display = shot.display.name(), "Screenshot saved");
            Ok(Taken::Saved(path))
        }
    }
}

/// Write the picture into a new screenshot file of the shell's own making, and answer its path.
/// A write that fails takes the half-written file with it.
pub(crate) fn save(png: &[u8], at: &chrono::DateTime<Local>) -> Result<PathBuf, String> {
    let (path, mut file) = names::create_fresh(Kind::Screenshot, &names::home()?, at)?;
    if let Err(e) = file.write_all(png).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("could not save the picture: {e}"));
    }
    Ok(path)
}

/// `wl-copy --type image/png`, the picture on its stdin. wl-copy reads all of it before it forks
/// to serve the clipboard.
fn copy_to_clipboard(png: &[u8]) -> Result<(), String> {
    let mut cmd = Command::new("wl-copy");
    cmd.args(["--type", "image/png"]).env_remove("WAYLAND_SOCKET");
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start wl-copy: {e}"))?;
    let written = child.stdin.take().map(|mut stdin| stdin.write_all(png));
    let out = child.wait_with_output().map_err(|e| format!("wl-copy: {e}"))?;
    if let Some(Err(e)) = written {
        return Err(format!("could not hand the picture to wl-copy: {e}"));
    }
    if !out.status.success() {
        return Err(format!("wl-copy failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Security review of #828, finding 3: grim given a path opens it with O_TRUNC and follows a
    /// link planted at the predictable next name. A screenshot is grim's stdout, written into a
    /// file the shell made exclusively, 0600 (names::create_fresh, whose own test holds that).
    #[test]
    fn grim_never_opens_a_screenshot_file_itself() {
        let src = include_str!("shot.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        assert!(!src.contains("grab::grab("), "grim is handed a path");
        assert!(src.contains("grab::grab_bytes("));
        assert!(src.contains("names::create_fresh(Kind::Screenshot"));
        assert!(!src.contains("fresh_path"), "a name checked, then opened by someone else");
    }

    /// Security review of #828, finding 6: the lock is asked before grim runs and again after it
    /// returns, before anything is kept - a file or the clipboard.
    #[test]
    fn a_lock_that_lands_during_the_grab_keeps_nothing() {
        let src = include_str!("shot.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        let take = &src[src.find("pub fn take(").unwrap()..];
        let grab = take.find("grab::grab_bytes(").unwrap();
        let before = take[..grab].rfind("super::locked_now()").expect("asked before the grab");
        let after = grab + take[grab..].find("super::locked_now()").expect("asked again after it");
        let kept = [take.find("copy_to_clipboard(&png)").unwrap(), take.find("save(&png").unwrap()];
        assert!(before < grab && kept.iter().all(|k| after < *k), "the second ask comes before anything is kept");
    }
}
