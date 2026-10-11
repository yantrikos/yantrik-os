//! Themes on the shell's control surface: what `describe shell` says under `theme`, the
//! `set_theme` action that Settings' theme cards also reach (`wire::theme::choose`), and
//! `set_dark_style`, the Quick Settings tile's and Settings' Dark/Light choice.
//!
//! `sensitive`: a theme changes how the whole desktop looks and rewrites files outside the shell
//! (labwc's theme, the terminal's colours, GTK's scheme). The person can put it back with a click,
//! but a mind that repaints their desktop unasked is exactly the kind of thing their grade is
//! for.
//!
//! The answer is a reading, not the request: the theme the shell now has, from the same property
//! the Settings card reads. The files on the machine are written on a worker after the answer
//! (labwc is asked to reload there, with a timeout), and the answer says so.
//!
//! `set_dark_style` is `sensitive`: it flips the appearance the tile flips, through the same
//! callback (`toggle-dark-mode`, wired in `wire::settings`), which saves `dark_mode`, so it is
//! still in force after a restart. That is the #48 rule (docs/app-control.md) `set_theme` and
//! `set_do_not_disturb` are graded by. `describe shell` reports the mode under
//! `settings.dark`. Named for the tile ("Dark style"), not `set_dark_mode`: an action whose name
//! says "mode" reads as a change to what a mind may do, and control_approvals' guard holds that
//! name for `set_mind_mode` alone.

use serde_json::{json, Value};
use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::wire::theme;
use crate::App;

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let weak = ui.as_weak();
    let dark_weak = ui.as_weak();
    surface.action(
        Action::new(
            "set_theme",
            "Choose the desktop's theme, a place that sets the colours, the accent and the wallpaper together and also \
             restyles the window frames, the Alt+Tab list, new terminals and the lock screen. `theme` is an id from \
             `describe shell` under `theme.themes` (lake, nightfall). Answers with the theme now in use; the files outside the \
             shell are written just after.",
        )
        .risk("sensitive")
        .arg(Param::text("theme").describe("The theme's id, e.g. lake or nightfall")),
        move |args| {
            let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
            let id = args["theme"].as_str().unwrap_or_default().trim().to_lowercase();
            if id.is_empty() {
                return Err("`theme` is empty".into());
            }
            let chosen = theme::choose(&ui, &id)?;
            Ok(answer(&ui, chosen))
        },
    )
    .action(
        Action::new(
            "set_dark_style",
            "Switch the desktop between its dark and light appearance, as the Dark style tile in Quick Settings and \
             Settings → Appearance do. The theme stays; its own palette shows in dark and gives way to the stock light \
             colours. Answers with the mode now in use, which `describe shell` reports under `settings.dark`.",
        )
        .risk("sensitive")
        .arg(Param::flag("dark").describe("true for dark, false for light")),
        move |args| {
            let ui = dark_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
            let dark = args["dark"].as_bool().ok_or("`dark` must be true or false")?;
            let was = ui.get_settings_dark_mode();
            // The tile's own callback, so the flag, the palette and the saved setting move
            // together exactly as a click moves them. It toggles, so it is pressed only when the
            // mode is not already the one asked for.
            if was != dark {
                ui.invoke_toggle_dark_mode();
            }
            let now = ui.get_settings_dark_mode();
            Ok(json!({ "dark": now, "changed": was != now }))
        },
    )
}

/// What was done, read back from the shell: the id now on the Settings card, not the one asked for.
fn answer(ui: &App, chosen: &theme::Theme) -> Value {
    json!({
        "theme": ui.get_settings_theme().to_string(),
        "name": chosen.name,
        "dark": ui.get_settings_dark_mode(),
        "accent": ui.get_settings_accent_color().to_string(),
        "wallpaper": ui.get_wallpaper_path().to_string(),
        "writing": "window frames, terminal colours and the GTK scheme are written just after this answer",
    })
}

#[cfg(test)]
mod tests {
    const SOURCE: &str = include_str!("control_theme.rs");

    /// The grade is the point of this file's header: a mind repainting the desktop is for the
    /// person's grade to stop.
    #[test]
    fn choosing_a_theme_is_sensitive() {
        let action = &SOURCE[SOURCE.find("Action::new(\n            \"set_theme\"").expect("set_theme is declared")..];
        let action = &action[..action.find("move |args|").unwrap()];
        assert!(action.contains(".risk(\"sensitive\")"), "set_theme must be sensitive");
    }

    /// The handler goes through the one function Settings uses, so a pointer and a mind cannot
    /// end in different places, and it never runs a process itself.
    #[test]
    fn the_action_is_the_settings_path_and_runs_nothing_itself() {
        let body = &SOURCE[SOURCE.find("move |args|").unwrap()..SOURCE.find("/// What was done").unwrap()];
        assert!(body.contains("theme::choose("));
        for blocking in ["Command::new", "std::fs::", "sleep("] {
            assert!(!body.contains(blocking), "{blocking} on the UI thread");
        }
    }

    /// Dark mode is a setting a restart reads back (sensitive, by the #48 rule), set through the
    /// tile's own callback and nothing beside it.
    #[test]
    fn dark_mode_is_sensitive_and_goes_through_the_tiles_callback() {
        let action = &SOURCE[SOURCE.find("\"set_dark_style\",").expect("set_dark_style is declared")..];
        let action = &action[..action.find("/// What was done").unwrap()];
        assert!(action.contains(".risk(\"sensitive\")"), "set_dark_style saves a setting, so it is sensitive");
        assert!(action.contains("ui.invoke_toggle_dark_mode()"), "the same path the tile and Settings take");
        assert!(!action.contains("set_dark("), "no second path beside the tile's");
        for blocking in ["Command::new", "std::fs::", "sleep("] {
            assert!(!action.contains(blocking), "{blocking} on the UI thread");
        }
    }

    /// Describe lists exactly the themes the action accepts.
    #[test]
    fn describe_lists_the_themes_the_action_accepts() {
        let say = crate::wire::theme::for_describe("lake");
        for t in say["themes"].as_array().unwrap() {
            assert!(crate::wire::theme::find(t["id"].as_str().unwrap()).is_some());
        }
    }
}
