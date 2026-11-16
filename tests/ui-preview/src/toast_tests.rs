//! A toast over the column of cards for promised things (#1184, VM 520: the "Got it" toast was not
//! in the 2x pictures). The shell shows "Put away. Preparing stays on." after every press; this
//! draws that toast over a full card in the whisper column, at 1920x1080 physical at scale 1 and at
//! scale 2 (960x540 logical), and checks it is drawn whole, in the corner, above the dock.
//!
//! If this passes at 2x, the banner is not what hid the toast on the machine: look at how the shot
//! was taken (`CaptureState.hiding` removes every toast from the shell's own screenshot) and at the
//! window stacking, with the live toasts `describe shell` lists.

use super::*;
use slint::{ComponentHandle, ModelRc, VecModel};

const GROUND: (u8, u8, u8) = (0x20, 0x40, 0x60);
const SUMMARY: &str = "Put away. Preparing stays on.";
/// What the banner gives the summary: the toast is 360px, less its padding (14 and 12), the icon (34),
/// the dismiss button (24) and the two gaps of 10 between them.
const SUMMARY_ROOM: f32 = 360.0 - 14.0 - 12.0 - 34.0 - 24.0 - 20.0;

fn toast() -> ToastData {
    ToastData {
        id: "local:Yantrik:Put away. Preparing stays on.".into(),
        app_name: "Yantrik".into(),
        summary: SUMMARY.into(),
        body: "".into(),
        urgency: 1,
        icon_char: "Y".into(),
        actions: ModelRc::default(),
        thumbnail: Default::default(),
        has_thumbnail: false,
    }
}

fn check(w: &MinimalSoftwareWindow, ui: &ToastProbe, scale: usize) {
    let (width, height) = (1920, 1080);
    let (lw, lh) = (width / scale, height / scale);
    ui.set_screen_width(lw as f32);
    ui.set_screen_height(lh as f32);
    ui.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale as f32 });
    w.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width as u32, height as u32);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width); });
    }
    let px = |x: usize, y: usize| { let c = p.as_slice()[y * width + x]; (c.r, c.g, c.b) };
    let amber = ui.get_amber();
    let amber = (amber.red(), amber.green(), amber.blue());
    // The accent bar down the toast's left edge: logical x from lw-380, 3px wide.
    let bar_x = (lw - 380 + 1) * scale;
    let rows: Vec<usize> = (0..height).filter(|&y| px(bar_x, y) == amber && y > height / 2).collect();
    assert!(!rows.is_empty(), "{scale}x: no toast bar is drawn at x={} (logical {})", bar_x, lw - 380 + 1);
    let (top, bottom) = (rows[0], rows[rows.len() - 1]);
    assert_eq!(rows.len(), bottom - top + 1, "{scale}x: the bar is one run");
    // Pinned above the dock: the stack's bottom edge is the dock and the gap above the screen's bottom.
    let limit = (lh as f32 - ui.get_taskbar_height() - ui.get_gap()).round() as usize * scale;
    assert!(bottom <= limit && bottom + 2 * scale >= limit, "{scale}x: the toast ends at {bottom}, the dock's edge is at {limit}");
    assert!(bottom - top >= 40 * scale, "{scale}x: the bar is only {}px tall", bottom - top);
    // The card's fill is drawn inside the right 360px, and the picture there is more than the ground.
    let (cx0, cx1) = ((lw - 380 + 8) * scale, (lw - 20 - 8) * scale);
    let mid = (top + bottom) / 2;
    assert!((cx0..cx1).all(|x| px(x, mid) != GROUND), "{scale}x: a gap of bare ground inside the toast's width at y={mid}");
    // Its words fit: the summary is not elided.
    let width_of_summary = ui.get_summary_width();
    assert!(width_of_summary <= SUMMARY_ROOM, "{scale}x: the summary needs {width_of_summary}px of {SUMMARY_ROOM}px, so it is cut");
    println!("PASS toast at {scale}x: drawn in the corner, above the dock, whole");
}

pub fn run(w: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ToastProbe::new()?;
    ui.show()?;
    ui.set_summary(SUMMARY.into());
    ui.set_toasts(ModelRc::new(VecModel::from(vec![toast()])));
    ui.global::<PrepState>().set_cards(ModelRc::new(VecModel::from(vec![crate::prep_card_tests::full_card("For your promise to Maya on Friday: 2 files found. The draft is in Email with the report attached. Nothing sent or opened for you.")])));
    check(w, &ui, 1);
    check(w, &ui, 2);
    Ok(())
}
