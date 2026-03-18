//! Yantrik Image Viewer — standalone app binary.
//!
//! Image display with zoom, rotate, slideshow, crop, and batch operations.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-image-viewer");

    let app = ImageViewerApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

fn wire(app: &ImageViewerApp) {
    // ── Navigation ──
    app.on_nav_prev(|| { tracing::info!("Navigate previous image"); });
    app.on_nav_next(|| { tracing::info!("Navigate next image"); });

    // ── Fit toggle ──
    {
        let weak = app.as_weak();
        app.on_toggle_fit(move || {
            let Some(ui) = weak.upgrade() else { return };
            let current = ui.get_fit_contain();
            ui.set_fit_contain(!current);
        });
    }

    // ── Rotation / Flip ──
    {
        let weak = app.as_weak();
        app.on_viewer_rotate_left(move || {
            let Some(ui) = weak.upgrade() else { return };
            let r = (ui.get_viewer_rotation() + 270) % 360;
            ui.set_viewer_rotation(r);
        });
    }
    {
        let weak = app.as_weak();
        app.on_viewer_rotate_right(move || {
            let Some(ui) = weak.upgrade() else { return };
            let r = (ui.get_viewer_rotation() + 90) % 360;
            ui.set_viewer_rotation(r);
        });
    }
    {
        let weak = app.as_weak();
        app.on_viewer_flip_horizontal(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_viewer_flip_h(!ui.get_viewer_flip_h());
        });
    }
    {
        let weak = app.as_weak();
        app.on_viewer_flip_vertical(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_viewer_flip_v(!ui.get_viewer_flip_v());
        });
    }

    // ── EXIF info panel ──
    {
        let weak = app.as_weak();
        app.on_viewer_toggle_info(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_viewer_info_open(!ui.get_viewer_info_open());
        });
    }

    // ── Slideshow ──
    {
        let weak = app.as_weak();
        app.on_viewer_slideshow_toggle(move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_viewer_slideshow_active() {
                ui.set_viewer_slideshow_paused(!ui.get_viewer_slideshow_paused());
            } else {
                ui.set_viewer_slideshow_active(true);
                ui.set_viewer_slideshow_paused(false);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_viewer_slideshow_stop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_viewer_slideshow_active(false);
            ui.set_viewer_slideshow_paused(false);
            ui.set_viewer_slideshow_progress(0.0);
        });
    }

    // ── Crop mode ──
    {
        let weak = app.as_weak();
        app.on_viewer_start_crop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_viewer_crop_mode(true);
        });
    }
    app.on_viewer_apply_crop(|| { tracing::info!("Apply crop (standalone mode)"); });
    {
        let weak = app.as_weak();
        app.on_viewer_cancel_crop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_viewer_crop_mode(false);
        });
    }

    // ── Batch operations ──
    app.on_viewer_batch_rotate_all(|degrees| { tracing::info!("Batch rotate all: {degrees} degrees"); });
    app.on_viewer_batch_resize(|w, h| { tracing::info!("Batch resize: {w}x{h}"); });

    // ── AI assist ──
    app.on_ai_describe_pressed(|| { tracing::info!("AI describe requested (standalone mode)"); });
    app.on_ai_dismiss(|| { tracing::info!("AI dismiss"); });
}
