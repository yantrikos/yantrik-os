//! Wire clipboard history panel — populate entries, handle paste, handle search, clear.

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::clipboard::SharedHistory;
use crate::{App, ClipboardEntryData};

/// Wire clipboard panel callbacks.
///
/// Opening is not wired here: the panel opens from the bar, a keybind and the control surface,
/// and the one hook all three reach is `shell-overlay-opened` (see `shell_overlays`), which calls
/// [`refresh_on_open`]. A 200 ms timer used to watch `clip-panel-open` for it, which woke the
/// shell five times a second for as long as it ran, panel or no panel.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_paste(ui, ctx);
    wire_search(ui, ctx);
    wire_clear(ui, ctx);
}

/// A fresh open: clear the search and load the newest entries, once.
pub(super) fn refresh_on_open(ui: &App, clip: &SharedHistory) {
    ui.set_clip_search_query("".into());
    populate_entries(ui, clip, "");
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

/// "Clear history" in the panel's footer: empty the store, then show what is left (nothing), with
/// the search kept so the person sees the panel answer the press.
fn wire_clear(ui: &App, ctx: &AppContext) {
    let clip = ctx.clip_history.clone();
    let ui_weak = ui.as_weak();
    ui.on_clipboard_clear(move || {
        if let Ok(mut history) = clip.lock() {
            history.clear();
        }
        let Some(ui) = ui_weak.upgrade() else { return };
        populate_entries(&ui, &clip, &ui.get_clip_search_query());
        tracing::info!("Clipboard history cleared");
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
