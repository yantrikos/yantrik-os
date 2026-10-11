//! What a running window is called, and whose tile it wears: one answer for the dock, the
//! launcher's Running section (which is the dock's rows) and the window overview (Super+Tab).
//!
//! Each surface used to name a window its own way. The dock used the desktop entry's name for a
//! program it had no glyph for, so the old foot terminal was "Foot" on the dock and in Running;
//! the overview used the app id it was handed, and our own Slint windows declare none, so Calendar
//! and the desktop itself had no name there at all. One function now, and every surface asks it.

use crate::apps::DesktopEntry;
use crate::windows::{APP_NAMES, SHELL_WINDOW_TITLE};

/// Programs that are not ours but that the shell already treats as one of its apps. foot was the
/// shell's Terminal before the Terminal app existed, and it still opens the sign-in and agent
/// terminals, so its window is the Terminal to a person: named so and drawn with Terminal's tile.
///
/// Only the name and the tile follow; the window keeps its own id, so opening Terminal still starts
/// Terminal rather than bringing a sign-in window forward (`already_open`).
const STAND_INS: &[(&str, &str)] = &[("foot", "terminal"), ("footclient", "terminal")];

/// The id whose tile, colour and name a window of the app `app_id` wears: the icon set's key for
/// one of ours (`yantrik-notes` is `notes`), a stand-in's app, or the id itself.
pub fn tile_id(app_id: &str) -> String {
    let id = crate::wire::dock::window_id(app_id);
    STAND_INS.iter().find(|(program, _)| *program == id).map_or(id, |(_, app)| (*app).to_string())
}

/// The display name of a running window of `app_id`, titled `title`.
///
/// The shell's own name for the app wins, then the desktop entry's `Name=`, because those are the
/// names a person reads on the dock and in the launcher; a binary or an app id is not a name. A
/// window that is neither ours nor installed is called by its title, then by its app id, and a
/// window is never left unnamed.
pub fn display_name(app_id: &str, title: &str, installed: &[DesktopEntry]) -> String {
    if title == SHELL_WINDOW_TITLE {
        return "Desktop".to_string();
    }
    let id = tile_id(app_id);
    if id == crate::mind_view::APP_ID {
        return "Mind View".to_string();
    }
    if let Some((_, name)) = APP_NAMES.iter().find(|(app, _)| *app == id) {
        return (*name).to_string();
    }
    // By the entry's own id, case aside (the shell keeps a declared id lowercased, and the file is
    // `org.gnome.Calculator.desktop`), or by the id the shell lists its window under (`notes` for
    // `yantrik-notes`).
    let entry = installed.iter().filter(|e| !e.name.trim().is_empty()).find(|e| {
        (!app_id.is_empty() && e.app_id.eq_ignore_ascii_case(app_id)) || crate::wire::dock::window_id(&e.app_id) == id
    });
    if let Some(e) = entry {
        return e.name.trim().to_string();
    }
    if !title.trim().is_empty() {
        return title.trim().to_string();
    }
    if !app_id.trim().is_empty() {
        return crate::windows::app_display_name(app_id.trim());
    }
    "Window".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(app_id: &str, name: &str) -> DesktopEntry {
        DesktopEntry { app_id: app_id.into(), name: name.into(), ..Default::default() }
    }

    fn installed() -> Vec<DesktopEntry> {
        vec![entry("foot", "Foot"), entry("yantrik-calendar", "Calendar"), entry("org.gnome.Calculator", "Calculator"), entry("files", "Files")]
    }

    /// The bug on the test machine: the old foot window was "Foot" in the launcher's Running
    /// section while the dock called the terminal "Terminal". The shell's own name wins over the
    /// desktop entry's, and the tile follows the name.
    #[test]
    fn the_shells_name_wins_over_the_binarys() {
        assert_eq!(display_name("foot", "~/src", &installed()), "Terminal");
        assert_eq!(tile_id("foot"), "terminal");
        assert_eq!(display_name("terminal", "Terminal", &installed()), "Terminal");
    }

    /// Calendar declares no Wayland app id; once its id is known from its title it is named by the
    /// shell, and a window the shell has no name for takes its desktop entry's.
    #[test]
    fn our_apps_and_installed_apps_are_named_by_their_names() {
        assert_eq!(display_name("calendar", "Calendar", &installed()), "Calendar");
        assert_eq!(display_name("yantrik-system-monitor", "System Monitor", &[]), "System Monitor");
        assert_eq!(display_name("org.gnome.calculator", "Calculator — 2+2", &installed()), "Calculator");
        assert_eq!(display_name("files", "Files", &installed()), "Files");
    }

    /// Why the overview had two unnamed windows: it named the app id each window DECLARED, and our
    /// Slint windows declare none. Resolved first, as the window list does, Calendar is Calendar.
    #[test]
    fn a_window_that_declares_no_app_id_is_named_from_its_resolved_id() {
        let calendar = crate::windows::shell_app_id("", "Calendar");
        assert_eq!(calendar, "calendar");
        assert_eq!(display_name(&calendar, "Calendar", &[]), "Calendar");
        assert_eq!(tile_id(&calendar), "calendar", "and wears Calendar's tile");
        assert_eq!(display_name(&crate::windows::shell_app_id("foot", "~/src"), "~/src", &installed()), "Terminal");
    }

    /// Never blank: no entry and no known id falls back to the title, then the app id.
    #[test]
    fn an_unknown_window_falls_back_to_its_title_then_its_app_id() {
        assert_eq!(display_name("xterm", "pranab@box: ~", &[]), "pranab@box: ~");
        assert_eq!(display_name("xterm", "  ", &[]), "Xterm");
        assert_eq!(display_name("", "", &[]), "Window");
        assert_eq!(display_name("", "Yantrik OS", &[]), "Desktop");
        assert_eq!(display_name("mind-view", "wlroots - WL-1", &[]), "Mind View");
    }

    /// The dock (and so the launcher's Running) and the overview name windows through this one
    /// function; a second naming rule in either is how "Foot" and "Terminal" came apart.
    #[test]
    fn the_dock_and_the_overview_both_ask_this_function() {
        let code = |src: &str| src.split("#[cfg(test)]").next().unwrap().to_string();
        let dock = code(include_str!("wire/dock_bar.rs"));
        assert!(dock.contains("crate::window_name::display_name("), "the dock names running apps here");
        assert!(dock.contains("crate::window_name::tile_id("), "and draws their tiles from here");
        assert!(!dock.contains("app_display_name("), "no second rule on the dock");
        let switcher = code(include_str!("control_switcher.rs"));
        assert!(switcher.contains("crate::window_name::display_name("), "the overview names its cells here");
        assert!(switcher.contains("crate::window_name::tile_id("));
        assert!(!switcher.contains("app_display_name("), "no second rule in the overview");
    }
}
