//! The `settings` section of `describe shell`: what Settings is set to, and, while the AI &
//! Intelligence page is showing, how that page is scrolled.
//!
//! `ai_page` carries three plain numbers in logical pixels and nothing the page says: how tall the
//! page's scrolling area is (`visible_height`), how tall its content is (`viewport_height`) and
//! where it is scrolled to (`viewport_y`, zero at the top and negative below it). A caller can tell
//! a page that cannot scroll (content no taller than the area) from one that does not move.

use crate::App;

/// Settings is screen 7 and AI & Intelligence is its category 1.
const SETTINGS_SCREEN: i32 = 7;
const AI_CATEGORY: i32 = 1;

/// The AI page's scroll geometry, only while Settings shows that page; the numbers are rounded to
/// whole pixels.
pub(crate) fn ai_page(screen: i32, category: i32, visible: f32, viewport: f32, y: f32) -> Option<serde_json::Value> {
    (screen == SETTINGS_SCREEN && category == AI_CATEGORY).then(|| {
        serde_json::json!({
            "visible_height": visible.round() as i64,
            "viewport_height": viewport.round() as i64,
            "viewport_y": y.round() as i64,
        })
    })
}

pub(crate) fn section(ui: &App, screen: i32) -> serde_json::Value {
    let mut section = serde_json::json!({
        "category": ui.get_settings_category(),
        "query": ui.get_settings_query().to_string(),
        "dark": ui.get_settings_dark_mode(),
        "accent": ui.get_settings_accent_color().to_string(),
        "wallpaper": ui.get_wallpaper_path().to_string(),
        "save_error": ui.get_settings_save_error(),
        "save_status": ui.get_settings_save_status().to_string(),
        "auto_lock_secs": ui.get_settings_auto_lock_secs(),
        "auto_lock_available": ui.get_settings_auto_lock_available(),
    });
    if let Some(page) = ai_page(
        screen,
        ui.get_settings_category(),
        ui.get_settings_ai_visible_height(),
        ui.get_settings_ai_viewport_height(),
        ui.get_settings_ai_viewport_y(),
    ) {
        section["ai_page"] = page;
    }
    section
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ai_page_geometry_appears_only_while_settings_shows_that_page() {
        let page = ai_page(7, 1, 714.4, 3273.6, -120.2).expect("Settings on AI & Intelligence reports it");
        assert_eq!(page["visible_height"], 714);
        assert_eq!(page["viewport_height"], 3274);
        assert_eq!(page["viewport_y"], -120);
        let mut keys: Vec<&str> = page.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["viewport_height", "viewport_y", "visible_height"], "plain numbers and nothing else");
        assert!(page.as_object().unwrap().values().all(serde_json::Value::is_number));
        assert!(ai_page(7, 0, 714.0, 900.0, 0.0).is_none(), "another Settings page");
        assert!(ai_page(1, 1, 714.0, 900.0, 0.0).is_none(), "Settings is not showing");
    }

    /// `section` is what `describe shell` publishes as `settings`, and it reads the page's numbers
    /// off the window.
    #[test]
    fn the_settings_section_carries_the_page_geometry_and_the_idle_lock_timeout() {
        let src = include_str!("control_settings.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for key in ["\"ai_page\"", "get_settings_ai_visible_height()", "get_settings_ai_viewport_height()", "get_settings_ai_viewport_y()"] {
            assert!(code.contains(key), "the settings section must read {key}");
        }
        assert!(
            code.contains("\"auto_lock_secs\": ui.get_settings_auto_lock_secs()"),
            "describe's `settings` must carry `auto_lock_secs` read off the window, the same property the idle watcher polls"
        );
        let control = include_str!("control.rs").split("#[cfg(test)]").next().unwrap();
        assert!(control.contains("crate::control_settings::section(&ui, screen)"), "describe shell publishes this section");
    }
}
