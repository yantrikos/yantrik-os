//! Wire clipboard history panel — populate entries, handle paste, handle search.

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::clipboard::SharedHistory;
use crate::{App, ClipboardEntryData};

/// Wire clipboard panel callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_open(ui, ctx);
    wire_paste(ui, ctx);
    wire_search(ui, ctx);
}

/// When the panel opens, populate the entry list from SharedHistory.
fn wire_open(ui: &App, ctx: &AppContext) {
    let clip = ctx.clip_history.clone();
    let ui_weak = ui.as_weak();

    // Watch the `clip-panel-open` property — refresh entries on open.
    // We use a small timer that checks for the panel being opened.
    let was_open = std::cell::Cell::new(false);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(200),
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let is_open = ui.get_clip_panel_open();

            // Only refresh when transitioning from closed -> open
            if is_open && !was_open.get() {
                // Clear search query on fresh open
                ui.set_clip_search_query("".into());
                populate_entries(&ui, &clip, "");
            }
            was_open.set(is_open);
        },
    );
    std::mem::forget(timer);
}

/// Build the model from SharedHistory (optionally filtered) and push it to the UI.
fn populate_entries(ui: &App, clip: &SharedHistory, query: &str) {
    let history = clip.lock().unwrap();

    let entries: Vec<ClipboardEntryData> = if query.is_empty() {
        // No search — show all recent
        history
            .recent(20)
            .iter()
            .enumerate()
            .map(|(i, e)| ClipboardEntryData {
                index: i as i32,
                preview: e.preview().into(),
                time_ago: e.time_ago().into(),
            })
            .collect()
    } else {
        // Search — case-insensitive substring match
        history
            .search(query)
            .iter()
            .map(|(idx, e)| ClipboardEntryData {
                index: *idx as i32,
                preview: e.preview().into(),
                time_ago: e.time_ago().into(),
            })
            .collect()
    };

    let model = VecModel::from(entries);
    ui.set_clip_panel_entries(ModelRc::new(model));
}

/// Handle paste — copy selected entry back to clipboard via wl-copy.
fn wire_paste(ui: &App, ctx: &AppContext) {
    let clip = ctx.clip_history.clone();
    ui.on_clipboard_paste(move |index| {
        let history = clip.lock().unwrap();
        if let Some(entry) = history.get(index as usize) {
            let content = entry.content.clone();
            drop(history); // release lock before spawning

            // Write to clipboard via wl-copy
            std::thread::spawn(move || {
                match std::process::Command::new("wl-copy")
                    .arg(&content)
                    .status()
                {
                    Ok(s) if s.success() => {
                        tracing::debug!("Clipboard paste: wrote {} bytes", content.len());
                    }
                    Ok(s) => {
                        tracing::warn!("wl-copy exited with {}", s);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "wl-copy failed");
                    }
                }
            });
        }
    });
}

/// Handle search — re-populate entries filtered by query string.
fn wire_search(ui: &App, ctx: &AppContext) {
    let clip = ctx.clip_history.clone();
    let ui_weak = ui.as_weak();
    ui.on_clipboard_search(move |query| {
        let query_str: String = query.into();
        let Some(ui) = ui_weak.upgrade() else { return };
        populate_entries(&ui, &clip, &query_str);
    });
}
