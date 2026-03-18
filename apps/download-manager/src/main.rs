//! Yantrik Download Manager — standalone app binary.
//!
//! Manages file downloads with pause/resume/checksum support.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-download-manager");

    let app = DownloadManagerApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &DownloadManagerApp) {
    // Start with empty download list
    app.set_downloads(ModelRc::new(VecModel::<DownloadItem>::default()));

    // Add download
    app.on_dl_add(|url, checksum, save_dir| {
        tracing::info!(
            "Download requested: url={}, checksum={}, save_dir={}",
            url, checksum, save_dir
        );
    });

    // Pause
    app.on_dl_pause(|id| {
        tracing::info!("Pause download {}", id);
    });

    // Resume
    app.on_dl_resume(|id| {
        tracing::info!("Resume download {}", id);
    });

    // Cancel
    app.on_dl_cancel(|id| {
        tracing::info!("Cancel download {}", id);
    });

    // Retry
    app.on_dl_retry(|id| {
        tracing::info!("Retry download {}", id);
    });

    // Open folder
    app.on_dl_open_folder(|id| {
        tracing::info!("Open folder for download {}", id);
    });

    // Clear completed
    {
        let weak = app.as_weak();
        app.on_dl_clear_completed(move || {
            if let Some(ui) = weak.upgrade() {
                let model = ui.get_downloads();
                let remaining: Vec<DownloadItem> = (0..model.row_count())
                    .filter_map(|i| {
                        let item = model.row_data(i)?;
                        if item.status.as_str() != "completed" {
                            Some(item)
                        } else {
                            None
                        }
                    })
                    .collect();
                ui.set_downloads(ModelRc::new(VecModel::from(remaining)));
            }
        });
    }

    // Filter
    {
        let weak = app.as_weak();
        app.on_dl_filter(move |filter| {
            if let Some(ui) = weak.upgrade() {
                ui.set_active_filter(filter);
            }
        });
    }

    // Verify checksum
    app.on_dl_verify_checksum(|id| {
        tracing::info!("Verify checksum for download {}", id);
    });

    // Toggle select
    app.on_dl_toggle_select(|id| {
        tracing::info!("Toggle select for download {}", id);
    });

    // Select all
    app.on_dl_select_all(|selected| {
        tracing::info!("Select all: {}", selected);
    });

    // Pause all
    app.on_dl_pause_all(|| {
        tracing::info!("Pause all downloads");
    });

    // Resume all
    app.on_dl_resume_all(|| {
        tracing::info!("Resume all downloads");
    });

    // Search
    app.on_dl_search(|query| {
        tracing::info!("Search downloads: {}", query);
    });

    // Sort
    app.on_dl_sort(|mode| {
        tracing::info!("Sort downloads by mode {}", mode);
    });

    // AI stubs
    app.on_ai_explain_pressed(|| {
        tracing::info!("AI explain requested (standalone mode)");
    });
    app.on_ai_dismiss(|| {});
}
