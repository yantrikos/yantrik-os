//! AI Provider wiring — V60 task routing, V60 privacy controls, V61 runtime switching, V62 cost tracking.

use slint::{ComponentHandle, Model};

use crate::app_context::AppContext;
use crate::App;

/// Wire V60-V62 AI provider management callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // V60 — Task routing: save fast/balanced/powerful provider preferences
    ui.on_save_task_routing(move |fast, balanced, powerful| {
        tracing::info!(
            fast = %fast,
            balanced = %balanced,
            powerful = %powerful,
            "Task routing saved"
        );
        // In production: persist to providers.yaml or settings.yaml
    });

    // V60 — Privacy preset selection
    let ui_weak = ui.as_weak();
    ui.on_set_privacy_preset(move |preset| {
        let preset_str = preset.to_string();
        tracing::info!(preset = %preset_str, "Privacy preset changed");

        let Some(ui) = ui_weak.upgrade() else { return };
        match preset_str.as_str() {
            "maximum" => {
                ui.set_settings_privacy_local_only(true);
                ui.set_settings_privacy_cloud_confirm(true);
            }
            "balanced" => {
                ui.set_settings_privacy_local_only(false);
                ui.set_settings_privacy_cloud_confirm(true);
            }
            "cloud-first" => {
                ui.set_settings_privacy_local_only(false);
                ui.set_settings_privacy_cloud_confirm(false);
            }
            _ => {}
        }
    });

    // V61 — Quick switch provider (from status bar popup)
    ui.on_ai_quick_switch_provider(move |provider_id| {
        tracing::info!(provider = %provider_id, "Quick-switching AI provider");
        // In production: update the active provider in the runtime
    });

    // V62 — Save budget limit
    let ui_weak = ui.as_weak();
    ui.on_save_budget(move |value| {
        let val = value.to_string();
        tracing::info!(budget = %val, "Budget limit set");

        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_settings_budget_limit(value);
        // In production: persist and start tracking
    });

    // Push initial AI privacy mode to status bar
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // Determine privacy mode from settings
        let is_local_only = ui.get_settings_privacy_local_only();
        let mode = if is_local_only {
            "local"
        } else {
            // Check if we have any cloud providers configured
            let providers = ui.get_settings_ai_providers();
            if providers.row_count() > 0 {
                "hybrid"
            } else {
                "local"
            }
        };
        ui.set_ai_privacy_mode(mode.into());

        // Set initial usage data (placeholder values — real values come from runtime)
        ui.set_settings_usage_local_percent(100.0);
        ui.set_settings_usage_cloud_percent(0.0);
        ui.set_settings_usage_total_cost("$0.00".into());
        ui.set_settings_usage_daily_cost("$0.00".into());
        ui.set_settings_usage_tokens_today("0".into());
        ui.set_settings_usage_tokens_month("0".into());
    });
}
