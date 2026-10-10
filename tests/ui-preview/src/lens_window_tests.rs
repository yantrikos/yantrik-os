//! The Lens in its own window (lens_window.slint's `LensWindow`, #733), at the size labwc gives it
//! on a 1280x800 display: 464 wide (the 440px panel and its 12px margins) and 716 tall, the room
//! between the 36px status bar and the 48px dock.
//!
//! Checks what a person would see and press: open, the conversation panel fills the window from
//! its top margin to its bottom one, with the transparent margins either side; a card waiting
//! that the conversation does not host is drawn over the panel's top and its Deny answers a
//! click; the Lens's own card in chat mode is the conversation's, not drawn twice; closed, the
//! window is the card alone. Renders are `lens-window.png` and `lens-window-card.png`.
use super::*;
use super::approval_tests::{card, message, save, scan, settle};
use slint::{ModelRc, VecModel};

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (464u32, 716u32);
    let ui = LensWindow::new()?;
    let denied = std::rc::Rc::new(std::cell::Cell::new(0));
    {
        let denied = denied.clone();
        ui.on_approval_deny(move |_| denied.set(denied.get() + 1));
    }
    ui.set_usable_height(height as f32);
    ui.set_lens_open(true);
    ui.set_lens_chat_mode(true);
    ui.set_active_harness_name("Yantrik Mind".into());
    ui.set_messages(ModelRc::new(VecModel::from(vec![
        message("user", "What is using my disk?"),
        message("assistant", "Your Videos folder: 41 GB of the 60 GB used."),
    ])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let open = settle(w, width, height);
    save(&open, output, width, height)?;
    let at = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, x: u32, y: u32| p.as_slice()[(y * width + x) as usize];
    let blank = slint::Rgb8Pixel { r: 0, g: 0, b: 0 };
    // The panel: drawn from 12px in on every side, so mid-panel is painted and the margins are not.
    assert_ne!(at(&open, width / 2, height / 2), blank, "the conversation panel fills the window");
    assert_ne!(at(&open, width / 2, height - 20), blank, "down to its bottom margin, where the composer is");
    assert_eq!(at(&open, 4, height / 2), blank, "the left margin is transparent");
    assert_eq!(at(&open, width / 2, 4), blank, "and so is the top one");
    assert!(!ui.get_card_shown(), "nothing waits, nothing drawn over it");

    // A card another caller raised: over the panel's top, answerable.
    let other = card("Delete the event “Dentist” from Calendar.");
    ui.set_pending_approvals(ModelRc::new(VecModel::from(vec![other.clone()])));
    ui.set_approval_in_lens(false);
    assert!(ui.get_card_shown(), "a card the conversation does not host is drawn over it");
    settle(w, width, height);
    // Deny is the left half of the card (404 wide, from x = 464 - 404 - 16 = 44).
    let deny_y = scan(w, 44.0 + 100.0, 16.0, height as f32 - 16.0, || denied.get() > 0)
        .expect("Deny on the card in the Lens's window answers a click");
    assert!(deny_y < height as f32 - 16.0, "inside the window");

    // The Lens's own card, in chat mode: the conversation hosts it, so it is not drawn twice.
    ui.set_approval_in_lens(true);
    assert!(!ui.get_card_shown(), "the Lens's own card is the conversation's");

    // Closed with a card waiting: the window is that card alone.
    ui.set_lens_open(false);
    ui.set_approval_in_lens(false);
    assert!(ui.get_card_shown(), "closed, the card is still shown");
    let card_only = settle(w, width, height);
    save(&card_only, &output.replace(".png", "-card.png"), width, height)?;
    assert_eq!(at(&card_only, 20, height / 2), blank, "no panel behind the card once the Lens is closed");
    ui.hide()?;
    println!("PASS: the Lens's own window draws the conversation between its margins, a card another caller raised over it (Deny answers at y={deny_y}), the Lens's own card once, and the card alone when the Lens is closed");
    Ok(())
}
