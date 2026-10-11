//! The shell's theme, carried into the windows it opens beside its own.
//!
//! A window has its own copy of every global, so a window the shell opens — an agent popped out
//! (wire/agents.rs), the Lens (wire/lens_window.rs) — takes the shell's theme, dark or light, the
//! accent, any community theme, from the shell, and keeps taking it.

/// Copy one global's property from the shell's window into `$window`, only when it differs.
macro_rules! copy_global {
    ($ui:expr, $window:expr, $global:ty, $get:ident, $set:ident) => {{
        let want = $ui.global::<$global>().$get();
        if $window.global::<$global>().$get() != want {
            $window.global::<$global>().$set(want);
        }
    }};
}
pub(crate) use copy_global;

/// Copy the shell's theme globals into `$window`'s, writing only what differs.
///
/// A macro rather than a function because every window is a type of its own and
/// `global::<T>()` is generic over the window, so one function would need a bound per global.
macro_rules! sync_theme {
    ($ui:expr, $window:expr) => {{
        use slint::ComponentHandle as _;
        use $crate::wire::theme_sync::copy_global;
        copy_global!($ui, $window, $crate::ThemeMode, get_dark, set_dark);
        copy_global!($ui, $window, $crate::AccentPreset, get_index, set_index);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_enabled, set_enabled);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_bg_deep_override, set_bg_deep_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_bg_surface_override, set_bg_surface_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_bg_card_override, set_bg_card_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_bg_elevated_override, set_bg_elevated_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_amber_override, set_amber_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_cyan_override, set_cyan_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_text_primary_override, set_text_primary_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_text_secondary_override, set_text_secondary_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_text_dim_override, set_text_dim_override);
        copy_global!($ui, $window, $crate::ThemeOverrides, get_accent_override, set_accent_override);
    }};
}
pub(crate) use sync_theme;
