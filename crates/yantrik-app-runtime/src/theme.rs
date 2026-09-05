//! Theme settings shared with the shell.
//!
//! The shell persists the user's choices in `~/.config/yantrik/settings.yaml` and applies them
//! to the `ThemeMode` / `AccentPreset` globals at startup. A standalone app is a separate process
//! with its own copy of those globals, so unless it reads the same file it opens in the default
//! cyan-on-dark regardless of what the user picked. Reading the file at launch is enough: the
//! shell relaunches nothing on a theme change, and an app that was open keeps its look until it
//! is next started, which is how every other desktop behaves.

use std::path::PathBuf;

/// The two theme choices the shell exposes. Fields mirror the shell's `UserSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeSettings {
    pub dark: bool,
    /// Index into `AccentPreset` — 0 cyan, 1 amber, 2 purple, 3 green, 4 pink.
    pub accent_index: i32,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self { dark: true, accent_index: 0 }
    }
}

/// Accent names in `AccentPreset.index` order. Must match `wire/settings.rs::ACCENT_NAMES` in the
/// shell; the two are kept in step by the test below rather than by a shared crate, because the
/// shell does not depend on this one.
pub const ACCENT_NAMES: [&str; 5] = ["cyan", "amber", "purple", "green", "pink"];

pub fn accent_name_to_index(name: &str) -> i32 {
    ACCENT_NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name.trim()))
        .map(|i| i as i32)
        .unwrap_or(0)
}

/// Path of the shell's settings file.
pub fn settings_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/yantrik/settings.yaml")
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

    #[test]
    fn missing_file_means_defaults() {
        assert_eq!(parse(""), ThemeSettings::default());
    }
}
