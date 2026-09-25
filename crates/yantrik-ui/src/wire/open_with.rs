//! Launching what the one MIME rule picked, from every path that asks.
//!
//! `mime_dispatch::classify` decides the app for a file; this module runs the decision. A
//! double-click in Files, an "Open with" row and the control surface's `files_open` all land
//! here, so the list a person chooses from and the launch that follows the choice cannot
//! disagree about how an app is started (#233).

use std::path::Path;

use crate::mime_dispatch::{self, FileAction};
use crate::App;

/// Run one `FileAction` on one file. `name` is the file's own name, for the notice that says
/// what could not play; `full` is its path. `ui` is the shell window, needed only to say a
/// missing media player on the Files screen — a caller without one still gets the launch, and
/// the refusal goes to the log instead.
pub fn launch(action: &FileAction, name: &str, full: &Path, ui: Option<&App>) {
    match action {
        // The same app the launcher opens, given the file that was double-clicked. It used
        // to load the picture into the shell's own screen instead, which is why the
        // standalone viewer could ship for months without anyone noticing it opened nothing.
        FileAction::ImageViewer => {
            super::dock::spawn_app_with_args(
                "image",
                "yantrik-image-viewer",
                &[&full.to_string_lossy()],
            );
        }
        FileAction::TextEditor => {
            super::dock::spawn_app_with_args(
                "editor",
                "yantrik-text-editor",
                &[&full.to_string_lossy()],
            );
        }
        FileAction::MediaPlayer => {
            // mpv's own window, opened the way its desktop entry opens it: pseudo-gui gives
            // a song a window with the play bar, where a bare `mpv song.mp3` plays with no
            // window at all and nothing to stop it by. Through the one launcher, so the
            // child does not inherit SLINT_FULLSCREEN.
            //
            // Said on the Files screen when there is no mpv: the image this desktop builds
            // does not install it, and a failed launch is otherwise only a log line.
            if super::dock::find_program("mpv").is_none() {
                let message = format!("{name} cannot play: no media player (mpv) is installed.");
                match ui {
                    // Through the shared setter, so a stale Open/Rename from an older
                    // creation notice does not stay behind under this message.
                    Some(ui) => super::files::set_notice(ui, &message, None),
                    None => tracing::error!("{message}"),
                }
                return;
            }
            let target = full.to_string_lossy().to_string();
            super::dock::spawn_app_with_args(
                "mpv",
                "mpv",
                &["--player-operation-mode=pseudo-gui", "--", target.as_str()],
            );
        }
        FileAction::Browser => {
            // The browser the Browser pin opens, with the file as its argument: the same
            // launcher, so the registry and the reaper see the window, and Chromium gets the
            // DevTools flags that let `yos web` drive the page a person just double-clicked.
            match super::dock::find_browser() {
                Some((bin, flags)) => {
                    let target = full.to_string_lossy().to_string();
                    let mut argv: Vec<&str> = flags.to_vec();
                    argv.push(&target);
                    super::dock::spawn_app_with_args("browser", bin, &argv);
                }
                None => tracing::error!(
                    path = %full.display(),
                    "Cannot open the file in a browser: none is installed"
                ),
            }
        }
        FileAction::DesktopApp(id) => {
            // An app the person chose, found in the same catalogue the choice was listed
            // from and launched through the one launcher, adapter included (#233).
            let installed = crate::apps::Catalogue::shared().get();
            let stem = id.strip_suffix(".desktop").unwrap_or(id);
            match installed
                .iter()
                .find(|e| e.app_id.eq_ignore_ascii_case(stem))
            {
                Some(entry) => super::dock::launch_entry_with_file(
                    entry,
                    &installed,
                    &full.to_string_lossy(),
                ),
                None => tracing::error!(
                    app = %id,
                    path = %full.display(),
                    "Cannot open the file: the app is not installed any more"
                ),
            }
        }
    }
}

/// Open one file in the app a desktop entry id names — one row of Files' "Open with" list, or
/// the default a `mimeapps.list` line points at. An id the shell has a route of its own to
/// launches through that route, exactly as a double-click that lands on the same app does:
/// whichever browser the person chose, the shell opens the browser this machine has (#233).
pub fn launch_desktop_id(id: &str, name: &str, full: &Path, ui: Option<&App>) {
    let action = mime_dispatch::action_for_desktop(id)
        .unwrap_or_else(|| FileAction::DesktopApp(id.to_string()));
    launch(&action, name, full, ui)
}
