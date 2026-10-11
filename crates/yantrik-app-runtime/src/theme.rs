//! Theme settings shared with the shell.
//!
//! The shell persists the user's choices in `~/.config/yantrik/settings.yaml` and applies them
//! to the `ThemeMode` / `AccentPreset` globals at startup. A standalone app is a separate process
//! with its own copy of those globals, so unless it reads the same file it opens in the default
//! soft-blue-on-dark regardless of what the user picked. Reading the file at launch is enough: the
//! shell relaunches nothing on a theme change, and an app that was open keeps its look until it
//! is next started, which is how every other desktop behaves.

use std::path::PathBuf;

/// The two theme choices the shell exposes. Fields mirror the shell's `UserSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeSettings {
    pub dark: bool,
    /// Index into `AccentPreset`: 0 soft blue, 2 violet, 4 pink (see `ACCENTS`).
    pub accent_index: i32,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self { dark: true, accent_index: 0 }
    }
}

/// The accents Settings offers, as (saved name, `AccentPreset.index`). The one list: the shell's
/// picker, its settings file and every app's launch all read it from here.
///
/// The indices are not 0, 1, 2 because amber (1) and green (3) were taken out (colour roles,
/// 4 Oct 2026): amber is "needs you" and green is success, so neither may be the colour of every
/// primary. The survivors keep their numbers, so an index already handed to a window, or written
/// by a build that had five, still means the same colour; 1 and 3 are never reused.
///
/// The saved name for the soft blue stays "cyan": settings files and the control surface already
/// say it.
pub const ACCENTS: [(&str, i32); 3] = [("cyan", 0), ("purple", 2), ("pink", 4)];

/// The accent every unknown or retired choice lands on: the soft blue.
pub const DEFAULT_ACCENT: &str = "cyan";

/// The saved name as Settings offers it, or the soft blue for one it no longer offers (a file
/// written when amber and green were choices) or never did.
pub fn offered_accent(name: &str) -> &'static str {
    ACCENTS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
        .map_or(DEFAULT_ACCENT, |(n, _)| n)
}

pub fn accent_name_to_index(name: &str) -> i32 {
    let name = offered_accent(name);
    ACCENTS.iter().find(|(n, _)| *n == name).map_or(0, |(_, i)| *i)
}

/// Path of the shell's settings file — the one the machine's ceiling is read from, so the theme
/// and the ceiling can never be reading two different files.
pub fn settings_path() -> PathBuf {
    yantrik_ipc_transport::gate::settings_path()
}

/// Read the theme settings, falling back to the defaults for anything missing or unreadable.
///
/// Only the two keys this module cares about are parsed; the rest of the file is the shell's
/// business and may change shape without breaking apps.
pub fn load() -> ThemeSettings {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return ThemeSettings::default();
    };
    parse(&text)
}

fn parse(text: &str) -> ThemeSettings {
    let mut t = ThemeSettings::default();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key.trim() {
            "dark_mode" => t.dark = value != "false",
            "accent_color" => t.accent_index = accent_name_to_index(value),
            _ => {}
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_two_keys_and_ignores_the_rest() {
        let t = parse("user_name: Pranab\ndark_mode: false\naccent_color: purple\nwallpaper: aurora\n");
        assert_eq!(t, ThemeSettings { dark: false, accent_index: 2 });
    }

    #[test]
    fn unknown_accent_falls_back_to_cyan() {
        assert_eq!(accent_name_to_index("teal"), 0);
        assert_eq!(accent_name_to_index("Pink"), 4);
    }

    /// Amber and green were presets once. A settings file that still says either opens on the
    /// soft blue, in the shell and in every app, and the survivors keep their old numbers.
    #[test]
    fn a_retired_accent_opens_on_the_soft_blue() {
        for retired in ["amber", "green", "Amber"] {
            assert_eq!(offered_accent(retired), "cyan", "{retired}");
            assert_eq!(accent_name_to_index(retired), 0, "{retired}");
        }
        assert_eq!(parse("accent_color: amber").accent_index, 0);
        assert_eq!(parse("accent_color: green").accent_index, 0);
        assert_eq!(accent_name_to_index("purple"), 2, "violet keeps index 2");
        assert_eq!(accent_name_to_index("pink"), 4, "pink keeps index 4");
        assert_eq!(offered_accent(" Purple "), "purple");
    }

    #[test]
    fn missing_file_means_defaults() {
        assert_eq!(parse(""), ThemeSettings::default());
    }
}
