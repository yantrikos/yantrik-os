//! Themes are places: one choice that sets the palette, the accent, the wallpaper and the dark
//! flag together, and reaches the parts of the desktop the shell does not draw.
//!
//! A theme is a small file in `crates/yantrik-design-tokens/themes/` (embedded here). Choosing one:
//!
//!   * on the UI thread, sets the tokens' `ThemeOverrides`, the dark flag, the accent and the
//!     wallpaper, and saves them as the person's settings;
//!   * on a worker, writes what other programs read, then asks the compositor to look again:
//!       - labwc's `themerc` (title bars, menus and the Alt+Tab list, so the window frames match),
//!       - foot's colours (a terminal opened afterwards; one already open keeps its colours),
//!       - the GTK colour scheme (apps that follow it, and the settings files GTK 3 and 4 read),
//!       - and the lock screen's blurred wallpaper (`lock_wallpaper.rs`).
//!
//! Nothing here runs a process on the UI thread: `labwc --reconfigure` and `gsettings` are on the
//! worker, each with a timeout of its own, and a missing tool is a log line, not an error the
//! person is shown for a program their machine never had.
//!
//! The person's own files are edited, not replaced: foot's `[colors]` section and the one GTK
//! key are swapped inside whatever else the file says, and a block this wrote is recognised by
//! its marker lines and replaced in place on the next choice.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use slint::ComponentHandle;

use crate::{AccentPreset, App, ThemeCardData, ThemeMode, ThemeOverrides};

// The file writers live beside this; re-exported so callers and the tests keep one path.
pub use super::theme_files::*;

/// The theme a new install starts on and a reset returns to.
pub const DEFAULT: &str = "lake";

/// The shipped themes, in the order Settings shows them.
const FILES: &[&str] = &[
    include_str!("../../../yantrik-design-tokens/themes/lake.toml"),
    include_str!("../../../yantrik-design-tokens/themes/nightfall.toml"),
];

#[derive(Debug, Clone, Deserialize)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub description: String,
    pub dark: bool,
    /// An accent Settings offers: `cyan` (the soft blue), `purple` (violet) or `pink`.
    pub accent: String,
    /// A wallpaper preset id.
    pub wallpaper: String,
    /// Whether the palette replaces the tokens' own. Lake's palette IS the tokens' own.
    pub overrides: bool,
    pub palette: Palette,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Palette {
    pub bg_deep: String,
    pub bg_surface: String,
    pub bg_card: String,
    pub bg_elevated: String,
    pub amber: String,
    pub cyan: String,
    pub text_primary: String,
    pub text_secondary: String,
    pub text_dim: String,
    pub accent: String,
}

/// Every shipped theme. A file that does not parse is a build-time mistake and a test fails on
/// it; at run time it is skipped, so one bad theme cannot take the Settings page with it.
pub fn all() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        FILES
            .iter()
            .filter_map(|text| match toml::from_str::<Theme>(text) {
                Ok(theme) => Some(theme),
                Err(e) => {
                    tracing::error!(error = %e, "A shipped theme does not parse; it is left out");
                    None
                }
            })
            .collect()
    })
}

pub fn find(id: &str) -> Option<&'static Theme> {
    all().iter().find(|t| t.id == id)
}

/// The theme named, or the default when the name is unknown (a settings file from a newer or
/// older build).
pub fn find_or_default(id: &str) -> Option<&'static Theme> {
    find(id).or_else(|| find(DEFAULT))
}

/// Run `program args` and wait at most `limit`; the child is killed past it. `Err` says why not.
fn run_for(program: &str, args: &[&str], limit: Duration) -> Result<(), String> {
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("{program} exited {status}")),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} did not answer in {limit:?}"));
            }
            Err(e) => return Err(format!("{program}: {e}")),
        }
    }
}

/// Everything outside the shell's own window, for a theme. Blocking (files, then two programs):
/// call it from a worker only.
pub fn apply_to_machine(t: &Theme) {
    match write_files(t, &Dirs::from_env()) {
        Ok(written) => {
            // Only when the file changed: a start that finds its theme already written must not
            // make the compositor redraw every window for nothing.
            if written.themerc_changed {
                if let Err(e) = run_for("labwc", &["--reconfigure"], Duration::from_secs(3)) {
                    tracing::info!(error = %e, "labwc was not told to reload its theme; window frames change at the next start");
                }
            }
        }
        Err(e) => tracing::warn!(theme = %t.id, error = %e, "Not every theme file could be written"),
    }
    if let Err(e) = run_for("gsettings", &["set", "org.gnome.desktop.interface", "color-scheme", gtk_scheme(t)], Duration::from_secs(3)) {
        tracing::info!(error = %e, "gsettings did not take the colour scheme; GTK apps follow settings.ini");
    }
}

/// Put the theme's palette on the tokens: its colours as `ThemeOverrides` (or none, for a theme
/// whose palette is the tokens' own). UI thread; no I/O.
fn apply_palette(ui: &App, t: &Theme) {
    let overrides = ui.global::<ThemeOverrides>();
    // A palette made for the dark ground is not laid over the light one: the overrides are mode
    // blind, and near-white text on a pale card is unreadable. Light mode shows the stock light.
    overrides.set_enabled(t.overrides && ui.global::<ThemeMode>().get_dark());
    if !t.overrides {
        return;
    }
    let colour = |hex: &str| rgb(hex).map(|[r, g, b]| slint::Color::from_rgb_u8(r, g, b));
    let p = &t.palette;
    let set = |value: &str, setter: &dyn Fn(slint::Color)| {
        if let Some(c) = colour(value) {
            setter(c);
        }
    };
    set(&p.bg_deep, &|c| overrides.set_bg_deep_override(c));
    set(&p.bg_surface, &|c| overrides.set_bg_surface_override(c));
    set(&p.bg_card, &|c| overrides.set_bg_card_override(c));
    set(&p.bg_elevated, &|c| overrides.set_bg_elevated_override(c));
    set(&p.amber, &|c| overrides.set_amber_override(c));
    set(&p.cyan, &|c| overrides.set_cyan_override(c));
    set(&p.text_primary, &|c| overrides.set_text_primary_override(c));
    set(&p.text_secondary, &|c| overrides.set_text_secondary_override(c));
    set(&p.text_dim, &|c| overrides.set_text_dim_override(c));
    set(&p.accent, &|c| overrides.set_accent_override(c));
}

/// Set the shell's own tokens and flags for `t`: palette, dark flag, accent, wallpaper. UI thread.
pub fn apply_to_ui(ui: &App, t: &Theme) {
    ui.global::<ThemeMode>().set_dark(t.dark);
    ui.set_settings_dark_mode(t.dark);
    ui.set_settings_accent_color(t.accent.clone().into());
    ui.global::<AccentPreset>().set_index(crate::wire::settings::accent_name_to_index(&t.accent));
    ui.set_settings_theme(t.id.clone().into());
    apply_palette(ui, t);
    ui.set_wallpaper_path(t.wallpaper.clone().into());
}

/// The person switched between the dark and light appearance: a theme's own palette goes with the
/// dark one and gives way to the stock light colours, and comes back with it.
pub fn dark_mode_changed(ui: &App) {
    if let Some(theme) = find(ui.get_settings_theme().as_str()) {
        apply_palette(ui, theme);
    }
}

/// Choose a theme: the tokens now, the settings saved, the machine's files on a worker. Called
/// on the UI thread, by the Settings cards and by the control surface alike. The unknown id is
/// an error, not a quiet fallback.
pub fn choose(ui: &App, id: &str) -> Result<&'static Theme, String> {
    let theme = find(id).ok_or_else(|| {
        format!("There is no theme called \"{id}\". The themes are: {}.", all().iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", "))
    })?;
    apply_to_ui(ui, theme);
    if let Err(e) = crate::wire::settings::record_theme(theme) {
        tracing::warn!(error = %e, "The theme was applied but could not be saved");
    }
    std::thread::spawn(move || apply_to_machine(theme));
    crate::lock_wallpaper::refresh(&theme.wallpaper);
    Ok(theme)
}

/// At start: the saved theme's palette onto the tokens, and, if a theme was ever chosen, its files
/// on the machine. The dark
/// flag, accent and wallpaper are the person's current settings, read before this runs; they may
/// have changed them since choosing the theme, and a restart must not undo that.
pub fn restore(ui: &App) {
    let id = crate::wire::settings::theme_id();
    let Some(theme) = find_or_default(&id) else { return };
    ui.set_settings_theme(theme.id.clone().into());
    apply_palette(ui, theme);
    // The machine's files only for a theme somebody chose: a machine that has never been told
    // keeps its window frames and its terminal's colours exactly as they are.
    if crate::wire::settings::theme_chosen() {
        std::thread::spawn(move || apply_to_machine(theme));
    }
}

/// The cards Settings shows, from the theme files: their own colours and their own wallpaper.
fn cards() -> Vec<ThemeCardData> {
    let colour = |hex: &str| {
        let [r, g, b] = rgb(hex).unwrap_or([0, 0, 0]);
        slint::Color::from_rgb_u8(r, g, b)
    };
    all()
        .iter()
        .map(|t| ThemeCardData {
            id: t.id.clone().into(),
            name: t.name.clone().into(),
            description: t.description.clone().into(),
            preview: crate::lock_wallpaper::preview_image(&t.wallpaper).unwrap_or_default(),
            ground: colour(&t.palette.bg_deep),
            surface: colour(&t.palette.bg_surface),
            text: colour(&t.palette.text_primary),
            accent: colour(&t.palette.accent),
        })
        .collect()
}

/// Settings' theme cards and what choosing one does.
pub fn wire(ui: &App) {
    ui.set_theme_cards(slint::ModelRc::new(slint::VecModel::from(cards())));
    ui.set_settings_theme(find_or_default(&crate::wire::settings::theme_id()).map_or_else(String::new, |t| t.id.clone()).into());
    let weak = ui.as_weak();
    ui.on_choose_theme(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        if let Err(e) = choose(&ui, id.as_str()) {
            tracing::warn!(error = %e, "Theme not applied");
        }
    });
}

/// What `describe shell` says about themes.
pub fn for_describe(current: &str) -> serde_json::Value {
    serde_json::json!({
        "current": current,
        "themes": all().iter().map(|t| serde_json::json!({
            "id": t.id, "name": t.name, "dark": t.dark, "accent": t.accent, "wallpaper": t.wallpaper,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lake() -> &'static Theme {
        find("lake").expect("Lake ships")
    }

    #[test]
    fn every_theme_keeps_the_minds_teal_and_the_needs_you_amber() {
        // Teal means a mind and amber means it needs you, on every theme (colour system): a theme
        // may change the grounds and the accent, never what those two say.
        for t in all() {
            assert_eq!(t.palette.cyan.to_lowercase(), lake().palette.cyan.to_lowercase(), "{} recolours the minds' teal", t.id);
            assert_eq!(t.palette.amber.to_lowercase(), lake().palette.amber.to_lowercase(), "{} recolours needs-you amber", t.id);
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-theme-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn every_shipped_theme_parses_and_names_things_that_exist() {
        assert_eq!(FILES.len(), all().len(), "a theme file that does not parse is left out at run time; here it fails");
        assert!(all().len() >= 2, "Lake and Nightfall");
        assert_eq!(all()[0].id, DEFAULT, "the default theme leads the cards");
        for t in all() {
            assert!(crate::wire::settings::WALLPAPER_PRESETS.contains(&t.wallpaper.as_str()), "{} names wallpaper {}", t.id, t.wallpaper);
            assert!(yantrik_app_runtime::theme::ACCENTS.iter().any(|(n, _)| *n == t.accent), "{} accent {} is not one Settings offers", t.id, t.accent);
            for c in [&t.palette.bg_deep, &t.palette.bg_surface, &t.palette.bg_card, &t.palette.bg_elevated, &t.palette.amber, &t.palette.cyan, &t.palette.text_primary, &t.palette.text_secondary, &t.palette.text_dim, &t.palette.accent] {
                assert!(rgb(c).is_some(), "{}: {c} is not #rrggbb", t.id);
            }
        }
        let ids: Vec<_> = all().iter().map(|t| &t.id).collect();
        assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len(), "ids are unique");
    }

    /// Lake turns the overrides off and says its palette is the tokens' own. If theme.slint's
    /// stock colours move and this file does not, the Settings card, the window frames and the
    /// terminal would show a colour the desktop no longer has.
    #[test]
    fn lake_is_the_stock_palette_of_the_tokens() {
        let slint = include_str!("../../../yantrik-design-tokens/slint/theme.slint");
        let block = &slint[slint.find("export global ThemeOverrides").unwrap()..];
        let block = &block[..block.find('}').unwrap()];
        let default_of = |name: &str| {
            let line = block.lines().find(|l| l.contains(&format!("{name}-override"))).unwrap_or_else(|| panic!("no {name}-override"));
            line.split(':').nth(1).unwrap().trim().trim_end_matches(';').trim().to_string()
        };
        let p = &lake().palette;
        for (name, value) in [
            ("bg-deep", &p.bg_deep), ("bg-surface", &p.bg_surface), ("bg-card", &p.bg_card), ("bg-elevated", &p.bg_elevated),
            ("amber", &p.amber), ("cyan", &p.cyan), ("text-primary", &p.text_primary), ("text-secondary", &p.text_secondary),
            ("text-dim", &p.text_dim), ("accent", &p.accent),
        ] {
            assert_eq!(&default_of(name), value, "Lake's {name}");
        }
        assert!(!lake().overrides, "stock colours are not overrides");
        assert!(lake().dark && lake().accent == "cyan", "charcoal and soft blue");
        assert!(find("nightfall").is_some_and(|t| t.overrides), "a theme with its own palette overrides");
    }

    #[test]
    fn the_themerc_changes_only_colours_and_every_colour_arrives() {
        let night = find("nightfall").unwrap();
        let rc = themerc(night);
        for (key, value) in frame_colours(night) {
            assert!(rc.lines().any(|l| l == format!("{key}: {value}")), "{key} should be {value}");
        }
        // Everything that is not a colour of the theme is the shipped file, line for line.
        let coloured: Vec<_> = frame_colours(night).into_iter().map(|(k, _)| k).collect();
        for line in BASE_THEMERC.lines() {
            let key = line.split_once(':').map(|(k, _)| k.trim());
            if !key.is_some_and(|k| coloured.contains(&k)) {
                assert!(rc.lines().any(|l| l == line), "lost a line of the shipped themerc: {line:?}");
            }
        }
        assert!(rc.contains("window.active.button.close.hover.bg.color: #e05263"), "the close button keeps its red under every theme");
        assert!(rc.contains("osd.window-switcher.item.active.border.width: 2"), "sizes are untouched");
        assert!(!rc.contains("#4ecdc4"), "no teal in a menu: teal marks minds");
        let again = themerc(night);
        assert_eq!(rc, again, "writing it twice writes the same file, so the second reconfigure never happens");
    }

    #[test]
    fn the_alt_tab_list_wears_the_theme() {
        for t in all() {
            let rc = themerc(t);
            assert!(rc.lines().any(|l| l == format!("osd.bg.color: {}", t.palette.bg_surface)), "{}", t.id);
            assert!(rc.lines().any(|l| l == format!("window.active.title.bg.color: {}", t.palette.bg_surface)), "{}", t.id);
        }
        assert_ne!(themerc(lake()), themerc(find("nightfall").unwrap()));
    }

    #[test]
    fn a_derived_edge_sits_between_its_surface_and_the_text() {
        assert_eq!(mix("#000000", "#ffffff", 0.0), "#000000");
        assert_eq!(mix("#000000", "#ffffff", 1.0), "#ffffff");
        assert_eq!(mix("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(mix("nonsense", "#ffffff", 0.0), "#000000", "a bad colour does not panic");
    }

    #[test]
    fn foot_keeps_the_persons_settings_and_swaps_only_the_colours() {
        let block = foot_block(lake());
        assert!(block.contains("background=0e1117") && block.contains("foreground=c0c8d6"));

        // The ISO's file: fonts, scrollback and a [colors] section in the middle.
        let iso = "[main]\nfont=DejaVu Sans Mono:size=11\n\n[colors]\nbackground=0c0b10\nforeground=c8c8d0\nregular0=1a1a2e\n\n[cursor]\nstyle=beam\n";
        let merged = merge_foot(iso, &block);
        assert!(merged.contains("font=DejaVu Sans Mono:size=11") && merged.contains("[cursor]\nstyle=beam"), "{merged}");
        assert!(!merged.contains("0c0b10") && !merged.contains("regular0=1a1a2e"), "the old colours are gone");
        assert_eq!(merged.matches("[colors]").count(), 1);

        // Choosing again replaces the block where it stands, wherever the person moved it.
        let night = foot_block(find("nightfall").unwrap());
        let again = merge_foot(&merged, &night);
        assert_eq!(again.matches(FOOT_BEGIN).count(), 1, "one block, not two");
        assert!(again.contains("background=0a0d17") && !again.contains("background=0e1117"));
        assert!(again.contains("font=DejaVu Sans Mono:size=11") && again.contains("[cursor]\nstyle=beam"));
        assert_eq!(merge_foot(&again, &night), again, "the same choice twice is the same file");

        // A file with no colours gets them at the end, after what was there; an empty one is just the block.
        let plain = merge_foot("[main]\nfont=monospace\n", &block);
        assert!(plain.starts_with("[main]\nfont=monospace\n\n") && plain.ends_with(&block), "{plain}");
        assert_eq!(merge_foot("", &block), block);
    }

    #[test]
    fn the_gtk_scheme_follows_the_dark_flag_and_the_ini_keeps_its_other_keys() {
        assert_eq!(gtk_scheme(lake()), "prefer-dark");
        let mut light = lake().clone();
        light.dark = false;
        assert_eq!(gtk_scheme(&light), "default");

        let ini = "[Settings]\ngtk-theme-name=Adwaita\ngtk-application-prefer-dark-theme=0\ngtk-font-name=Barlow 10\n\n[Other]\nx=1\n";
        let merged = merge_ini_key(ini, "gtk-application-prefer-dark-theme", "1");
        assert!(merged.contains("gtk-application-prefer-dark-theme=1") && !merged.contains("prefer-dark-theme=0"));
        assert!(merged.contains("gtk-theme-name=Adwaita") && merged.contains("gtk-font-name=Barlow 10") && merged.contains("[Other]\nx=1"));
        assert_eq!(merge_ini_key("", "k", "v"), "[Settings]\nk=v\n");
        assert_eq!(merge_ini_key("[Other]\nx=1\n", "k", "v"), "[Other]\nx=1\n\n[Settings]\nk=v\n");
        assert_eq!(merge_ini_key("[Settings]\nx=1\n", "k", "v"), "[Settings]\nk=v\nx=1\n");
        // A key of the same name in another section is not this one.
        assert_eq!(merge_ini_key("[Other]\nk=old\n[Settings]\nx=1\n", "k", "v"), "[Other]\nk=old\n[Settings]\nk=v\nx=1\n");
    }

    #[test]
    fn writing_a_theme_writes_the_frame_the_terminal_and_gtk_and_a_repeat_changes_nothing() {
        let root = scratch("write");
        let dirs = Dirs { config: root.join("config"), data: root.join("data") };
        std::fs::create_dir_all(dirs.config.join("foot")).unwrap();
        std::fs::write(dirs.config.join("foot/foot.ini"), "[main]\nfont=monospace\n").unwrap();

        let first = write_files(find("nightfall").unwrap(), &dirs).expect("written");
        assert_eq!(
            std::fs::read_to_string(dirs.config.join("foot/foot.ini.before-yantrik-theme")).unwrap(),
            "[main]\nfont=monospace\n",
            "the person's own foot.ini is kept the first time"
        );
        assert!(first.themerc_changed, "a new frame: the compositor is told");
        let rc = std::fs::read_to_string(dirs.themerc()).unwrap();
        assert!(rc.contains("window.active.title.bg.color: #111627"));
        let foot = std::fs::read_to_string(dirs.config.join("foot/foot.ini")).unwrap();
        assert!(foot.contains("font=monospace") && foot.contains("background=0a0d17"));
        for gtk in ["gtk-3.0", "gtk-4.0"] {
            let ini = std::fs::read_to_string(dirs.config.join(gtk).join("settings.ini")).unwrap();
            assert!(ini.contains("gtk-application-prefer-dark-theme=1"), "{gtk}");
        }

        let second = write_files(find("nightfall").unwrap(), &dirs).expect("written");
        assert!(!second.themerc_changed, "the same theme again: no reconfigure, no flicker");

        let back = write_files(lake(), &dirs).unwrap();
        assert_eq!(
            std::fs::read_to_string(dirs.config.join("foot/foot.ini.before-yantrik-theme")).unwrap(),
            "[main]\nfont=monospace\n",
            "and not replaced by a later copy of ours"
        );
        assert!(back.themerc_changed);
        assert!(std::fs::read_to_string(dirs.themerc()).unwrap().contains("window.active.title.bg.color: #161b23"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_cannot_be_written_does_not_stop_the_others() {
        let root = scratch("blocked");
        let dirs = Dirs { config: root.join("config"), data: root.join("data") };
        // foot's folder is a FILE: foot.ini cannot be created under it.
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("foot"), "in the way").unwrap();
        let result = write_files(lake(), &dirs);
        assert!(result.is_err(), "the failure is reported");
        assert!(dirs.themerc().exists(), "but the window frames were written");
        assert!(dirs.config.join("gtk-3.0/settings.ini").exists(), "and GTK's file");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_theme_is_an_error_that_names_the_choices() {
        assert!(find("no-such-place").is_none());
        assert_eq!(find_or_default("no-such-place").map(|t| t.id.as_str()), Some(DEFAULT), "an old settings file lands on the default");
        let say = for_describe("lake");
        assert_eq!(say["current"], "lake");
        assert_eq!(say["themes"][0]["id"], "lake");
        assert_eq!(say["themes"].as_array().unwrap().len(), all().len());
    }

    #[test]
    fn a_program_that_hangs_is_killed_and_one_that_is_missing_is_a_message() {
        let started = std::time::Instant::now();
        let hung = run_for("sleep", &["5"], Duration::from_millis(200));
        assert!(hung.unwrap_err().contains("did not answer"));
        assert!(started.elapsed() < Duration::from_secs(3), "it did not wait for the sleep");
        assert!(run_for("yantrik-no-such-program", &[], Duration::from_secs(1)).is_err());
        assert!(run_for("true", &[], Duration::from_secs(2)).is_ok());
    }

    /// The playbook's rule, as a test: no blocking call on the UI thread. The two functions the
    /// UI thread calls hold no process and no file I/O of their own; the work is in the one that
    /// a worker runs.
    #[test]
    fn the_ui_thread_functions_run_no_process() {
        let source = include_str!("theme.rs");
        let body = |name: &str| {
            let at = source.find(&format!("pub fn {name}(")).unwrap();
            let rest = &source[at..];
            let end = rest.find("\n}\n").unwrap();
            &rest[..end]
        };
        for name in ["apply_to_ui", "dark_mode_changed", "choose", "restore"] {
            let text = body(name);
            for blocking in ["Command::new", "run_for(", "write_files(", "std::fs::", "apply_to_machine(theme)"] {
                // The machine's part is only ever handed to a thread.
                if blocking == "apply_to_machine(theme)" {
                    assert!(!text.contains(blocking) || text.contains("std::thread::spawn(move || apply_to_machine(theme))"), "{name} must run it on a worker");
                } else {
                    assert!(!text.contains(blocking), "{name} must not call {blocking} on the UI thread");
                }
            }
        }
    }
}
