//! Image Viewer wiring — prev/next navigation and fit toggle.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

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

/// Load the current image into the UI. Called from callbacks.rs when opening an image.
pub fn load_current_image(ui: &App, state: &ImageViewerState) {
    if let Some(path) = state.current_path() {
        let img = slint::Image::load_from_path(path);
        match img {
            Ok(image) => {
                ui.set_viewer_image(image);
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "Failed to load image");
                ui.set_viewer_image(slint::Image::default());
            }
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        ui.set_viewer_file_name(name.to_string().into());
        ui.set_viewer_counter(state.counter_text().into());
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
}
