//! Image Viewer wiring — prev/next navigation, fit toggle, rotate/flip,
//! EXIF metadata, and slideshow control.

use std::path::{Path, PathBuf};

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::mime_dispatch;
use crate::App;

/// State for the image viewer.
pub struct ImageViewerState {
    pub images: Vec<PathBuf>,
    pub current_index: usize,
}

impl Default for ImageViewerState {
    fn default() -> Self {
        Self {
            images: Vec::new(),
            current_index: 0,
        }
    }
}

impl ImageViewerState {
    /// Open an image file — populates the sibling list and sets current index.
    pub fn open(&mut self, path: &PathBuf) {
        let (images, idx) = mime_dispatch::sibling_images(path);
        self.images = images;
        self.current_index = idx;
    }

    /// Current image path.
    pub fn current_path(&self) -> Option<&PathBuf> {
        self.images.get(self.current_index)
    }

    /// Navigate to previous image.
    pub fn prev(&mut self) {
        if !self.images.is_empty() {
            if self.current_index == 0 {
                self.current_index = self.images.len() - 1;
            } else {
                self.current_index -= 1;
            }
        }
    }

    /// Navigate to next image.
    pub fn next(&mut self) {
        if !self.images.is_empty() {
            self.current_index = (self.current_index + 1) % self.images.len();
        }
    }

    /// Counter text like "3 / 12".
    pub fn counter_text(&self) -> String {
        if self.images.is_empty() {
            String::new()
        } else {
            format!("{} / {}", self.current_index + 1, self.images.len())
        }
    }
}

/// EXIF metadata extracted from an image file.
#[derive(Default)]
struct ExifInfo {
    dimensions: String,
    file_size: String,
    format: String,
    camera: String,
    focal_length: String,
    iso: String,
    exposure: String,
    date_taken: String,
    gps: String,
}

/// Read EXIF data from an image file.
fn read_exif_info(path: &Path) -> ExifInfo {
    let mut info = ExifInfo::default();

    // File size
    if let Ok(meta) = std::fs::metadata(path) {
        let bytes = meta.len();
        info.file_size = if bytes >= 1_048_576 {
            format!("{:.1} MB", bytes as f64 / 1_048_576.0)
        } else if bytes >= 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{} B", bytes)
        };
    }

    // Format from extension
    if let Some(ext) = path.extension() {
        info.format = ext.to_string_lossy().to_uppercase();
    }

    // Try reading EXIF data
    if let Ok(file) = std::fs::File::open(path) {
        let mut bufreader = std::io::BufReader::new(&file);
        if let Ok(exif_reader) = exif::Reader::new().read_from_container(&mut bufreader) {
            // Dimensions from EXIF
            let pw = exif_reader.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY);
            let ph = exif_reader.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY);
            if let (Some(w), Some(h)) = (pw, ph) {
                let wv = w.value.get_uint(0).unwrap_or(0);
                let hv = h.value.get_uint(0).unwrap_or(0);
                if wv > 0 && hv > 0 {
                    info.dimensions = format!("{} \u{00d7} {}", wv, hv);
                }
            }
            // Fallback: ImageWidth / ImageLength
            if info.dimensions.is_empty() {
                let iw = exif_reader.get_field(exif::Tag::ImageWidth, exif::In::PRIMARY);
                let ih = exif_reader.get_field(exif::Tag::ImageLength, exif::In::PRIMARY);
                if let (Some(w), Some(h)) = (iw, ih) {
                    let wv = w.value.get_uint(0).unwrap_or(0);
                    let hv = h.value.get_uint(0).unwrap_or(0);
                    if wv > 0 && hv > 0 {
                        info.dimensions = format!("{} \u{00d7} {}", wv, hv);
                    }
                }
            }

            // Camera model
            if let Some(f) = exif_reader.get_field(exif::Tag::Model, exif::In::PRIMARY) {
                info.camera = f.display_value().to_string().trim_matches('"').to_string();
            }
            // Prepend make if available
            if let Some(f) = exif_reader.get_field(exif::Tag::Make, exif::In::PRIMARY) {
                let make = f.display_value().to_string().trim_matches('"').to_string();
                if !info.camera.is_empty() && !info.camera.starts_with(&make) {
                    info.camera = format!("{} {}", make, info.camera);
                } else if info.camera.is_empty() {
                    info.camera = make;
                }
            }

            // Focal length
            if let Some(f) = exif_reader.get_field(exif::Tag::FocalLength, exif::In::PRIMARY) {
                info.focal_length = f.display_value().to_string();
            }

            // ISO
            if let Some(f) = exif_reader.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY) {
                info.iso = f.display_value().to_string();
            }

            // Exposure time
            if let Some(f) = exif_reader.get_field(exif::Tag::ExposureTime, exif::In::PRIMARY) {
                info.exposure = f.display_value().to_string();
            }

            // Date taken
            if let Some(f) = exif_reader.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY) {
                info.date_taken = f.display_value().to_string();
            } else if let Some(f) = exif_reader.get_field(exif::Tag::DateTime, exif::In::PRIMARY) {
                info.date_taken = f.display_value().to_string();
            }

            // GPS coordinates
            let lat = exif_reader.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY);
            let lat_ref = exif_reader.get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY);
            let lon = exif_reader.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY);
            let lon_ref = exif_reader.get_field(exif::Tag::GPSLongitudeRef, exif::In::PRIMARY);
            if let (Some(lat_f), Some(lon_f)) = (lat, lon) {
                let lat_r = lat_ref.map(|f| f.display_value().to_string()).unwrap_or_default();
                let lon_r = lon_ref.map(|f| f.display_value().to_string()).unwrap_or_default();
                info.gps = format!("{} {}, {} {}",
                    lat_f.display_value(), lat_r.trim_matches('"'),
                    lon_f.display_value(), lon_r.trim_matches('"'));
            }
        }
    }

    // If dimensions are still empty, try to get them from the loaded image
    if info.dimensions.is_empty() {
        // We'll fill this from slint::Image in load_current_image
    }

    info
}

/// Apply EXIF info to the UI.
fn apply_exif_info(ui: &App, info: &ExifInfo) {
    ui.set_viewer_exif_dimensions(info.dimensions.clone().into());
    ui.set_viewer_exif_file_size(info.file_size.clone().into());
    ui.set_viewer_exif_format(info.format.clone().into());
    ui.set_viewer_exif_camera(info.camera.clone().into());
    ui.set_viewer_exif_focal_length(info.focal_length.clone().into());
    ui.set_viewer_exif_iso(info.iso.clone().into());
    ui.set_viewer_exif_exposure(info.exposure.clone().into());
    ui.set_viewer_exif_date_taken(info.date_taken.clone().into());
    ui.set_viewer_exif_gps(info.gps.clone().into());
}

/// Load the current image into the UI. Called from callbacks.rs when opening an image.
pub fn load_current_image(ui: &App, state: &ImageViewerState) {
    if let Some(path) = state.current_path() {
        let img = slint::Image::load_from_path(path);
        match img {
            Ok(image) => {
                // Get dimensions from the loaded image if EXIF didn't have them
                let img_size = image.size();
                ui.set_viewer_image(image);

                let mut exif_info = read_exif_info(path);
                if exif_info.dimensions.is_empty() && img_size.width > 0 && img_size.height > 0 {
                    exif_info.dimensions = format!("{} \u{00d7} {}", img_size.width, img_size.height);
                }
                apply_exif_info(ui, &exif_info);
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "Failed to load image");
                ui.set_viewer_image(slint::Image::default());
                // Still show file metadata even if image load failed
                let exif_info = read_exif_info(path);
                apply_exif_info(ui, &exif_info);
            }
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        ui.set_viewer_file_name(name.to_string().into());
        ui.set_viewer_counter(state.counter_text().into());

        // Reset rotation/flip when navigating to a new image
        ui.set_viewer_rotation(0);
        ui.set_viewer_flip_h(false);
        ui.set_viewer_flip_v(false);
    }
}

/// Wire image viewer callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let state = ctx.image_viewer_state.clone();

    // Prev
    let ui_weak = ui.as_weak();
    let st = state.clone();
    ui.on_viewer_nav_prev(move || {
        st.borrow_mut().prev();
        if let Some(ui) = ui_weak.upgrade() {
            load_current_image(&ui, &st.borrow());
        }
    });

    // Next
    let ui_weak = ui.as_weak();
    let st = state.clone();
    ui.on_viewer_nav_next(move || {
        st.borrow_mut().next();
        if let Some(ui) = ui_weak.upgrade() {
            load_current_image(&ui, &st.borrow());
        }
    });

    // Fit toggle
    ui.on_viewer_toggle_fit(move || {
        // Handled purely in Slint via two-way binding
    });

    // ── Rotate / Flip callbacks ──
    // Rotation and flip state is tracked in Slint properties.
    // The callbacks allow the backend to react if needed (e.g., saving orientation).
    ui.on_viewer_rotate_left(|| {
        tracing::debug!("Image rotated left (CCW)");
    });
    ui.on_viewer_rotate_right(|| {
        tracing::debug!("Image rotated right (CW)");
    });
    ui.on_viewer_flip_horizontal(|| {
        tracing::debug!("Image flipped horizontally");
    });
    ui.on_viewer_flip_vertical(|| {
        tracing::debug!("Image flipped vertically");
    });

    // ── EXIF Info Panel toggle ──
    let ui_weak = ui.as_weak();
    let st = state.clone();
    ui.on_viewer_toggle_info(move || {
        if let Some(ui) = ui_weak.upgrade() {
            // Re-read EXIF when panel is opened (in case file changed)
            if ui.get_viewer_info_open() {
                let state = st.borrow();
                if let Some(path) = state.current_path() {
                    let mut exif_info = read_exif_info(path);
                    // Try to get dimensions from current image if EXIF doesn't have them
                    if exif_info.dimensions.is_empty() {
                        let img = ui.get_viewer_image();
                        let size = img.size();
                        if size.width > 0 && size.height > 0 {
                            exif_info.dimensions = format!("{} \u{00d7} {}", size.width, size.height);
                        }
                    }
                    apply_exif_info(&ui, &exif_info);
                }
            }
        }
    });

    // ── Crop callbacks ──
    {
        let ui_weak = ui.as_weak();
        ui.on_viewer_start_crop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_viewer_crop_mode(true);
                tracing::debug!("Crop mode started");
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_viewer_apply_crop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                tracing::info!("Crop applied (scaffold — real crop implementation requires image crate)");
                ui.set_viewer_crop_mode(false);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_viewer_cancel_crop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                tracing::debug!("Crop cancelled");
                ui.set_viewer_crop_mode(false);
            }
        });
    }

    // ── Batch rotate all ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_viewer_batch_rotate_all(move |degrees| {
            let images = st.borrow().images.clone();
            let count = images.len() as i32;
            if count == 0 { return; }

            let ui_weak2 = ui_weak.clone();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_viewer_batch_total(count);
                ui.set_viewer_batch_done(0);
                ui.set_viewer_batch_status(format!("Rotating {} images by {}°...", count, degrees).into());
            }

            std::thread::spawn(move || {
                for (i, path) in images.iter().enumerate() {
                    tracing::info!(
                        path = %path.display(),
                        degrees = degrees,
                        "Batch rotate image {}/{}",
                        i + 1, count
                    );
                    // Real implementation would use image crate to rotate and save
                    // For now, just log the operation

                    let done = (i + 1) as i32;
                    let ui_w = ui_weak2.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_w.upgrade() {
                            ui.set_viewer_batch_done(done);
                        }
                    });
                }

                let ui_w = ui_weak2.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_w.upgrade() {
                        ui.set_viewer_batch_status(format!("Rotated {} images by {}° (logged)", count, degrees).into());
                    }
                });
            });
        });
    }

    // ── Batch resize ──
    {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        ui.on_viewer_batch_resize(move |width, height| {
            let images = st.borrow().images.clone();
            let count = images.len() as i32;
            if count == 0 { return; }

            let w = width as u32;
            let h = height as u32;
            if w == 0 || h == 0 { return; }

            let ui_weak2 = ui_weak.clone();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_viewer_batch_total(count);
                ui.set_viewer_batch_done(0);
                ui.set_viewer_batch_status(format!("Resizing {} images to {}x{}...", count, w, h).into());
            }

            std::thread::spawn(move || {
                for (i, path) in images.iter().enumerate() {
                    tracing::info!(
                        path = %path.display(),
                        width = w,
                        height = h,
                        "Batch resize image {}/{}",
                        i + 1, count
                    );
                    // Real implementation would use image crate to resize and save

                    let done = (i + 1) as i32;
                    let ui_w = ui_weak2.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_w.upgrade() {
                            ui.set_viewer_batch_done(done);
                        }
                    });
                }

                let ui_w = ui_weak2.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_w.upgrade() {
                        ui.set_viewer_batch_status(format!("Resized {} images to {}x{} (logged)", count, w, h).into());
                    }
                });
            });
        });
    }

    // ── Slideshow callbacks ──
    ui.on_viewer_slideshow_toggle(|| {
        tracing::debug!("Slideshow toggled");
    });
    ui.on_viewer_slideshow_stop(|| {
        tracing::debug!("Slideshow stopped");
    });

    // ── AI Describe callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_viewer_ai_describe(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let filename = ui.get_viewer_file_name().to_string();
        if filename.is_empty() { return; }

        let prompt = super::ai_assist::image_describe_prompt(&filename);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_viewer_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_viewer_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_viewer_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_viewer_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_viewer_ai_panel_open(false);
        }
    });
}
