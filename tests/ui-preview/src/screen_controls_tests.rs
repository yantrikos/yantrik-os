//! Every screen the shell draws in a window frame can be left by its own controls (#231).
//!
//! On VM 520 the shell's editor screen filled the space between the bars with no minimise,
//! maximise or close in sight, so once it was open there was no way off it but the taskbar. The
//! frame hides its own title bar when a screen says it draws the controls itself, and the mind
//! panel's strip takes the right edge where controls sit. This presses along the header row of
//! each framed screen, maximised, from just left of the panel's strip leftwards, and requires a
//! press that takes the person off the screen — close, or minimise — before the middle of it.
use super::*;

/// The screens app.slint draws inside a WindowFrame (`if current-screen == N : WindowFrame`).
/// Images, Editor and Media were here until #253 made each of them a window of its own.
const FRAMED: &[(i32, &str)] = &[
    (4, "Interaction history"), (5, "Personality"), (6, "Memory"), (7, "Settings"), (8, "Files"),
    (9, "Notifications"), (10, "System"), (16, "About"), (21, "Packages"), (27, "Devices"), (28, "Permissions"), (33, "Problems"),
    (34, "Agents"), (35, "Recipes"),
];

pub fn run(w: &MinimalSoftwareWindow, _output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        }
    };

    let mut stuck = Vec::new();
    // Maximised, the header row is just under the status bar and ends at the mind panel's strip.
    // Not maximised, the frame sits at its default place (80, 40), 85% of the width across.
    let unmax_right = 80.0 + width as f32 * 0.85 - 10.0;
    let states: [(bool, &str, [f32; 3], f32); 2] = [
        (true, "maximised", [44.0, 52.0, 60.0], width as f32 - 50.0),
        (false, "not maximised", [52.0, 60.0, 70.0], unmax_right),
    ];
    for &(screen, name) in FRAMED {
        for &(maximised, how, rows, right) in &states {
            let mut left = None;
            'search: for y in rows {
                let mut x = right;
                while x > width as f32 / 2.0 {
                    ui.set_current_screen(screen);
                    ui.set_window_maximized(maximised);
                    draw();
                    click(w, x, y);
                    draw();
                    if ui.get_current_screen() != screen {
                        left = Some((x, y, ui.get_current_screen()));
                        break 'search;
                    }
                    x -= 6.0;
                }
            }
            match left {
                Some((x, y, to)) => println!("  ok    {name} ({screen}), {how}: left for screen {to} by a press at ({x}, {y})"),
                None => {
                    println!("  STUCK {name} ({screen}), {how}: no press in its header row takes the person off it");
                    stuck.push(format!("{name} ({how})"));
                }
            }
        }
    }
    assert!(stuck.is_empty(), "screens with no reachable close or minimise: {stuck:?}");
    println!("PASS: every framed screen, maximised or not, can be left by a control in its header row");
    Ok(())
}
