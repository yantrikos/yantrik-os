//! The Mind's view of Images: a projection of the committed model, in the same keys and words
//! the app has always answered, so `yos check image-viewer` reads the same.

use yantrik_app::control::{agent_is_calling, View};

use crate::agent_rule;
use crate::model::{Model, NO_PICTURE};

/// How many of the folder's names `describe` lists.
const FOLDER_NAMES: usize = 200;

pub fn view(m: &Model) -> View {
    // A picture the person opened where an agent may not look: neither it nor the names of the
    // pictures beside it are read back. Who is reading is the dispatch's to say (the caller and
    // the token it installed), and `describe` is read inside it.
    if agent_is_calling() {
        if let Some(why) = agent_rule::hidden(m) {
            return View::new("Images — showing a picture an agent is not shown").with("hidden", why);
        }
    }
    let g = &m.gallery;
    let shown = &m.shown;
    let name = shown.name.clone();
    let summary = if name.is_empty() {
        "Images — nothing open".to_string()
    } else if g.len() > 1 {
        format!("Images — {name}, {} of {}", g.index() + 1, g.len())
    } else {
        format!("Images — {name}")
    };
    let o = shown.orientation;
    View::new(summary)
        .with("file", name)
        .with("path", g.current().map(|p| p.display().to_string()))
        .with("position", g.index() as i64 + 1)
        .with("of", g.len() as i64)
        .with("dimensions", shown.info.dimensions.clone())
        .with("file_size", shown.info.file_size.clone())
        .with("format", shown.info.format.clone())
        .with("taken", shown.info.date_taken.clone())
        .with("camera", shown.info.camera.clone())
        .with("rotation", o.rotation as i64)
        .with("flipped_horizontally", o.flip_h)
        .with("flipped_vertically", o.flip_v)
        .with("fit_to_window", m.view.fit_contain && !m.view.zoomed)
        .with("zoomed", m.view.zoomed)
        .with("info_panel", m.view.info_open)
        .with("slideshow", m.view.slideshow_active)
        .with("notice", shown.notice.clone())
        // Why rotate, fit and toggle_info will refuse right now, in the window's own words.
        .with("view_controls_off", (!shown.has_picture()).then(|| NO_PICTURE.to_string()))
        .with(
            "folder",
            g.paths()
                .iter()
                .take(FOLDER_NAMES)
                .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                .collect::<Vec<_>>(),
        )
}
