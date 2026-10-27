//! What an agent or the Mind may open or launch, decided in one place (#942, #953).
//!
//! A person opens a file in whatever app handles it. A non-person caller does not: a browser
//! that opens a local page shows other local files on screen (a frame of `file:///home/…/.ssh/…`
//! renders though script cannot read it), and LibreOffice evaluates links and macros with all of
//! the person's rights. Only Yantrik's own framework apps apply `home_paths` and Private mode
//! themselves, so they are the only apps an agent may reach at all. Even they give a Mind no
//! window (#978 step 1): the refusal says to use the app's actions and `os_describe`, which start
//! the app on demand with no window. Every shell route that opens a file or starts an app asks
//! here, so no handler is a gap to close one type at a time.
//!
//! Anything that cannot be told is refused: a handler that is not installed may be anything.

use crate::apps::{exec_argv, DesktopEntry};
use crate::framework;
use crate::office::{is_libreoffice, CLOSED_TO_AGENTS};

/// Names that mean LibreOffice to a caller, whether or not an entry is installed under them.
const OFFICE_NAMES: &[&str] = &["libreoffice", "soffice", "writer", "calc", "impress", "draw", "office"];

/// What the shell's one MIME rule picked to open a file in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Handler<'a> {
    /// `yantrik-image-viewer`.
    ImageViewer,
    /// `yantrik-text-editor`.
    TextEditor,
    /// The shell's media route, which is mpv.
    MediaPlayer,
    /// The machine's web browser.
    Browser,
    /// An installed app, by desktop entry id (with or without `.desktop`).
    Desktop(&'a str),
}

fn canon(s: &str) -> String {
    s.trim().to_lowercase().replace([' ', '-'], "_")
}

/// The program an entry runs, by its file name.
fn program(entry: &DesktopEntry) -> String {
    exec_argv(&entry.exec, None)
        .first()
        .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
        .unwrap_or_default()
}

fn shown(entry: &DesktopEntry) -> &str {
    if entry.name.is_empty() { &entry.app_id } else { &entry.name }
}

/// Whether `entry` is one of Yantrik's framework apps: listed, and running its own program.
pub fn is_yantrik_app(entry: &DesktopEntry) -> bool {
    !is_libreoffice(entry)
        && framework::FRAMEWORK_APPS.iter().any(|a| a.program == entry.app_id && program(entry) == a.program)
}

/// The refusal for a Mind asking for a window of the framework app that runs `program`: its name
/// as the catalogue shows it, and the surface to use instead (#978 step 1).
fn no_window_for_minds(program: &str, installed: &[DesktopEntry]) -> String {
    let app = framework::by_program(program).expect("a framework program");
    let name = installed
        .iter()
        .find(|e| e.app_id == app.program && !e.name.is_empty())
        .map_or_else(|| app.id.to_string(), |e| e.name.clone());
    framework::refused_for_minds(&name, app.id)
}

fn not_a_yantrik_app(what: &str) -> String {
    format!("Agents can open files only in Yantrik's own apps; {what} isn't one of them. Nothing was opened.")
}

/// The verdict for an agent or the Mind opening a file in `handler`, `installed` being the
/// catalogue. Call it only for a non-person caller.
pub fn may_open_file(handler: Handler, installed: &[DesktopEntry]) -> Result<(), String> {
    match handler {
        Handler::ImageViewer => Err(no_window_for_minds("yantrik-image-viewer", installed)),
        Handler::TextEditor => Err(no_window_for_minds("yantrik-text-editor", installed)),
        // mpv follows playlists to other local files and draws subtitle files on screen, so a
        // protected text file would be readable through it. There is no framework video app.
        Handler::MediaPlayer => Err(not_a_yantrik_app("the media player")),
        Handler::Browser => Err(not_a_yantrik_app("the browser")),
        Handler::Desktop(id) => {
            let stem = id.trim().strip_suffix(".desktop").unwrap_or(id.trim());
            match installed.iter().find(|e| e.app_id.eq_ignore_ascii_case(stem)) {
                Some(e) if is_libreoffice(e) => Err(CLOSED_TO_AGENTS.to_string()),
                Some(e) if is_yantrik_app(e) => Err(no_window_for_minds(&e.app_id, installed)),
                Some(e) => Err(not_a_yantrik_app(shown(e))),
                None if OFFICE_NAMES.iter().any(|n| canon(stem).contains(n)) => Err(CLOSED_TO_AGENTS.to_string()),
                None => Err(not_a_yantrik_app(&format!("`{stem}` (not installed, so what it runs cannot be told)"))),
            }
        }
    }
}

/// The verdict for an agent or the Mind launching the app `name` (any spelling `open_app` takes).
/// A name no catalogue entry carries is one of the shell's own screens or routes, which
/// `check_launchable` has already vetted.
pub fn may_launch(name: &str, installed: &[DesktopEntry]) -> Result<(), String> {
    let want = canon(name);
    let bare = want.strip_prefix("yantrik_").unwrap_or(&want);
    if OFFICE_NAMES.contains(&bare) {
        return Err(CLOSED_TO_AGENTS.to_string());
    }
    let carries = |e: &&DesktopEntry| {
        let id = canon(&e.app_id);
        id == want
            || id.strip_prefix("yantrik_") == Some(want.as_str())
            || e.surface.as_deref().is_some_and(|s| canon(s) == want)
            || e.aliases.iter().any(|a| canon(a) == want)
    };
    for e in installed.iter().filter(carries) {
        if is_libreoffice(e) {
            return Err(CLOSED_TO_AGENTS.to_string());
        }
        if is_yantrik_app(e) {
            return Err(no_window_for_minds(&e.app_id, installed));
        }
        if e.exec != "__builtin__" && !program(e).starts_with("yantrik-") {
            return Err(format!(
                "Agents can launch only Yantrik's own apps; {} isn't one of them. Nothing was launched.",
                shown(e)
            ));
        }
    }
    // An app of the framework by its surface id or program, with no catalogue entry to say so.
    if let Some(app) = framework::FRAMEWORK_APPS.iter().find(|a| canon(a.id) == bare || canon(a.program) == want) {
        return Err(no_window_for_minds(app.program, installed));
    }
    Ok(())
}

#[cfg(test)]
#[path = "agent_open_tests.rs"]
mod tests;
