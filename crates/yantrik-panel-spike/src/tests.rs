//! What can be proven without a compositor: the Slint half of the spike, through the same
//! `Platform` and `MinimalSoftwareWindow` the Wayland half uses. These do NOT prove anything
//! about layer-shell, wl_shm or the compositor's input routing.

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::software_renderer::PremultipliedRgbaColor;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

use super::*;

/// Paint like `Panel::draw_if_needed`, but into a Vec; the size of the dirty box, if it painted.
fn paint(window: &MinimalSoftwareWindow, buf: &mut [PremultipliedRgbaColor], w: usize) -> Option<(u32, u32)> {
    let mut out = None;
    window.draw_if_needed(|r| {
        let region = r.render(buf, w);
        let s = region.bounding_box_size();
        out = Some((s.width, s.height));
    });
    out
}

fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
    window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
}

/// One test function: Slint's platform is per-thread and set once, and these share it.
#[test]
fn two_windows_one_platform_pointer_keys_and_idle() {
    platform::install();
    let (bar_w, bar) = platform::with_window(|| BarView::new().unwrap());
    let (pop_w, pop) = platform::with_window(|| PopoverView::new().unwrap());
    bar.show().unwrap();
    pop.show().unwrap();
    bar_w.set_size(slint::PhysicalSize::new(1280, 32));
    pop_w.set_size(slint::PhysicalSize::new(360, 200));
    let mut bar_px = vec![PremultipliedRgbaColor::default(); 1280 * 32];
    let mut pop_px = vec![PremultipliedRgbaColor::default(); 360 * 200];

    // First frame paints the whole surface; the two windows are distinct (different sizes).
    assert_eq!(paint(&bar_w, &mut bar_px, 1280), Some((1280, 32)));
    assert_eq!(paint(&pop_w, &mut pop_px, 360), Some((360, 200)));
    // Settled: nothing changed, so nothing is painted. This is the idle claim (0 redraws).
    for _ in 0..100 {
        assert_eq!(paint(&bar_w, &mut bar_px, 1280), None);
        assert_eq!(paint(&pop_w, &mut pop_px, 360), None);
    }

    // The button is at the right edge (96px wide, 12px padding); a click fires the callback.
    let fired = Rc::new(Cell::new(0));
    let f = fired.clone();
    bar.on_toggle_popover(move || f.set(f.get() + 1));
    click(&bar_w, 1280.0 - 12.0 - 48.0, 16.0);
    assert_eq!(fired.get(), 1, "a click on the button reaches the Slint callback");
    // A click on empty bar does not.
    click(&bar_w, 400.0, 16.0);
    assert_eq!(fired.get(), 1);

    // Hover/press repaint only a small box, not the 1280x32 surface.
    let (dw, dh) = paint(&bar_w, &mut bar_px, 1280).expect("the click changed hover/press state");
    assert!(dw < 1280 && dh <= 32, "damage should be the button, was {dw}x{dh}");
    println!("button hover/press damage box: {dw}x{dh} of 1280x32");

    // The label follows state set from Rust.
    bar.set_popover_open(true);
    assert!(paint(&bar_w, &mut bar_px, 1280).is_some());
    assert_eq!(paint(&bar_w, &mut bar_px, 1280), None);

    // Keys typed into the popover window land in the popover's field, not the bar.
    pop_w.dispatch_event(WindowEvent::WindowActiveChanged(true));
    for c in ["h", "i"] {
        pop_w.dispatch_event(WindowEvent::KeyPressed { text: c.into() });
        pop_w.dispatch_event(WindowEvent::KeyReleased { text: c.into() });
    }
    assert_eq!(pop.get_typed().as_str(), "hi");
    assert!(paint(&pop_w, &mut pop_px, 360).is_some());
}

/// What wakes an idle process: Slint's own timers. Printed, not asserted, while it is being
/// understood; the finding is in the design doc.
#[test]
fn which_slint_timers_keep_an_idle_bar_awake() {
    platform::install();
    let (bar_w, bar) = platform::with_window(|| BarView::new().unwrap());
    bar.show().unwrap();
    bar_w.set_size(slint::PhysicalSize::new(1280, 32));
    let mut px = vec![PremultipliedRgbaColor::default(); 1280 * 32];
    paint(&bar_w, &mut px, 1280);
    slint::platform::update_timers_and_animations();
    println!("bar alone, settled: next timer {:?}", slint::platform::duration_until_next_timer_update());
    let (pop_w, pop) = platform::with_window(|| PopoverView::new().unwrap());
    slint::platform::update_timers_and_animations();
    println!("+ popover created, never shown: next timer {:?}", slint::platform::duration_until_next_timer_update());
    pop.show().unwrap();
    pop_w.set_size(slint::PhysicalSize::new(360, 200));
    pop_w.dispatch_event(WindowEvent::WindowActiveChanged(true));
    slint::platform::update_timers_and_animations();
    println!("+ popover shown and active: next timer {:?}", slint::platform::duration_until_next_timer_update());
    pop.hide().unwrap();
    slint::platform::update_timers_and_animations();
    println!("+ popover hidden again: next timer {:?}", slint::platform::duration_until_next_timer_update());
    pop_w.dispatch_event(WindowEvent::WindowActiveChanged(false));
    slint::platform::update_timers_and_animations();
    println!("+ ... and inactive: next timer {:?}", slint::platform::duration_until_next_timer_update());
}
