//! The email composer's header (email_compose_header.slint). On VM 520 the attachment sentence sat on
//! the row with Save Draft, Cancel and Send and pushed them off the right edge, so a prepared draft
//! could be neither cancelled nor sent with the pointer. The status now has a row of its own: the
//! buttons stay inside the pane, nothing is drawn past it, and the least width does not depend on how
//! much the status says, at any scale and any window size.

use super::*;
use slint::ComponentHandle;

const GROUND: (u8, u8, u8) = (0x20, 0x40, 0x60);
/// The full sentence the composer says over a prepared draft that carries a file.
const ATTACHED: &str = "A file is attached to this draft. Email cannot send attachments yet, so it will not be sent from here.";

/// Draw the probe at `scale` in a window of `w` by `h` logical pixels; the strip right of the pane
/// is the 40px at the right edge. Returns the pixels drawn there that are not the ground.
fn strip_drawn(win: &MinimalSoftwareWindow, ui: &EmailComposeProbe, scale: usize, w: usize, h: usize) -> usize {
    let (width, height) = (w * scale, h * scale);
    ui.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale as f32 });
    win.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width as u32, height as u32);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        win.request_redraw();
        win.draw_if_needed(|r| { r.render(p.make_mut_slice(), width); });
    }
    let pane_right = ui.get_pane_right().round() as usize * scale;
    (0..height).flat_map(|y| (pane_right + scale..width).map(move |x| (x, y))).filter(|&(x, y)| { let c = p.as_slice()[y * width + x]; (c.r, c.g, c.b) != GROUND }).count()
}

pub fn run(win: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    let ui = EmailComposeProbe::new()?;
    ui.show()?;
    let long = "A file is attached to this draft and it cannot be sent from here. ".repeat(5);
    assert!(long.len() > 300);
    ui.set_status("".into());
    strip_drawn(win, &ui, 1, 1280, 800);
    let least = ui.get_header_min_width();
    let mut checked = 0;
    for status in ["", ATTACHED, long.as_str()] {
        ui.set_status(status.into());
        for scale in [1, 2] {
            for (w, h) in [(1280, 800), (900, 600), (640, 480)] {
                let past = strip_drawn(win, &ui, scale, w, h);
                let (buttons, pane) = (ui.get_buttons_right(), ui.get_pane_right());
                let at = format!("{scale}x, {w}x{h}, a status of {} characters", status.len());
                assert!(buttons <= pane, "{at}: Send ends at {buttons}px, the pane at {pane}px");
                assert_eq!(past, 0, "{at}: {past} pixels are drawn past the pane");
                assert_eq!(ui.get_header_min_width(), least, "{at}: the status changed the header's least width");
                checked += 1;
            }
        }
    }
    println!("PASS email composer: Save Draft, Cancel and Send stay inside the pane for any status, at 1x and 2x, in {checked} cases");
    Ok(())
}
