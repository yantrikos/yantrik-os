//! What each app this OS ships is called, and the id its icon is keyed by: one list, read by
//! the shell (yantrik-ui) and by Alt+Tab (yantrik-switcher), which knows a window of ours only by
//! its title because our apps set no Wayland app_id (#877).

/// What each app this OS ships is CALLED. One list, because there were four.
///
/// The same application answered to a different name depending on which surface you were
/// looking at: the dock said "Editor", the launcher said "Text Editor", the window title said
/// "Text Editor" and the header said whatever the screen author wrote. Downloads was
/// "Download Manager" in three places and "Downloads" in a fourth. No single one of those was
/// wrong, which is exactly why it survived — it only reads as sloppy when you see two at once,
/// and you always do: the taskbar entry sits directly beneath the window it names.
///
/// Short names, because that is the family the dock already used and the dock is the surface a
/// person reads most. The suffixes went rather than being invented away: Container Manager to
/// Containers, Music Player to Music, Image Viewer to Images. The office three keep the brand
/// the dock gave them.
///
/// Keys are the shell's own app ids on the left, matching `Icons.app`, and the .desktop file
/// stems for the rest. `app_names_agree_everywhere` in yantrik-ui's windows.rs reads the .desktop
/// files and each app's Window title and fails if any of them drifts from this.
pub const APP_NAMES: &[(&str, &str)] = &[
    ("arcade", "Arcade"),
    ("browser", "Browser"),
    ("calendar", "Calendar"),
    ("containers", "Containers"),
    ("documents", "yDoc"),
    ("downloads", "Downloads"),
    ("editor", "Editor"),
    ("email", "Email"),
    ("image", "Images"),
    ("music", "Music"),
    ("network", "Network"),
    ("notes", "Notes"),
    ("presentation", "yPresent"),
    ("snippets", "Snippets"),
    ("spreadsheet", "ySheets"),
    ("studio", "Studio"),
    ("sysmonitor", "System Monitor"),
    ("terminal", "Terminal"),
    ("weather", "Weather"),
];

/// The app id whose window is titled exactly this, if it is one of ours.
///
/// Exactly, not loosely: "Notes" is Notes and "Notes: Handover" is a note open in it, and a
/// substring match would make the second one a second copy of the first in every window list.
pub fn app_id_for_title(title: &str) -> Option<&'static str> {
    APP_NAMES.iter().find(|(_, name)| *name == title).map(|(id, _)| *id)
}

/// The id the icon set is keyed by, for one of the apps this OS ships.
///
/// Two naming schemes meet here and neither is wrong. A freedesktop entry needs a name unique
/// across everything installed on the machine, so ours are `yantrik-download-manager`. The
/// icon set is keyed by what the rest of the shell calls the same app, which is `downloads`.
/// Stripping the prefix gets six of the sixteen; the other ten need saying out loud.
///
/// Anything not listed keeps its own id, so a third-party app is unaffected and a new app that
/// happens to match an icon name works without an entry.
pub fn icon_id_for(app_id: &str) -> String {
    let bare = app_id.strip_prefix("yantrik-").unwrap_or(app_id);
    let mapped = match bare {
        "container-manager" => "containers",
        "document-editor" => "documents",
        "download-manager" => "downloads",
        "image-viewer" => "image",
        "music-player" => "music",
        "network-manager" => "network",
        "snippet-manager" => "snippets",
        "system-monitor" => "sysmonitor",
        "text-editor" => "editor",
        other => other,
    };
    mapped.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_of_ours_is_known_by_its_exact_title() {
        assert_eq!(app_id_for_title("System Monitor"), Some("sysmonitor"));
        assert_eq!(app_id_for_title("Notes"), Some("notes"));
        assert_eq!(app_id_for_title("Notes: Handover"), None);
    }
}
