//! A window has its own copy of every global, so a second window of the shell's — an agent popped
//! out (wire/agents.rs), the Lens (lens_window.rs) — takes the shell's theme from the shell: dark
//! or light, the accent, any community theme. One list, so the two windows cannot drift apart.

/// Copy the shell's theme globals from `$ui` (the `App`) into `$window`, writing only what
/// differs, so a caller on a timer repaints nothing when nothing changed.
macro_rules! sync_theme {
    ($ui:expr, $window:expr) => {{
        use slint::ComponentHandle as _;
        macro_rules! copy {
            ($global:ty, $get:ident, $set:ident) => {{
                let want = $ui.global::<$global>().$get();
                if $window.global::<$global>().$get() != want {
                    $window.global::<$global>().$set(want);
                }
            }};
        }
        copy!(crate::ThemeMode, get_dark, set_dark);
        copy!(crate::AccentPreset, get_index, set_index);
        copy!(crate::ThemeOverrides, get_enabled, set_enabled);
        copy!(crate::ThemeOverrides, get_bg_deep_override, set_bg_deep_override);
        copy!(crate::ThemeOverrides, get_bg_surface_override, set_bg_surface_override);
        copy!(crate::ThemeOverrides, get_bg_card_override, set_bg_card_override);
        copy!(crate::ThemeOverrides, get_bg_elevated_override, set_bg_elevated_override);
        copy!(crate::ThemeOverrides, get_amber_override, set_amber_override);
        copy!(crate::ThemeOverrides, get_cyan_override, set_cyan_override);
        copy!(crate::ThemeOverrides, get_text_primary_override, set_text_primary_override);
        copy!(crate::ThemeOverrides, get_text_secondary_override, set_text_secondary_override);
        copy!(crate::ThemeOverrides, get_text_dim_override, set_text_dim_override);
        copy!(crate::ThemeOverrides, get_accent_override, set_accent_override);
    }};
}
pub(crate) use sync_theme;
