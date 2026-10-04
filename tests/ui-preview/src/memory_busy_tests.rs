//! The Memory screen when no answer comes (VM 520, 4 October, OS ace075fd): it sat on "1049
//! stored · searching" over an empty body. The whole shell, app.slint's App, on screen 6 with
//! the wait given up (`memory-busy`): the body says the memories are busy, and a real press on
//! its Retry asks again — for the newest memories, since nothing was searched.
use super::*;
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
    ui.set_current_screen(6);
    ui.set_window_maximized(true);
    ui.set_memory_count(1049);
    ui.set_memory_busy(true);
    ui.set_is_searching_memories(false);
    let asked: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let asked = asked.clone();
        ui.on_search_memories(move |q| asked.borrow_mut().push(q.to_string()));
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
    // The window frame animates to its maximised size (200 ms); let it arrive before the picture.
    draw();
    std::thread::sleep(std::time::Duration::from_millis(400));
    save(&draw(), output, width, height)?;

    // Press along the body until something asks again. The busy state's Retry is the only
    // control in the body; the header (search field, buttons) is above y = 90.
    let mut pressed = None;
    'scan: for y in (120..760).step_by(8) {
        for x in (300..1000).step_by(16) {
            click(w, x as f32, y as f32);
            if !asked.borrow().is_empty() {
                pressed = Some((x, y));
                break 'scan;
            }
        }
    }
    let Some((x, y)) = pressed else {
        panic!("the busy Memory screen offers no Retry: nothing in its body asks again");
    };
    assert_eq!(asked.borrow().as_slice(), [String::new()], "Retry asks for the newest memories again");
    println!("  ok    Retry at ({x}, {y}) asked again for the newest memories");

    // And while a wait is still running, the body says so rather than standing empty.
    ui.set_memory_busy(false);
    ui.set_is_searching_memories(true);
    save(&draw(), &output.replace(".png", "-reading.png"), width, height)?;
    ui.hide()?;
    println!("PASS: a Memory screen that got no answer says it is busy and Retry asks again");
    Ok(())
}
