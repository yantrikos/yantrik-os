//! The email composer's header (email_compose_header.slint), in the real client (`EmailClient`). On
//! VM 520 the attachment sentence sat on the row with Save Draft, Cancel and Send and pushed them off
//! the right edge, so a prepared draft could be neither cancelled nor sent with the pointer. The
//! status now has a row of its own. The compose pane is what is left after the 220px folder column
//! and the 340px list, so the check measures Send in that pane, at 1x and 2x, at three window sizes
//! and at the window's least width: Send is inside the pane, or the window's least width is above
//! that size, so the compositor cannot make the window that narrow.
//!
//! It runs on the build host with the other `verify-*` checks (`tests/ui-preview/validate.sh`);
//! `.github/workflows/ci.yml` does not build this crate.

use super::*;
use slint::ComponentHandle;

/// The full sentence the composer says over a prepared draft that carries a file.
const ATTACHED: &str = "A file is attached to this draft. Email cannot send attachments yet, so it will not be sent from here.";

/// Draw the real client at `scale` in a window of `w` by `h` logical pixels, so its layout settles
/// and reports where the composer's last button and the compose pane end.
fn settle(win: &MinimalSoftwareWindow, ui: &EmailComposeProbe, scale: usize, w: usize, h: usize) {
    let (width, height) = (w * scale, h * scale);
    ui.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale as f32 });
    ui.set_canvas_width(w as f32);
    ui.set_canvas_height(h as f32);
    win.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width as u32, height as u32);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        win.request_redraw();
        win.draw_if_needed(|r| { r.render(p.make_mut_slice(), width); });
    }
}

pub fn run(win: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    let ui = EmailComposeProbe::new()?;
    ui.show()?;
    let long = "A file is attached to this draft and it cannot be sent from here. ".repeat(5);
    assert!(long.len() > 300);
    // The client reports an edge when it changes, not the layout it first settles at: lay it out once
    // at a size none of the cases uses, so every case below is a change it reports.
    settle(win, &ui, 1, 1000, 700);
    settle(win, &ui, 1, 1280, 800);
    let least = ui.get_least_width();
    let mut checked = 0;
    let mut too_narrow = 0;
    let mut slack = f32::MAX;
    for status in ["", ATTACHED, long.as_str()] {
        ui.set_status(status.into());
        for scale in [1, 2] {
            for (w, h) in [(1280, 800), (900, 600), (640, 480), (least as usize, 600)] {
                settle(win, &ui, scale, w, h);
                let (buttons, pane) = (ui.get_buttons_right(), ui.get_pane_right());
                let at = format!("{scale}x, {w}x{h}, a status of {} characters", status.len());
                assert!(buttons > 0.0, "{at}: the composer reported no button edge, so nothing was measured");
                // Send is inside the pane, or the window cannot be made this narrow at all: the
                // compositor honours the window's least width, so a size below it is not one a
                // person can reach. Either way Send is never drawn outside the window.
                let reachable = least <= w as f32;
                assert!(!reachable || (buttons <= pane && pane <= w as f32), "{at}: Send ends at {buttons}px, the pane at {pane}px, the window at {w}px; the window's least width is {least}px");
                too_narrow += usize::from(!reachable);
                if reachable { slack = slack.min(pane - buttons); }
                checked += 1;
            }
        }
    }
    println!("PASS email composer: Save Draft, Cancel and Send stay inside the real compose pane for any status, at 1x and 2x, in {checked} cases ({too_narrow} below the window's least width of {least}px; Send ends at least {slack}px inside the pane)");
    Ok(())
}
