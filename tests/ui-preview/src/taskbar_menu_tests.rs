//! The taskbar entry's right-click menu (#232), drawn by the whole shell — app.slint's `App` —
//! with real pointer and key events: a right press on a window entry asks for that window's
//! menu and the menu draws over the bar, choosing Close fires the action callback with the
//! control surface's own name for it, Escape puts the menu away, and the Menu key and Shift+F10
//! open the same menu from the keyboard. The rows are what wire/window_switcher.rs builds:
//! the control action names, with the pin row last under a divider.
use super::*;
use slint::platform::{Key, WindowEvent};
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    let win = |title: &str, app_id: &str| WindowItem {
        title: title.into(),
        app_id: app_id.into(),
        icon_char: title.chars().next().unwrap_or('W').to_string().into(),
        subtitle: "".into(),
    };
    ui.set_window_list(ModelRc::new(VecModel::from(vec![
        win("Notes: Handover", "notes"),
        win("Blender: (Unsaved) - Blender 4.3.2", "blender"),
    ])));

    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        // What the shell's own handler does, minus the pins store and the catalogue: build the
        // rows under the control surface's names and open the menu at the entry's corner.
        let weak = ui.as_weak();
        let l = log.clone();
        ui.on_window_menu(move |title, app_id, x, y| {
            l.borrow_mut().push(format!("menu:{title}:{app_id}"));
            if let Some(ui) = weak.upgrade() {
                let row = |id: &str, label: &str| YMenuAction { id: id.into(), label: label.into(), ..Default::default() };
                ui.set_taskbar_menu_actions(ModelRc::new(VecModel::from(vec![
                    YMenuAction { is_danger: true, ..row("close_window", "Close window") },
                    row("minimise_window", "Minimise"),
                    YMenuAction { separator_after: true, ..row("maximise_window", "Maximise") },
                    row("pin_app", "Pin to START"),
                ])));
                ui.set_taskbar_menu_title(title);
                ui.set_taskbar_menu_app_id(app_id);
                ui.set_taskbar_menu_x(x);
                ui.set_taskbar_menu_y(y);
                ui.set_taskbar_menu_open(true);
            }
        });
        let l = log.clone();
        ui.on_window_menu_action(move |id, title, app_id| {
            l.borrow_mut().push(format!("action:{id}:{title}:{app_id}"));
        });
        let l = log.clone();
        ui.on_switch_window(move |title| l.borrow_mut().push(format!("switch:{title}")));
    }
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        }
        pixels
    };
    draw();

    let taskbar_y = height as f32 - 20.0;

    // A right press along the bar finds the first window entry: the menu opens for the window
    // pressed. Swept, as the Apps button is found in its test, so a change of padding does not
    // break this — but starting past the Apps button and the gap after it: a right press on bare
    // bar is the desktop's to answer (its right-click TouchArea reaches under the taskbar), and
    // that second menu opening under this one would muddy what the render shows.
    let before = draw();
    let mut entry_x = None;
    for x in (100..700).step_by(6) {
        right_click(w, x as f32, taskbar_y);
        draw();
        if ui.get_taskbar_menu_open() {
            entry_x = Some(x as f32);
            break;
        }
    }
    let entry_x = entry_x.expect("a right press on a taskbar entry opens the window's menu");
    assert!(
        log.borrow().contains(&"menu:Notes: Handover:notes".to_string()),
        "the menu is asked for the window that was pressed, with its app id: {:?}",
        log.borrow()
    );
    // A right press is not a left click: the window must not also be brought forward.
    assert!(
        !log.borrow().iter().any(|e| e.starts_with("switch:")),
        "opening the menu did not activate the window: {:?}",
        log.borrow()
    );

    // The menu is on screen: over the bar, inside the screen, and drawn — the panel's pixels
    // changed against the desktop that was there.
    let after = draw();
    let menu_x = ui.get_taskbar_menu_x().min(width as f32 - 232.0);
    let menu_y = ui.get_taskbar_menu_y().min(height as f32 - 200.0);
    assert!(menu_y + 140.0 < taskbar_y + 20.0, "the clamp puts the panel above the bar, at y={menu_y}");
    let (x0, x1) = (menu_x as usize, (menu_x + 220.0) as usize);
    let (y0, y1) = (menu_y as usize, (menu_y + 137.0) as usize);
    let changed = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| y * width as usize + x))
        .filter(|&i| before.as_slice()[i] != after.as_slice()[i])
        .count();
    assert!(changed > 2_000, "the menu is drawn where the desktop was: only {changed} pixels changed");
    save(&after, output, width, height)?;

    // Choosing Close fires the action callback with the control surface's own name for it —
    // the first row, 32px tall, under the layout's 4px padding.
    click(w, menu_x + 110.0, menu_y + 20.0);
    draw();
    assert!(
        log.borrow().contains(&"action:close_window:Notes: Handover:notes".to_string()),
        "choosing Close fires the action, named as the shell action it is: {:?}",
        log.borrow()
    );
    assert!(!ui.get_taskbar_menu_open(), "and the menu is away");

    // Escape puts it away, which is the other half of "a menu you can leave".
    right_click(w, entry_x, taskbar_y);
    draw();
    assert!(ui.get_taskbar_menu_open(), "the entry's menu opens again");
    key(w, Key::Escape.into());
    draw();
    assert!(!ui.get_taskbar_menu_open(), "Escape closes the menu");

    // The Menu key on a focused entry — a left click focuses it, and a left click is also what
    // the entry has always answered, so both are asserted on the way.
    click(w, entry_x, taskbar_y);
    draw();
    assert!(
        log.borrow().iter().any(|e| e == "switch:Notes: Handover"),
        "a left click still activates the window: {:?}",
        log.borrow()
    );
    key(w, Key::Menu.into());
    draw();
    assert!(
        ui.get_taskbar_menu_open() && ui.get_taskbar_menu_title() == "Notes: Handover",
        "the Menu key opens the focused entry's menu"
    );
    save(&draw(), &output.replace(".png", "-keyboard.png"), width, height)?;
    key(w, Key::Escape.into());
    draw();

    // Shift+F10, the other name for the same press, on the second entry — found by walking
    // right until the menu that opens is Blender's.
    let mut second_x = None;
    let mut x = entry_x as i32;
    while x < 900 {
        x += 6;
        if ui.get_taskbar_menu_open() {
            key(w, Key::Escape.into());
            draw();
        }
        right_click(w, x as f32, taskbar_y);
        draw();
        if ui.get_taskbar_menu_open() && ui.get_taskbar_menu_title().starts_with("Blender") {
            second_x = Some(x as f32);
            break;
        }
    }
    let second_x = second_x.expect("the second window entry has a menu of its own");
    key(w, Key::Escape.into());
    draw();
    click(w, second_x, taskbar_y);
    draw();
    // A held Shift, the way a person presses it: the modifier state follows the dispatched
    // press and release events.
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::F10.into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::F10.into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
    draw();
    assert!(
        ui.get_taskbar_menu_open() && ui.get_taskbar_menu_title().starts_with("Blender"),
        "Shift+F10 opens the focused entry's menu"
    );
    key(w, Key::Escape.into());
    draw();

    ui.hide()?;
    println!(
        "PASS: a right press on a taskbar entry opens its window's menu without activating it; the menu draws above the bar; Close fires the control action's own name; Escape closes it; a left click still activates; the Menu key and Shift+F10 open the focused entry's menu"
    );
    Ok(())
}
