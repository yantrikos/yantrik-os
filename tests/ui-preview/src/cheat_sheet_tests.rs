//! The keyboard cheat sheet (story 4.2) drawn by the real Slint screens, with the rows the
//! shell generates from the shipped rc.xml (the production `cheat_sheet.rs`, compiled in here).
//! Checks it is drawn over a screen, that a search narrows it and that closing it restores the
//! frame that was there before.
use super::*;

#[path = "../../../crates/yantrik-ui/src/cheat_sheet.rs"]
#[allow(dead_code)]
mod cheat_sheet;

fn model(query: &str) -> slint::ModelRc<CheatRowData> {
    let all = cheat_sheet::shipped();
    let mut out = Vec::new();
    let mut group = "";
    for row in cheat_sheet::matching(&all, query) {
        if row.group != group {
            group = row.group;
            out.push(CheatRowData { heading: true, text: group.into(), caps: slint::ModelRc::default() });
        }
        let caps: Vec<slint::SharedString> = row.caps.iter().map(|c| c.as_str().into()).collect();
        out.push(CheatRowData { heading: false, text: row.text.as_str().into(), caps: slint::ModelRc::new(slint::VecModel::from(caps)) });
    }
    slint::ModelRc::new(slint::VecModel::from(out))
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}x{height})");
    Ok(())
}

fn differing(a: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, b: &slint::SharedPixelBuffer<slint::Rgb8Pixel>) -> usize {
    a.as_slice().iter().zip(b.as_slice()).filter(|(x, y)| x != y).count()
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        }
        pixels
    };
    let park = || w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(20.0, 400.0) });
    park();
    let before = draw();

    ui.set_cheat_sheet_rows(model(""));
    ui.set_cheat_sheet_open(true);
    let open = draw();
    let drawn = differing(&before, &open);
    assert!(drawn > 100_000, "the sheet is drawn over the screen: only {drawn} pixels changed");
    save(&open, output, width, height)?;

    ui.set_cheat_sheet_rows(model("snap"));
    let searched = draw();
    assert!(differing(&open, &searched) > 5_000, "a search changes what is listed");
    save(&searched, &output.replace(".png", "-search.png"), width, height)?;

    // A click outside, on the backdrop, puts it away and the frame is the one from before.
    click(w, 20.0, 400.0);
    let after = draw();
    assert!(!ui.get_cheat_sheet_open(), "a click outside closes the sheet");
    assert_eq!(differing(&before, &after), 0, "closed, the frame is the one that was there");

    // Esc closes it too, with the search field holding the keyboard.
    ui.set_cheat_sheet_open(true);
    draw();
    key(w, slint::platform::Key::Escape.into());
    draw();
    assert!(!ui.get_cheat_sheet_open(), "Esc closes the sheet");
    println!("PASS: the cheat sheet draws over the desktop, narrows on search and closes on Esc or a click outside");
    Ok(())
}
