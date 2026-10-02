//! Pinned apps — one list, with one job.
//!
//! The desktop used to offer the same applications three times. The START row showed a
//! hardcoded list of sixteen, rebuilt every three seconds by the system poll; the taskbar showed
//! the first six of that same list as pins; and the Apps launcher showed everything installed.
//! None of the three was a choice anybody had made. START even carried a tile for `launchpad` —
//! the Apps button itself — so the desktop offered a shortcut to the launcher sitting two inches
//! below it in the corner.
//!
//! The surfaces now do different jobs:
//!
//! - **START** is the person's shortcuts: this list, in this order, and nothing else.
//! - **Apps** is everything installed, and it is where pins are made and unmade.
//! - **The taskbar** is what is running.
//!
//! So the question "which apps are on my desktop" has one answer, it lives in `settings.yaml`,
//! and it is whatever the person last decided.

use slint::{ModelRc, SharedString, VecModel};

use crate::apps::DesktopEntry;
use crate::{App, DockItem, Tr};

/// What a machine pins before anybody has chosen.
///
/// The six things a person reaches for on a first day: somewhere to find files, somewhere to
/// browse, somewhere to type commands, somewhere to write, mail and a calendar. Deliberately not
/// Memory, System or Alerts — those are this shell's own screens and belong in the launcher and
/// the status bar, not on the desktop of someone who has not asked for them.
pub const DEFAULT_PINS: &[&str] = &["files", "browser", "terminal", "notes", "email", "calendar"];

/// Ids that must never be pinned, because pinning them duplicates something always on screen.
///
/// `launchpad` is the Apps launcher. Its button is permanently in the taskbar's corner, and a
/// START tile that opens the thing START sits next to is the exact duplication this module was
/// written to remove.
const NEVER_PINNED: &[&str] = &["launchpad"];

/// The id a pin is stored under.
///
/// The catalogue names this OS's own apps by their .desktop filename (`yantrik-notes`), which has
/// to be unique across the whole machine; the rest of the shell calls the same app `notes`. A
/// pin made from the launcher and a pin from the defaults must be the same pin, so both go
/// through here.
pub fn pin_id(app_id: &str) -> String {
    let bare = app_id.strip_prefix("yantrik-").unwrap_or(app_id);
    match bare {
        // The catalogue knows the browser by its package, the shell by its job. Launching
        // `chromium` from its .desktop file runs it without the Wayland flags and the separate
        // profile the `browser` arm exists to supply — so the pin is always `browser`, and the
        // launcher's Chromium tile shows as pinned when it is.
        "chromium" | "chromium-browser" => "browser".to_string(),
        other => other.to_string(),
    }
}

pub fn is_pinnable(app_id: &str) -> bool {
    !NEVER_PINNED.contains(&pin_id(app_id).as_str())
}

pub fn is_pinned(app_id: &str) -> bool {
    let id = pin_id(app_id);
    super::settings::pinned_apps().iter().any(|p| *p == id)
}

/// Pin it if it is not pinned, unpin it if it is. Returns whether it is pinned afterwards.
///
/// A new pin goes on the END: the row is in the order the person built it, and a new arrival
/// jumping to the front would reorder everything they had already learned to find.
pub fn toggle(app_id: &str) -> bool {
    let id = pin_id(app_id);
    if !is_pinnable(&id) {
        return false;
    }
    let mut pins = super::settings::pinned_apps();
    let now_pinned = if let Some(at) = pins.iter().position(|p| *p == id) {
        pins.remove(at);
        false
    } else {
        pins.push(id.clone());
        true
    };
    tracing::info!(app = %id, pinned = now_pinned, "Pins changed");
    super::settings::set_pinned_apps(pins);
    now_pinned
}

/// The shell's own apps, with their translated labels.
///
/// This was the whole of START: the hardcoded list the poll rebuilt. It is now only a label
/// table — which apps appear is the pinned list's business.
pub(super) fn builtin_label(tr: &Tr, id: &str) -> Option<SharedString> {
    Some(match id {
        "terminal" => tr.get_dock_terminal(),
        "browser" => tr.get_dock_browser(),
        "files" => tr.get_dock_files(),
        "email" => tr.get_dock_email(),
        "notes" => tr.get_dock_notes(),
        "editor" => tr.get_dock_editor(),
        "memory" => tr.get_dock_memory(),
        "notifications" => tr.get_dock_alerts(),
        "system" => tr.get_dock_system(),
        "media" => tr.get_dock_media(),
        "calendar" => tr.get_dock_calendar(),
        "spreadsheet" => tr.get_dock_ysheets(),
        "documents" => tr.get_dock_ydoc(),
        "presentation" => tr.get_dock_ypresent(),
        "settings" => tr.get_dock_settings(),
        _ => return None,
    })
}

/// "download-manager" → "Download Manager", for an app nothing else could name.
pub(super) fn humanise(id: &str) -> String {
    id.split(['-', '_', '.'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Put the pinned list on the desktop.
///
/// Called by the system poll, which is also what knows which windows are open, and by the
/// launcher the moment a pin changes — so a pin appears on START when you make it rather than up
/// to three seconds later, which would read as the click not having worked.
pub fn publish(ui: &App, installed: &[DesktopEntry]) {
    use slint::ComponentHandle;

    let tr = ui.global::<Tr>();
    let wins = crate::windows::list_windows_throttled();

    let items: Vec<DockItem> = super::settings::pinned_apps()
        .iter()
        .filter(|id| is_pinnable(id))
        // A pin is a promise that clicking it opens something. Browser was pinned by default on
        // every machine and ran `chromium`, which the installer does not put on the disk, so
        // START's second tile did nothing at all. The pin stays in settings — install a browser
        // and it comes back — but a tile that cannot open is not shown.
        .filter(|id| super::dock::is_launchable(id, installed))
        .map(|id| {
            let entry = installed
                .iter()
                .find(|e| e.app_id == *id || pin_id(&e.app_id) == *id);

            let label = builtin_label(&tr, id)
                .or_else(|| entry.map(|e| SharedString::from(e.name.as_str())))
                .unwrap_or_else(|| SharedString::from(humanise(id)));

            // A real icon only for apps this shell has no glyph for. The shell's own apps are
            // drawn in its own stroke set, like everywhere else on the desktop; a pinned Chromium
            // gets Chromium's icon rather than a generic category shape.
            let icon = match (builtin_label(&tr, id), entry) {
                (None, Some(e)) => crate::icons::resolve(&e.icon),
                _ => None,
            };

            DockItem {
                app_id: id.clone().into(),
                label,
                icon_char: SharedString::default(),
                icon_id: super::app_grid::icon_id_for(id).into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
                is_running: wins.iter().any(|w| w.app_id == *id),
            }
        })
        .collect();

    if let Some(model) = crate::models::changed(ui.get_dock_items(), items) {
        ui.set_dock_items(model);
    }
    // A pin made or unmade is a button made or unmade in the dock, now, not on the next poll.
    super::dock_bar::publish(ui, installed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launcher's name for one of our apps and the defaults' name for it are the same pin.
    #[test]
    fn a_catalogue_id_and_a_shell_id_are_one_pin() {
        assert_eq!(pin_id("yantrik-notes"), "notes");
        assert_eq!(pin_id("notes"), "notes");
        assert_eq!(pin_id("org.gnome.Nautilus"), "org.gnome.Nautilus");
    }

    /// Pinning Chromium from the launcher and the default Browser pin are one pin, launched the
    /// shell's way.
    #[test]
    fn chromium_is_the_browser() {
        assert_eq!(pin_id("chromium"), "browser");
    }

    /// The launcher cannot be pinned next to its own button.
    #[test]
    fn the_launcher_is_never_a_pin() {
        assert!(!is_pinnable("launchpad"));
        assert!(!DEFAULT_PINS.contains(&"launchpad"));
        assert!(is_pinnable("notes"));
    }

    /// Every default is something the shell can actually launch. A default pin that did nothing
    /// when clicked would be the first thing a new person tried.
    #[test]
    fn every_default_pin_launches() {
        // The shell's own routes, and the apps this OS ships through their .desktop files.
        let shipped = crate::surfaces::shipped_catalogue();
        for pin in DEFAULT_PINS {
            assert!(
                !matches!(
                    super::super::dock::resolve(pin, &shipped),
                    super::super::dock::Resolved::Unknown | super::super::dock::Resolved::Shelved(_)
                ),
                "default pin `{pin}` is not an app the shell knows how to launch"
            );
        }
    }

    /// No default pin is a shelved app, and a pin left in settings.yaml for one does not draw.
    ///
    /// `publish` filters on `is_launchable`, which the shelf answers no to — so a person who
    /// pinned ySheets before it was shelved keeps the line in their settings and loses the tile,
    /// which is the same thing that happens to a pin for an app they uninstalled. Un-shelve it
    /// and the pin comes back on its own.
    #[test]
    fn a_shelved_app_is_never_on_start() {
        for pin in DEFAULT_PINS {
            assert!(super::super::dock::shelved(pin).is_none(), "default pin `{pin}` is shelved");
        }
        for id in ["music", "spreadsheet", "ySheets"] {
            assert!(!super::super::dock::is_launchable(id, &[]), "`{id}` would draw a START tile");
        }
    }

    #[test]
    fn names_nothing_else_could_name_are_readable() {
        assert_eq!(humanise("download-manager"), "Download Manager");
        assert_eq!(humanise("org.gnome.Nautilus"), "Org Gnome Nautilus");
    }
}
