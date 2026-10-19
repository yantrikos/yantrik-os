//! What a capture tells the person, and the three buttons on it: Open, Show in Files, Delete.
//!
//! Sent as `Yantrik`, the desktop's own name, through the notifications service, so the
//! notification centre keeps it and a mind can see what was said. The buttons are carried out
//! here, inside the shell, never on the control surface: they act on a file in the person's
//! home, and a button press is the person's. `wire::notifications` hands a press to [`press`]
//! only for a notification the service says the desktop itself sent (`Sender::desktop`), and
//! [`press`] acts only on a file this desktop saved (`names::is_capture`). A notification filed
//! by anyone else under the desktop's name, with a button naming `~/.ssh`, moves nothing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use yantrik_app_runtime::notify::{self, Level, Notification};

use super::names;
use super::recorder::Why;
use super::Who;

pub const OPEN: &str = "capture_open_file";
pub const SHOW: &str = "capture_show_file";
pub const DELETE: &str = "capture_delete_file";

/// Whether a button id is one of the three this module carries out.
pub fn handles(action_id: &str) -> bool {
    matches!(action_id, OPEN | SHOW | DELETE)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn with_buttons(n: Notification, path: &Path) -> Notification {
    let args = serde_json::json!({ "path": path.to_string_lossy() });
    n.action_with(OPEN, "Open", args.clone())
        .action_with(SHOW, "Show in Files", args.clone())
        .action_with(DELETE, "Delete", args)
}

/// A screenshot the person took, saved and copied.
pub fn screenshot_saved(path: &Path) {
    let n = Notification::new("Yantrik", "Screenshot saved")
        .body(format!("{}, in Pictures/Screenshots. Copied to the clipboard.", file_name(path)))
        .image(path.to_string_lossy())
        .urgency(Level::Low);
    notify::send(with_buttons(n, path));
}

/// Ctrl+Print: on the clipboard, no file.
pub fn screenshot_copied() {
    notify::send(
        Notification::new("Yantrik", "Screenshot copied to the clipboard")
            .body("Not saved to a file.")
            .urgency(Level::Low),
    );
}

/// A mind took a screenshot. Every one is told, naming the mind, whichever display it was of.
pub fn screenshot_by_mind(mind: &str, path: &Path, desktop: bool) {
    let (title, urgency) = if desktop {
        (format!("{mind} took a screenshot of your desktop"), Level::Normal)
    } else {
        (format!("{mind} took a screenshot of Mind View"), Level::Low)
    };
    let n = Notification::new("Yantrik", title)
        .body(format!("{}, in Pictures/Screenshots.", file_name(path)))
        .image(path.to_string_lossy())
        .urgency(urgency);
    notify::send(with_buttons(n, path));
}

/// A mind opened one of the person's saved screenshots or recordings (`open_capture`), which the
/// person allowed on a card a moment ago. Told after the fact as a screenshot is, naming the mind
/// and the file, so the person sees what their Allow let it look at.
pub fn capture_opened_by_mind(mind: &str, path: &Path) {
    let n = Notification::new("Yantrik", format!("{mind} opened {}", file_name(path)))
        .body("One of your saved captures, opened in Mind View for the mind to look at.")
        .urgency(Level::Normal);
    notify::send(n);
}

/// A mind started recording Mind View. The red dot shows it too; this says whose it is.
pub fn recording_by_mind(mind: &str) {
    notify::send(
        Notification::new("Yantrik", format!("{mind} is recording Mind View"))
            .body("The red dot in the status bar stops it.")
            .urgency(Level::Normal),
    );
}

/// A screenshot that could not be taken. A cancelled selection is not news and is not sent.
pub fn screenshot_failed(why: &str) {
    if why == super::shot::CANCELLED {
        return;
    }
    notify::send(Notification::new("Yantrik", "The screenshot was not taken").body(why.to_string()).urgency(Level::Normal));
}

/// A recording that could not start.
pub fn recording_failed(why: &str) {
    if why == super::shot::CANCELLED {
        return;
    }
    notify::send(Notification::new("Yantrik", "The recording did not start").body(why.to_string()).urgency(Level::Normal));
}

/// "1 min 12 s", "42 s".
pub fn length_words(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 60, s % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// A recording ended. With a file worth keeping, its buttons; with none, why not.
pub fn recording_finished(path: &Path, size: u64, length: Duration, why: &Why) {
    let reason = why.sentence().map(|s| format!(" {s}")).unwrap_or_default();
    if size == 0 {
        let _ = std::fs::remove_file(path);
        notify::send(
            Notification::new("Yantrik", "The recording could not be saved")
                .body(format!("Nothing playable was written.{reason}"))
                .urgency(Level::Normal),
        );
        return;
    }
    // Stopping for the person's own reasons is quiet news; stopping by itself is worth seeing.
    let urgency = if matches!(why, Why::Person) { Level::Low } else { Level::Normal };
    let n = Notification::new("Yantrik", "Recording saved")
        .body(format!("{}, {}, in Videos/Recordings.{reason}", file_name(path), length_words(length)))
        .urgency(urgency);
    notify::send(with_buttons(n, path));
}

/// A recording a previous shell left running was stopped at startup.
pub fn leftover_stopped() {
    notify::send(
        Notification::new("Yantrik", "A screen recording was stopped")
            .body("It was still running when the desktop restarted. It is in Videos/Recordings.")
            .urgency(Level::Normal),
    );
}

/// The file a press names, if the press may act on it: sent by the desktop, a capture this
/// desktop saved, and still there.
fn target(args: Option<&serde_json::Value>, from_desktop: bool, home: &Path) -> Option<PathBuf> {
    if !from_desktop {
        return None;
    }
    let path = PathBuf::from(args?.get("path")?.as_str()?);
    (names::is_capture(&path, home) && path.is_file()).then_some(path)
}

/// Carry out one of the three buttons. UI thread: `wire::notifications::invoke_action`.
pub fn press(ui: &crate::App, action_id: &str, args: Option<&serde_json::Value>, from_desktop: bool) {
    let home = names::home().unwrap_or_default();
    let Some(path) = target(args, from_desktop, &home) else {
        tracing::warn!(action_id, from_desktop, "A capture button named no capture this desktop saved; nothing was done");
        return;
    };
    let name = file_name(&path);
    match action_id {
        OPEN => {
            // The same rule and launcher a double-click in Files uses: a picture in the image
            // viewer, a recording in mpv.
            let action = crate::mime_dispatch::classify(&name);
            crate::wire::open_with::launch(&action, &name, &path, Some(ui));
        }
        SHOW => {
            if let Some(dir) = path.parent() {
                crate::control_files::ensure_files_screen(ui);
                ui.invoke_file_navigate_to_path(dir.to_string_lossy().into_owned().into());
            }
        }
        DELETE => match crate::fileops::trash(&path, &crate::fileops::trash_root()) {
            // To the Trash, where Files can put it back: a button on a toast is too easy to
            // press by mistake for a delete that is final.
            Ok(_) => tracing::info!(path = %path.display(), "A capture was moved to the Trash from its notification"),
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "A capture could not be moved to the Trash"),
        },
        _ => {}
    }
}

impl Who {
    /// "the person", or the mind's name: `describe shell` and the log say who started it.
    pub fn words(&self) -> String {
        match self {
            Who::Person => "the person".into(),
            Who::Mind { name, .. } => name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_acts_only_on_a_capture_the_desktop_itself_announced() {
        let home = std::env::temp_dir().join(format!("yantrik-notice-{}", std::process::id()));
        let dir = home.join("Pictures/Screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("Screenshot 1999-01-01 at 00.00.00.png");
        std::fs::write(&real, b"png").unwrap();
        std::fs::write(home.join(".bashrc"), b"x").unwrap();
        let args = serde_json::json!({ "path": real.to_string_lossy() });
        assert_eq!(target(Some(&args), true, &home), Some(real.clone()));
        assert_eq!(target(Some(&args), false, &home), None, "filed under the desktop's name by someone else");
        let elsewhere = serde_json::json!({ "path": home.join(".bashrc").to_string_lossy() });
        assert_eq!(target(Some(&elsewhere), true, &home), None, "not a capture");
        let gone = serde_json::json!({ "path": dir.join("Screenshot 1999-01-01 at 00.00.01.png").to_string_lossy() });
        assert_eq!(target(Some(&gone), true, &home), None, "no longer there");
        assert_eq!(target(None, true, &home), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn lengths_read_as_minutes_and_seconds() {
        assert_eq!(length_words(Duration::from_secs(42)), "42 s");
        assert_eq!(length_words(Duration::from_secs(120)), "2 min");
        assert_eq!(length_words(Duration::from_secs(72)), "1 min 12 s");
    }
}
