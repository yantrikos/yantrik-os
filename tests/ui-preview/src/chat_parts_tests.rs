//! ChatPill and ChatIconButton with real key and pointer events (ChatPartsProbe in
//! chat_parts_probe.slint). #1165: the conversation panel's buttons could not be reached from
//! the keyboard. Tab lands on each, Enter and Space press it (a held key does not repeat), and a
//! mouse click on one does not pull the caret out of the text field beside it.
use super::*;
use slint::platform::Key;

pub fn run(window: &MinimalSoftwareWindow, _output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ChatPartsProbe::new()?;
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(400, 120));
    let press = |k: slint::SharedString| {
        key(window, k);
        slint::platform::update_timers_and_animations();
    };

    // The text field first, then the pill, then the icon button.
    press(Key::Tab.into());
    assert!(ui.get_field_focused(), "the first Tab stop is the text field");
    press(Key::Tab.into());
    assert!(ui.get_pill_focused() && !ui.get_field_focused(), "the second is the pill");
    press(Key::Return.into());
    assert_eq!(ui.get_pill_presses(), 1, "Enter presses the pill");
    press(" ".into());
    assert_eq!(ui.get_pill_presses(), 2, "Space presses the pill");
    press(Key::Tab.into());
    assert!(ui.get_icon_focused() && !ui.get_pill_focused(), "the third is the icon button");
    press(Key::Return.into());
    press(" ".into());
    assert_eq!(ui.get_icon_presses(), 2, "Enter and Space press the icon button");
    assert_eq!(ui.get_pill_presses(), 2, "and not the pill");

    // A click on the pill presses it and leaves the caret in the field.
    click(window, 20.0, 20.0);
    slint::platform::update_timers_and_animations();
    assert!(ui.get_field_focused(), "a click in the field focuses it");
    click(window, 40.0, 76.0);
    slint::platform::update_timers_and_animations();
    assert_eq!(ui.get_pill_presses(), 3, "a click presses the pill");
    assert!(ui.get_field_focused(), "and the caret stays in the field");
    ui.hide()?;
    println!("verify-chat-parts: PASS");
    Ok(())
}
