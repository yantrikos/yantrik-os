//! JumpToPresent, the kit's one transcript follower (the Lens, an agent's session, a route): it
//! follows new lines at the bottom, stays put once the reader scrolls up, says when something
//! arrived, and one press on its pill returns to the newest line.

use super::*;

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = JumpProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(480, 400));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(480, 400);
        for _ in 0..4 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), 480); });
        }
        p
    };
    draw();
    // Grows while at the bottom: it follows.
    ui.set_rows(40);
    draw();
    assert!(ui.get_following() && ui.get_at_bottom(), "a growing transcript is followed while the reader is at the bottom");
    let bottom = ui.get_list_y();
    assert!(bottom < 0.0, "and it scrolled to the newest line ({bottom})");

    // The reader scrolls up to read: it stays there, and no longer follows.
    ui.set_list_y(0.0);
    draw();
    assert!(!ui.get_following() && !ui.get_at_bottom());
    ui.set_rows(45);
    draw();
    assert_eq!(ui.get_list_y(), 0.0, "new lines do not pull the reader away from what they are reading");
    assert!(ui.get_missed(), "and the pill says something new arrived");
    let p = draw();
    {
        let f = BufWriter::new(File::create(output)?);
        let mut e = png::Encoder::new(f, 480, 400);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(p.as_bytes())?;
    }

    // The pill sits centred at the bottom: pressing it returns to the present.
    click(w, 240.0, 400.0 - 12.0 - 15.0);
    draw();
    assert!(ui.get_following() && ui.get_at_bottom() && !ui.get_missed(), "the pill returns to the newest line");
    println!("PASS jump to present: follows, stays put while reading, says what is new, returns in one press");
    Ok(())
}
