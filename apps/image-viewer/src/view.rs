//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that writes the window.

use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;
use yantrik_app::rail;

use crate::model::{
    Command, Model, Shown, Turn, ViewState, PART_AI, PART_FIT, PART_ORIENTATION, PART_PICTURE,
    PART_RAISE, PART_SHOWN, PART_VIEW,
};
use crate::{AgentContextItem, ImageViewer, ImageViewerApp};

pub fn wire(cx: &Cx<ImageViewer>) {
    let ui = cx.ui();
    // ── Navigation ──
    ui.on_nav_prev(cx.cmd(|| Command::Previous));
    ui.on_nav_next(cx.cmd(|| Command::Next));

    // ── Rotation and flip ──
    //
    // The buttons only ask; the turn is done by the app, to the pixels (orient.rs). The screen
    // used to set the properties itself and draw only the rotation, so both flip buttons lit up
    // and left the picture as it was.
    ui.on_viewer_rotate_left(cx.cmd(|| Command::Turn(Turn::RotateLeft)));
    ui.on_viewer_rotate_right(cx.cmd(|| Command::Turn(Turn::RotateRight)));
    ui.on_viewer_flip_horizontal(cx.cmd(|| Command::Turn(Turn::FlipHorizontal)));
    ui.on_viewer_flip_vertical(cx.cmd(|| Command::Turn(Turn::FlipVertical)));

    // ── Describe ──
    ui.on_ai_describe_pressed(cx.cmd(|| Command::AskDescribe));
    ui.on_ai_dismiss(cx.cmd(|| Command::DismissAi));

    // ── The view's own state ──
    //
    // The screen runs these itself: the fit button (`toggle-fit-mode`), the info panel, and the
    // slideshow, whose Timer inside the screen moves the progress bar and calls nav-next when it
    // fills. Each callback only says it happened, and the app takes what the window now shows.
    // It used to toggle the same flag a second time instead, so the header's Info button and its
    // Slideshow button undid their own press: the panel never opened from the header and the
    // slideshow started paused, every time.
    ui.on_toggle_fit(seen(cx));
    ui.on_viewer_toggle_info(seen(cx));
    ui.on_viewer_slideshow_toggle(seen(cx));
    ui.on_viewer_slideshow_stop(seen(cx));
    ui.on_view_changed(seen(cx));
    // The rail's rows are rows to read, not buttons, and it offers nothing to press: there are
    // no suggestion or proposal callbacks to bind (rail.rs).
}

/// A callback that tells the app how the window shows the picture now.
fn seen(cx: &Cx<ImageViewer>) -> impl Fn() + 'static {
    let cx = cx.clone();
    move || {
        let state = view_state(&cx.ui());
        cx.run(Command::ViewSeen(state));
    }
}

fn view_state(ui: &ImageViewerApp) -> ViewState {
    ViewState {
        fit_contain: ui.get_fit_contain(),
        zoomed: ui.get_zoomed(),
        info_open: ui.get_viewer_info_open(),
        slideshow_active: ui.get_viewer_slideshow_active(),
        slideshow_paused: ui.get_viewer_slideshow_paused(),
    }
}

pub fn render(ui: &ImageViewerApp, m: &Model, changed: &Changed) {
    if changed.has(PART_PICTURE) {
        ui.set_current_image(drawn(&m.shown));
    }
    if changed.has(PART_SHOWN) {
        shown(ui, &m.shown);
    }
    if changed.has(PART_ORIENTATION) {
        // The toolbar's lit buttons and the corner badge follow the view's orientation.
        let o = m.shown.orientation;
        ui.set_viewer_rotation(o.rotation);
        ui.set_viewer_flip_h(o.flip_h);
        ui.set_viewer_flip_v(o.flip_v);
    }
    if changed.has(PART_VIEW) && ui.get_viewer_info_open() != m.view.info_open {
        ui.set_viewer_info_open(m.view.info_open);
    }
    if changed.has(PART_FIT) {
        // `fit`, asked of the app: out of a zoom of the person's own through the screen's own
        // step (its zoom and pan are its own), then fit or fill as the model says.
        if ui.get_zoomed() && !m.view.zoomed {
            ui.invoke_toggle_fit_mode();
        }
        if ui.get_fit_contain() != m.view.fit_contain {
            ui.set_fit_contain(m.view.fit_contain);
        }
    }
    if changed.has(PART_AI) {
        ui.set_ai_is_working(m.ai.working);
        ui.set_ai_response(m.ai.response.as_str().into());
    }
    // Only once asked: the first render draws every part, and that is no request to come forward.
    if changed.has(PART_RAISE) && m.raises > 0 {
        ui.window().set_minimized(false);
    }
    if changed.has(rail::PART) {
        let context: Vec<AgentContextItem> = m
            .rail
            .context()
            .iter()
            .map(|c| AgentContextItem {
                id: c.id.as_str().into(),
                label: c.label.as_str().into(),
                detail: c.detail.as_str().into(),
                source: c.source.as_str().into(),
            })
            .collect();
        ui.set_agent_context(ModelRc::new(VecModel::from(context)));
        // The rail offers nothing to ask, so it has no "cannot ask" to say either.
        ui.set_agent_unavailable(SharedString::new());
    }
}

/// The picture as it is drawn: turned, once the turned pixels have landed; as stored otherwise.
fn drawn(shown: &Shown) -> slint::Image {
    let Some(picture) = &shown.picture else {
        return slint::Image::default();
    };
    match &shown.turned {
        Some((_, pixels)) if !shown.orientation.is_identity() => slint::Image::from_rgba8(pixels.0.clone()),
        _ => picture.image(),
    }
}

/// The name, the counter, the notice and what the file says.
fn shown(ui: &ImageViewerApp, s: &Shown) {
    ui.set_file_name(s.name.as_str().into());
    ui.set_counter_text(s.counter.as_str().into());
    ui.set_notice(s.notice.as_str().into());
    let info = &s.info;
    ui.set_viewer_exif_dimensions(info.dimensions.as_str().into());
    ui.set_viewer_exif_file_size(info.file_size.as_str().into());
    ui.set_viewer_exif_format(info.format.as_str().into());
    ui.set_viewer_exif_camera(info.camera.as_str().into());
    ui.set_viewer_exif_focal_length(info.focal_length.as_str().into());
    ui.set_viewer_exif_iso(info.iso.as_str().into());
    ui.set_viewer_exif_exposure(info.exposure.as_str().into());
    ui.set_viewer_exif_date_taken(info.date_taken.as_str().into());
    ui.set_viewer_exif_gps(info.gps.as_str().into());
}
