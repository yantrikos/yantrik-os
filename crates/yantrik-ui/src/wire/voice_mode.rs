//! Voice mode wiring — on_mic_pressed + on_cancel_voice.

use std::sync::{Arc, Mutex};

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::voice;
use crate::App;

/// Wire mic and cancel voice callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let voice_session: Arc<Mutex<Option<voice::VoiceSession>>> = Arc::new(Mutex::new(None));

    // Mic pressed — start voice session
    let session_mic = voice_session.clone();
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let voice_config = ctx.voice_config.clone();

    ui.on_mic_pressed(move || {
        let mut session = session_mic.lock().unwrap();
        if session.is_none() {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_voice_active(true);
                ui.set_voice_state(0);
                ui.set_voice_transcribed("".into());
                ui.set_voice_response("".into());
                ui.set_companion_status("listening".into());
            }
            *session = Some(voice::VoiceSession::start(
                bridge.clone(),
                ui_weak.clone(),
                voice_config.clone(),
            ));
            tracing::info!("Voice mode started");
        }
    });

    // Cancel voice — stop session
    let session_cancel = voice_session.clone();
    let ui_weak_cancel = ui.as_weak();

    ui.on_cancel_voice(move || {
        let mut session = session_cancel.lock().unwrap();
        if let Some(mut s) = session.take() {
            s.stop();
        }
        if let Some(ui) = ui_weak_cancel.upgrade() {
            ui.set_voice_active(false);
            ui.set_companion_status("idle".into());
        }
        tracing::info!("Voice mode cancelled");
    });
}
