//! AI Provider wiring — V61 runtime switching.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

/// Wire V61 AI provider management callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // V61 — Quick switch provider (from status bar popup)
    ui.on_ai_quick_switch_provider(move |provider_id| {
        tracing::info!(provider = %provider_id, "Quick-switching AI provider");
        // In production: update the active provider in the runtime
    });
}
