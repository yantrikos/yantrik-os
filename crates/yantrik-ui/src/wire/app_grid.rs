//! App grid — populate grid apps from installed apps, handle launch, search and categories.

use std::sync::Arc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::app_context::AppContext;
use crate::apps::DesktopEntry;
use crate::icons;
use crate::{App, AppGridItem};

pub fn wire(ui: &App, ctx: &AppContext) {
    let catalogue = ctx.installed_apps.clone();
    let installed = catalogue.get();

    // Rescan every time the launcher opens.
    //
    // The catalogue used to be scanned once at startup, so an app installed while the shell was
    // running simply did not exist: not in the launcher, not in the Lens, not to `open_app`.
    // The launcher opening is exactly the moment someone who has just installed something goes
    // looking for it, and a scan is a few directories of small ini files -- cheap enough to do
    // on a keystroke and far cheaper than being wrong.
    {
        let catalogue = catalogue.clone();
        let weak = ui.as_weak();
        ui.on_app_grid_opened(move || {
            // The launcher is part of the shell's own window, and with an app in front that
            // window is behind it: from the Editor, the Apps button opened a launcher nobody
            // could see (#219). Bring the shell forward; when the compositor will not, say so
            // where a launcher that never appeared can be traced.
            if let Err(why) = crate::windows::raise_shell() {
                tracing::warn!(%why, "The launcher opened, but the shell could not be brought in front of the window over it");
            }
            let count = catalogue.refresh();
            tracing::debug!(apps = count, "rescanned installed apps for the launcher");
            if let Some(ui) = weak.upgrade() {
                // The search field is recreated on open, empty; the query and Running go with it.
                super::launcher::reset(&ui);
                populate_grid(&ui, &catalogue.get(), "");
            }
        });
    }

    populate_grid(ui, &installed, "");

    // One query filters both sections: Running here through the launcher module, All apps below.
    {
        let catalogue = catalogue.clone();
        let ui_weak = ui.as_weak();
        ui.on_grid_search_apps(move |q| {
            if let Some(ui) = ui_weak.upgrade() {
                super::launcher::set_query(&ui, &q);
                populate_grid(&ui, &catalogue.get(), &q);
            }
        });
    }

    // Pin or unpin, from the launcher — the one place every app is listed, and so the one place
    // a pin can always be made or undone.
    {
        let catalogue = catalogue.clone();
        let ui_weak = ui.as_weak();
        ui.on_grid_toggle_pin(move |app_id| {
            super::pins::toggle(&app_id);
            let Some(ui) = ui_weak.upgrade() else { return };
            let apps = catalogue.get();
            populate_grid(&ui, &apps, &super::launcher::query());
            // START updates now, not on the next three-second poll — a pin that takes three
            // seconds to appear reads as a click that did not work.
            super::pins::publish(&ui, &apps);
        });
    }

    // Handle grid-launch-app — routes built-in apps to screens, external apps to processes
    let ui_weak = ui.as_weak();
    ui.on_grid_launch_app(move |app_id| {
        let app_id_str = app_id.as_str();

        // Close grid first
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_app_grid_open(false);
        }

        // Every tile opens through the dock's one dispatch, `launch_app`, whatever it is.
        //
        // Program tiles used to be spawned here, from the Exec line, and that was a third launch
        // path beside the dock's and `open_app`'s. It had already been brought into the shell's
        // one launcher once — it had spawned a bare Command that inherited SLINT_FULLSCREEN=1, so
        // an app opened from the grid came up fullscreen with no way out ("I opened notes app
        // and now its showing no option to close") — but it still chose its own registry id
        // (the file name without `yantrik-`, so System Monitor registered as `system-monitor`
        // while APP_NAMES and every other launch call it `sysmonitor`) and it could not start an
        // app's adapter. Through `launch_app` the tile opens exactly what `open_app` with the same
        // name would, under the same id, with the same adapter; Blender's tile gets its route.
        //
        // A Running tile is the dock's own button, so its id is the dock's and goes the same way:
        // it focuses the window that is open instead of starting a second copy.
        let installed = catalogue.get();
        let running = ui_weak
            .upgrade()
            .is_some_and(|ui| ui.get_grid_running().iter().any(|b| b.app_id == app_id));
        if let Some(entry) = installed.iter().find(|e| e.app_id == app_id_str) {
            tracing::info!(app = %entry.name, exec = %entry.exec, "Launching app from grid");
            if let Some(ui) = ui_weak.upgrade() {
                ui.invoke_launch_app(app_id);
            }
        } else if running {
            tracing::info!(app = %app_id_str, "Switching to a running app from the launcher");
            if let Some(ui) = ui_weak.upgrade() {
                ui.invoke_launch_app(app_id);
            }
        }
    });
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
pub(crate) fn icon_id_for(app_id: &str) -> String {
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
mod launcher_tests {
    /// With the Editor in front, the Apps button opened a launcher behind it (#219): the launcher
    /// is part of the shell's own window, and a Wayland client cannot raise itself. The handler
    /// for the grid opening has to ask the compositor, as `open_lens` and `show_screen` do.
    #[test]
    fn opening_the_launcher_brings_the_shell_forward() {
        let source = include_str!("app_grid.rs");
        // Split, so this test's own text is not what the search finds.
        let start = source.find(concat!("ui.on_app_grid_", "opened(")).expect("the grid-opened handler");
        let end = start + source[start..].find("\n        });").expect("its end");
        let handler = &source[start..end];
        assert!(
            handler.contains(concat!("crate::windows::raise_", "shell()")),
            "the launcher opening must bring the shell's window forward. Handler as written:\n{handler}"
        );
    }
}

#[cfg(test)]
mod one_per_app_tests {
    use super::one_per_app;
    use crate::apps::DesktopEntry;

    fn entry(app_id: &str, name: &str) -> DesktopEntry {
        DesktopEntry { app_id: app_id.into(), name: name.into(), exec: app_id.into(), ..Default::default() }
    }

    /// The distribution's Blender and ours were two tiles; ours is the one kept.
    #[test]
    fn our_entry_shadows_the_one_it_wraps() {
        let installed = vec![entry("blender", "Blender"), entry("yantrik-blender", "Blender"), entry("gimp", "GIMP")];
        let ids: Vec<&str> = one_per_app(&installed).iter().map(|e| e.app_id.as_str()).collect();
        assert_eq!(ids, ["yantrik-blender", "gimp"]);
    }

    /// Only the same app: a bare id that happens to match ours under another name stays.
    #[test]
    fn a_different_app_under_a_matching_id_is_kept() {
        let installed = vec![entry("notes", "Sticky Notes"), entry("yantrik-notes", "Notes")];
        assert_eq!(one_per_app(&installed).len(), 2);
    }
}

#[cfg(test)]
mod icon_id_tests {
    use super::icon_id_for;

    /// Every app this OS ships resolves to an id the icon set actually knows.
    ///
    /// The list on the right is `Icons.app` in crates/yantrik-ui-kit/slint/icon.slint. Without
    /// this mapping ten of the sixteen fell through to their category glyph, so Mail,
    /// Downloads and Network Manager all drew the same picture — which is what the launcher
    /// was photographed doing.
    #[test]
    fn every_shipped_app_maps_to_an_icon_the_set_knows() {
        const KNOWN: &[&str] = &[
            "terminal", "browser", "files", "editor", "email", "notes", "system", "network",
            "packages", "memory", "media", "music", "weather", "settings", "calendar", "bond",
            "notifications", "spreadsheet", "documents", "presentation", "launchpad", "yantrik",
            "about", "containers", "devices", "downloads", "permissions", "personality",
            "skills", "snippets", "sysmonitor", "image", "studio",
        ];
        const SHIPPED: &[&str] = &[
            "yantrik-calendar", "yantrik-container-manager", "yantrik-document-editor",
            "yantrik-download-manager", "yantrik-email", "yantrik-image-viewer",
            "yantrik-music-player", "yantrik-network-manager", "yantrik-notes",
            "yantrik-presentation", "yantrik-snippet-manager", "yantrik-spreadsheet",
            "yantrik-studio", "yantrik-system-monitor", "yantrik-terminal",
            "yantrik-text-editor", "yantrik-weather",
        ];

        let missing: Vec<String> = SHIPPED
            .iter()
            .map(|app| (app, icon_id_for(app)))
            .filter(|(_, id)| !KNOWN.contains(&id.as_str()))
            .map(|(app, id)| format!("{app} -> {id}"))
            .collect();

        assert!(
            missing.is_empty(),
            "these apps resolve to an icon id the set does not have, so they will draw their \
             category glyph instead of their own icon:\n  {}",
            missing.join("\n  ")
        );
    }

    /// A foreign app keeps its own id; this table is only for ours.
    #[test]
    fn a_third_party_app_is_left_alone() {
        assert_eq!(icon_id_for("chromium"), "chromium");
        assert_eq!(icon_id_for("org.gnome.Nautilus"), "org.gnome.Nautilus");
    }
}

#[cfg(test)]
mod app_colour_tests {
    use super::icon_id_for;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// The one table that gives an app its colour, in the UI kit.
    fn app_color_slint() -> String {
        let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yantrik-ui-kit/slint/app_color.slint");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// The body of one `public pure function <name>` in that file.
    fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
        let start = src
            .find(&format!("public pure function {name}("))
            .unwrap_or_else(|| panic!("app_color.slint has no function {name}"));
        let rest = &src[start..];
        let end = rest.find("\n    }").expect("a function body ends at its closing brace");
        &rest[..end]
    }

    /// `(key, value)` for every `<var> == "key" ? "value"` or `<var> == "key" ? root.x` arm.
    fn arms(body: &str, var: &str) -> Vec<(String, String)> {
        let needle = format!("{var} == \"");
        body.lines()
            .filter_map(|line| {
                let after = &line[line.find(&needle)? + needle.len()..];
                let (key, rest) = after.split_once('"')?;
                let value = rest.split_once('?')?.1.trim();
                let value = value.split("//").next()?.trim().trim_matches('"').to_string();
                Some((key.to_string(), value))
            })
            .collect()
    }

    /// Every app id the launcher, the desktop's workspace row and the taskbar can draw a tile
    /// for: the shell's built-in apps, every app we ship a .desktop entry for (under the id the
    /// icon set and the tiles are keyed by), and every app the taskbar can name a window after.
    fn ids_the_launcher_knows() -> BTreeSet<String> {
        let mut ids: BTreeSet<String> = yantrik_shell_core::apps::builtin_apps()
            .into_iter()
            .map(|entry| icon_id_for(&entry.app_id))
            .collect();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop-files");
        for entry in std::fs::read_dir(&dir).expect("apps/desktop-files is in the tree") {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".desktop") {
                ids.insert(icon_id_for(stem));
            }
        }
        ids.extend(crate::windows::APP_NAMES.iter().map(|(id, _)| id.to_string()));
        ids
    }

    /// Every app a tile can be drawn for has a colour of its own.
    ///
    /// An app missing from `AppColor.hue-for-app` does not fail to draw: it falls back to a
    /// quiet grey tile, which is right for a third-party app and wrong for one of ours — it
    /// is how Agents, Arcade and Weather came to wear the house accent in the launcher while
    /// every app beside them had a colour. So the table is held to the launcher's own list.
    #[test]
    fn every_app_the_launcher_knows_has_a_colour() {
        let src = app_color_slint();
        let table: BTreeSet<String> = arms(function_body(&src, "hue-for-app"), "id")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let missing: Vec<String> = ids_the_launcher_knows()
            .into_iter()
            .filter(|id| !table.contains(id))
            .collect();
        assert!(
            missing.is_empty(),
            "these apps have no colour in AppColor.hue-for-app \
             (crates/yantrik-ui-kit/slint/app_color.slint), so their tile falls back to grey:\n  {}",
            missing.join("\n  ")
        );
    }

    /// A hue the table names is one the palette has, as a glyph tone AND as a tile.
    ///
    /// A misspelt hue ("amer") would not fail to compile — `hue()` falls through to the accent
    /// and `tile-hue()` to a grey tile — so this is the only thing that catches it.
    #[test]
    fn every_hue_in_the_table_is_in_the_palette() {
        let src = app_color_slint();
        let glyph: BTreeSet<String> =
            arms(function_body(&src, "hue"), "name").into_iter().map(|(h, _)| h).collect();
        let tile: BTreeSet<String> =
            arms(function_body(&src, "tile-hue"), "name").into_iter().map(|(h, _)| h).collect();
        for (id, hue) in arms(function_body(&src, "hue-for-app"), "id") {
            assert!(glyph.contains(&hue), "{id} is {hue:?}, which hue() does not know");
            assert!(tile.contains(&hue), "{id} is {hue:?}, which tile-hue() does not know");
        }
        assert_eq!(glyph, tile, "the glyph tones and the tile fills name the same hues");
    }

    /// The colours the design names are the ones the table gives (desk-and-mind, "Colour per
    /// app"): Files blue, Calendar red, Notes amber, Terminal green, Mail blue, Browser teal,
    /// Studio violet. Files is the folder blue ("sky") so it and Mail, alphabetical
    /// neighbours in the launcher as Email and Files, are two blues and not one.
    #[test]
    fn the_design_colours_hold() {
        let src = app_color_slint();
        let table = arms(function_body(&src, "hue-for-app"), "id");
        let hue = |id: &str| {
            table
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, h)| h.as_str())
                .unwrap_or_else(|| panic!("{id} has no colour"))
        };
        assert_eq!(hue("files"), "sky");
        assert_eq!(hue("email"), "blue");
        assert_eq!(hue("calendar"), "red");
        assert_eq!(hue("notes"), "amber");
        assert_eq!(hue("terminal"), "green");
        assert_eq!(hue("browser"), "teal");
        assert_eq!(hue("studio"), "violet");
    }

    /// Teal is a mind's colour, so an app tile wears it only where the design names it: Browser.
    /// Containers, Downloads and Packages were teal and read as three minds in the launcher.
    #[test]
    fn teal_is_the_browser_alone() {
        let src = app_color_slint();
        let teal: Vec<String> = arms(function_body(&src, "hue-for-app"), "id")
            .into_iter()
            .filter(|(_, h)| h == "teal")
            .map(|(id, _)| id)
            .collect();
        assert_eq!(teal, ["browser"], "only Browser is a teal tile");
        let table = arms(function_body(&src, "hue-for-app"), "id");
        let hue = |id: &str| table.iter().find(|(k, _)| k == id).map(|(_, h)| h.clone());
        assert_eq!(hue("containers").as_deref(), Some("blue"));
        assert_eq!(hue("downloads").as_deref(), Some("sky"));
        assert_eq!(hue("packages").as_deref(), Some("green"));
    }
}

/// One tile per app.
///
/// The scan already keeps one entry per desktop id, but the same program can arrive under two:
/// the distribution's `blender.desktop` and our `yantrik-blender.desktop`, which starts the same
/// Blender with the addon that gives it a surface. "Blender" was in All apps twice. Our entry
/// shadows the bare id it wraps — same id once `yantrik-` is taken off, same name — and the
/// distribution's is left out of the grid. The launch dispatch routes either id to the same
/// place, so nothing that opened before stops opening.
fn one_per_app(installed: &[DesktopEntry]) -> Vec<&DesktopEntry> {
    let ours: Vec<(&str, String)> = installed
        .iter()
        .filter_map(|e| Some((e.app_id.strip_prefix("yantrik-")?, e.name.to_lowercase())))
        .collect();
    installed
        .iter()
        .filter(|e| !ours.iter().any(|(bare, name)| *bare == e.app_id && *name == e.name.to_lowercase()))
        .collect()
}

fn populate_grid(ui: &App, installed: &Arc<Vec<DesktopEntry>>, query: &str) {
    let apps: Vec<AppGridItem> = one_per_app(installed)
        .into_iter()
        .filter(|entry| {
            super::launcher::matches(
                query,
                &[&entry.name, &entry.app_id, &entry.categories, &entry.comment],
            )
        })
        .map(|entry| {
            let icon = icons::resolve(&entry.icon);
            AppGridItem {
                app_id: entry.app_id.clone().into(),
                name: entry.name.clone().into(),
                icon_char: entry.icon_char.clone().into(),
                icon_id: icon_id_for(&entry.app_id).into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
                category: SharedString::from(icons::category_id(&entry.categories)),
                pinned: super::pins::is_pinned(&entry.app_id),
                pinnable: super::pins::is_pinnable(&entry.app_id),
            }
        })
        .collect();
    ui.set_grid_apps(ModelRc::new(VecModel::from(apps)));
}
