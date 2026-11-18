//! A window as the framework drives it: made, themed and given its chrome by the runner, not by
//! each app's `main` (design/app-framework-2026-10-21.md, §2.3).
//!
//! Slint generates a window's component and its globals (`ThemeMode`, `AccentPreset`,
//! `WindowChrome`) into the app's own crate, so this crate cannot name them. The app names them
//! once, with [`window!`], which implements [`Window`] at the app's site:
//!
//! ```ignore
//! slint::include_modules!();
//! yantrik_app::window!(WeatherApp);
//! ```
//!
//! That is the adapter the design holds provisional until it has compiled in two separate
//! generated UI crates in release mode; Weather and Notes are those two.

use crate::theme::ThemeSettings;

/// What the runner needs of a window. Implemented by [`window!`].
pub trait Window: slint::ComponentHandle + Sized + 'static {
    /// The window, made. The backend has been chosen before this runs.
    fn make() -> Result<Self, slint::PlatformError>;
    /// The current theme, applied to the window's generated globals.
    fn apply_theme(&self, theme: &ThemeSettings);
    /// The title bar's drag, minimize, maximize and close, wired to the compositor.
    fn connect_chrome(&self);
}

/// Implements [`Window`] for a generated root component. The root `.slint` must export the
/// kit's `ThemeMode`, `AccentPreset` and `WindowChrome` globals, and `slint::include_modules!()`
/// must be in scope where this is written.
#[macro_export]
macro_rules! window {
    ($ui:ident) => {
        impl $crate::window::Window for $ui {
            fn make() -> Result<Self, $crate::slint::PlatformError> {
                $ui::new()
            }
            fn apply_theme(&self, theme: &$crate::theme::ThemeSettings) {
                use $crate::slint::ComponentHandle as _;
                self.global::<ThemeMode>().set_dark(theme.dark);
                self.global::<AccentPreset>().set_index(theme.accent_index);
            }
            fn connect_chrome(&self) {
                $crate::window_chrome!(self);
            }
        }
    };
}
