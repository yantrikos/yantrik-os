//! Chat wiring — on_send_message + on_lens_submit.
//!
//! Both use the shared streaming helper to send text to the companion
//! and stream tokens back into the message model.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Timer};

use crate::app_context::AppContext;
use crate::{apps, lens, streaming, App};

/// Wire on_send_message and on_lens_submit callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_send_message(ui, ctx);
    wire_lens_submit(ui, ctx);
}

/// Direct chat: send message → stream response.
fn wire_send_message(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let timer_slot: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let slot = timer_slot.clone();

    ui.on_send_message(move |text| {
        let text = text.to_string();
        if text.is_empty() {
            return;
        }
        // V22: No offline guard — companion handles offline mode internally
        // via OfflineResponder (memory recall + pattern matching + templates)
        streaming::start_ai_stream(ui_weak.clone(), &bridge, &text, &slot);
    });
}

/// Lens submit: try app launch first, fall back to AI streaming.
fn wire_lens_submit(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let apps = ctx.installed_apps.clone();
    let timer_slot: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let slot = timer_slot.clone();

    ui.on_lens_submit(move |query| {
        let query = query.to_string();
        if query.is_empty() {
            return;
        }

        tracing::info!(query = %query, "Lens submit");

        let lower = query.to_lowercase();

        // Check installed .desktop apps first
        let app_matches = apps::search(&lower, &apps);
        if let Some(entry) = app_matches.first() {
            let parts: Vec<&str> = entry.exec.split_whitespace().collect();
            if let Some((bin, args)) = parts.split_first() {
                tracing::info!(exec = %entry.exec, name = %entry.name, "Launching app from Lens");
                match std::process::Command::new(bin).args(args).spawn() {
                    Ok(_) => tracing::info!(name = %entry.name, "App started"),
                    Err(e) => tracing::error!(name = %entry.name, error = %e, "Failed to launch"),
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
                return;
            }
        }

        // Fallback: hardcoded KNOWN_APPS
        for (app_id, cmd, _) in lens::KNOWN_APPS {
            if lower.contains(&format!("open {}", app_id))
                || lower.contains(app_id)
                || lower.contains(cmd)
            {
                tracing::info!(cmd, "Launching app from Lens (fallback)");
                match std::process::Command::new(cmd).spawn() {
                    Ok(_) => tracing::info!(cmd, "App started"),
                    Err(e) => tracing::error!(cmd, error = %e, "Failed to launch app"),
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
                return;
            }
        }

        // V22: No offline guard — companion handles offline mode internally
        streaming::start_ai_stream(ui_weak.clone(), &bridge, &query, &slot);
    });
}
