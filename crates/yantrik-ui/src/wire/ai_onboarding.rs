//! AI Onboarding wiring — V59 hardware scan + provider setup during first boot.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

/// Wire AI onboarding callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // AI mode selected (local/cloud/later)
    ui.on_onboard_ai_mode_selected(move |mode| {
        tracing::info!(mode = %mode, "Onboarding: AI mode selected");
    });

    // Provider selected during onboarding
    ui.on_onboard_ai_provider_selected(move |provider| {
        tracing::info!(provider = %provider, "Onboarding: AI provider selected");
    });

    // API key submitted during onboarding
    let ui_weak = ui.as_weak();
    ui.on_onboard_ai_api_key_submitted(move |provider, key| {
        let provider_str = provider.to_string();
        let _key_str = key.to_string();
        tracing::info!(provider = %provider_str, "Onboarding: API key submitted");

        // In a full implementation, this would save the key and configure the provider.
        // For now, just log it. The test-connection callback handles validation.
        let Some(_ui) = ui_weak.upgrade() else { return };
    });

    // Test AI connection during onboarding
    let ui_weak = ui.as_weak();
    ui.on_onboard_ai_test_connection(move || {
        tracing::info!("Onboarding: Testing AI connection");

        let weak = ui_weak.clone();
        // Set testing state
        if let Some(ui) = weak.upgrade() {
            ui.set_onboard_ai_test_status("testing".into());
        }

        // Simulate test (in production, would actually hit the provider)
        let weak2 = weak.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak2.upgrade() {
                    // Simulate success for now
                    ui.set_onboard_ai_test_status("success".into());
                    ui.set_onboard_ai_test_model("auto-detected".into());
                    ui.set_onboard_ai_test_latency_ms(150);
                    ui.set_onboard_ai_test_privacy("local".into());
                }
            });
        });
    });

    // Skip AI setup
    ui.on_onboard_ai_skip_setup(move || {
        tracing::info!("Onboarding: AI setup skipped, using bundled fallback");
    });

    // Populate hardware scan data (simulated — in production, would use sysinfo)
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // Detect hardware capabilities
        ui.set_onboard_ai_hw_cpu_ok(true);
        ui.set_onboard_ai_hw_cpu_label("Detected".into());
        ui.set_onboard_ai_hw_ram_ok(true);
        ui.set_onboard_ai_hw_ram_label("Available".into());

        // GPU detection — check for NVIDIA via environment or sysinfo
        // For now, assume GPU is present if running on desktop
        ui.set_onboard_ai_hw_gpu_ok(true);
        ui.set_onboard_ai_hw_gpu_label("Available".into());

        ui.set_onboard_ai_hw_disk_ok(true);
        ui.set_onboard_ai_hw_disk_label("Sufficient".into());
        ui.set_onboard_ai_hw_network_ok(true);

        // Check if Ollama is running (try to connect to default port)
        let weak2 = ui.as_weak();
        std::thread::spawn(move || {
            let ollama_ok = check_ollama_running();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak2.upgrade() {
                    ui.set_onboard_ai_hw_runtime_ok(ollama_ok);
                    // Recommend based on GPU + runtime availability
                    let recommend = if ollama_ok { "local" } else { "cloud" };
                    ui.set_onboard_ai_hw_recommend(recommend.into());
                }
            });
        });
    });
}

/// Check if Ollama is running on the default port.
fn check_ollama_running() -> bool {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(3))
        .build();

    match agent.get("http://localhost:11434/api/tags").call() {
        Ok(_) => true,
        Err(_) => false,
    }
}
