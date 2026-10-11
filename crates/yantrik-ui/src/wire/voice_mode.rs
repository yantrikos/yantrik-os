//! Voice mode wiring — push-to-talk hotkey, runtime lifecycle, context snapshots.
//!
//! The VoiceRuntime is started once at boot and lives for the entire session.
//! Super+Space (or mic button) activates listening. The runtime handles:
//! - VAD → STT → the Lens's mind (`chat::send_turn`) → TTS pipeline
//! - Follow mode (15s window for back-and-forth)
//! - Barge-in detection during TTS playback
//! - OS context injection for each voice turn

use std::sync::Arc;

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::streaming::Streams;
use crate::voice::{SendTurn, VoiceMode, VoiceRuntime, VoiceTurn};
use crate::App;

thread_local! {
    /// The pumps for spoken turns' replies in the Lens, as each typed entry point holds its own.
    /// Lives on the UI thread, where `send_turn` runs.
    static STREAMS: Streams = Streams::new();
}

/// How the voice runtime, on its own thread, sends a turn: on the UI thread through the Lens's
/// own send path, waiting for the reply stream to be handed back.
fn send_turn(ctx: &AppContext, ui: &App) -> SendTurn {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    Arc::new(move |turn: VoiceTurn| {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (bridge, ui_weak) = (bridge.clone(), ui_weak.clone());
        slint::invoke_from_event_loop(move || {
            let reply = STREAMS.with(|streams| {
                super::chat::send_turn(&ui_weak, &bridge, &turn.text, turn.origin, &turn.desk, streams)
            });
            let _ = tx.send(reply);
        })
        .ok()?;
        rx.recv_timeout(std::time::Duration::from_secs(10)).ok()
    })
}

/// Wire voice callbacks and start the persistent voice runtime.
pub fn wire(ui: &App, ctx: &AppContext) {
    // Start the voice runtime (persistent — lives for entire session)
    let runtime = VoiceRuntime::start(
        ctx.bridge.clone(),
        ui.as_weak(),
        ctx.voice_config.clone(),
        send_turn(ctx, ui),
    );
    let voice_state = runtime.state.clone();

    // Store runtime in a leaked Arc so it lives forever
    // (the runtime thread will shut down when the app exits)
    let _runtime = Arc::new(runtime);
    let runtime_keep = _runtime.clone();

    // ── Mic button pressed — activate voice (same as hotkey) ──
    let state_mic = voice_state.clone();
    let ui_weak_mic = ui.as_weak();
    ui.on_mic_pressed(move || {
        let current = state_mic.mode();
        match current {
            VoiceMode::Idle => {
                state_mic.activate();
                if let Some(ui) = ui_weak_mic.upgrade() {
                    ui.set_companion_status("listening".into());
                }
                tracing::info!("Voice: mic button → activate");
            }
            VoiceMode::Speaking => {
                // Barge-in via button
                state_mic.activate();
                tracing::info!("Voice: mic button → barge-in");
            }
            VoiceMode::Following => {
                // Already in follow mode, just ensure listening
                tracing::info!("Voice: mic button in follow mode (already listening)");
            }
            _ => {}
        }
    });

    // ── Cancel voice — force idle ──
    let state_cancel = voice_state.clone();
    let ui_weak_cancel = ui.as_weak();
    ui.on_cancel_voice(move || {
        // Force back to idle by sending shutdown + restart? No — just set idle mode.
        // The runtime will see this on next loop iteration.
        // For now, we use the activate mechanism in reverse.
        // Actually, let's add a proper cancel: just store idle mode directly.
        state_cancel.shutdown();
        if let Some(ui) = ui_weak_cancel.upgrade() {
            ui.set_voice_active(false);
            ui.set_companion_status("idle".into());
        }
        tracing::info!("Voice: cancelled");
    });

    // ── Global hotkey: Super+Space ──
    // This is wired in the Slint key handler (app.slint) which calls voice-hotkey-pressed()
    let state_hotkey = voice_state.clone();
    let ui_weak_hotkey = ui.as_weak();
    ui.on_voice_hotkey_pressed(move || {
        let current = state_hotkey.mode();
        match current {
            VoiceMode::Idle => {
                state_hotkey.activate();
                if let Some(ui) = ui_weak_hotkey.upgrade() {
                    ui.set_companion_status("listening".into());
                }
            }
            VoiceMode::Speaking => {
                state_hotkey.activate(); // barge-in
            }
            VoiceMode::Following => {
                // User pressed hotkey in follow mode — they want to speak
                // Follow mode is already listening, so this is a no-op
            }
            _ => {}
        }
    });

    // ── Context snapshot timer ──
    // Every 500ms, capture current OS state and push it to the voice runtime.
    // This ensures the next voice turn has fresh context.
    let state_ctx = voice_state.clone();
    let ui_weak_ctx = ui.as_weak();
    let ctx_timer = slint::Timer::default();
    ctx_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            let mode = state_ctx.mode();
            // Only capture context when voice is active (not idle)
            if mode == VoiceMode::Idle {
                return;
            }

            if let Some(ui) = ui_weak_ctx.upgrade() {
                let context = capture_os_context(&ui);
                state_ctx.set_context(context);
            }
        },
    );

    // Keep the timer and runtime alive
    std::mem::forget(ctx_timer);
    std::mem::forget(runtime_keep);
}

/// Capture a snapshot of the current OS state for voice context grounding.
fn capture_os_context(ui: &App) -> String {
    let mut ctx = String::new();

    // Active screen / app
    let screen_id = ui.get_current_screen();
    let screen_name = screen_id_to_name(screen_id);
    ctx.push_str(&format!("Active app: {screen_name}\n"));

    // Companion status
    let status = ui.get_companion_status().to_string();
    ctx.push_str(&format!("Companion status: {status}\n"));

    ctx
}

/// Map screen ID to human-readable app name.
fn screen_id_to_name(id: i32) -> &'static str {
    match id {
        0 => "Boot screen",
        1 => "Desktop / Home",
        2 => "Onboarding",
        3 => "Lock screen",
        4 => "Bond screen",
        5 => "Personality",
        6 => "Memory browser",
        7 => "Settings",
        8 => "File manager",
        9 => "Notifications",
        10 => "System info",
        11 => "Image viewer",
        12 => "Text editor",
        13 => "Media player",
        14 => "Terminal",
        15 => "Notes",
        16 => "About",
        17 => "Email",
        18 => "Calendar",
        19 => "Weather",
        20 => "Music player",
        21 => "Package manager / Skill store",
        22 => "Network manager",
        23 => "System monitor",
        _ => "Unknown",
    }
}
