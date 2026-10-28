//! AI Provider wiring — V61 runtime switching.

use slint::{ComponentHandle, Model};

use crate::app_context::AppContext;
use crate::App;

/// Wire V61 AI provider management callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // V61 — Quick switch provider (from status bar popup)
    ui.on_ai_quick_switch_provider(move |provider_id| {
        tracing::info!(provider = %provider_id, "Quick-switching AI provider");
        // In production: update the active provider in the runtime
    });

    // Push initial AI privacy mode to status bar
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // The status bar says "hybrid" while any account is saved, otherwise "local".
        let providers = ui.get_settings_ai_providers();
        let mode = if providers.row_count() > 0 { "hybrid" } else { "local" };
        ui.set_ai_privacy_mode(mode.into());
    });
}
