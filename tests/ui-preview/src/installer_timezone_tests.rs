use super::*;
use crate::installer_rules as rules;
use slint::{ModelRc, SharedString, VecModel};

/// The Review screen's timezone row (#1171).
///
/// The installer asks the network nothing and starts at UTC, so a click-through would leave the
/// machine hours out with nobody asked. The row therefore shows amber, with the question "Is this
/// your timezone?", the first time Review is shown, and answers it with Yes (or Change). This is
/// checked as a person sees it: the screen is drawn, the amber text (the production
/// `Theme.amber`) is found by colour, Yes is pressed by pointing at the right of the
/// question, and the screen is drawn again. Install is not gated on the answer: that is
/// `can-install`'s, checked beside this in `installer_tests`.
pub fn run(window: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let zoneinfo = std::env::temp_dir().join(format!("yos-preview-tzrow-{}", std::process::id()));
    std::fs::create_dir_all(zoneinfo.join("Europe"))?;
    std::fs::write(zoneinfo.join("Europe/Berlin"), b"TZif")?;

    let ui = InstallerProbe::new()?;
    ui.set_canvas_width(width as f32);
    ui.set_canvas_height(height as f32);
    ui.on_check_timezone(move |t| rules::timezone_problem(&t, &zoneinfo).unwrap_or_default().into());
    ui.on_is_macos_disk(|d, l| rules::disk_in_list(&d, &l));
    ui.set_keyboards(ModelRc::new(VecModel::from(Vec::<KeyboardChoice>::new())));
    ui.set_timezone("Europe/Berlin".into());
    ui.set_step(3);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(width, height));

    let draw = || {
        for _ in 0..4 {
            slint::platform::update_timers_and_animations();
            window.request_redraw();
            window.draw_if_needed(|_| {});
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(p.make_mut_slice(), width as usize);
        });
        p
    };
    let save = |name: &str, pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>| -> Result<(), Box<dyn std::error::Error>> {
        let path = output.replace(".png", &format!("-{width}x{height}-{name}.png"));
        let mut e = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(pixels.as_bytes())?;
        Ok(())
    };

    // First show: amber, with the question.
    let first = draw();
    save("review-timezone-asking", &first)?;
    let before = amber_pixels(&first, width);
    assert!(before.len() >= 100, "the unanswered timezone row is amber on first show ({} amber pixels)", before.len());
    let bottom = before.iter().map(|p| p.1).max().unwrap();
    let question: Vec<&(u32, u32)> = before.iter().filter(|p| p.1 + 20 > bottom).collect();
    assert!(question.len() >= 60, "the question \"Is this your timezone?\" sits under the zone ({} pixels)", question.len());
    let zone_row_top = before.iter().map(|p| p.1).min().unwrap();
    assert!(bottom > zone_row_top + 12, "the question is a second line, below the amber zone");

    // Yes is the link right of the question text.
    let right = question.iter().map(|p| p.0).max().unwrap() as f32;
    let middle = (question.iter().map(|p| p.1).min().unwrap() + bottom) as f32 / 2.0;
    let mut after = first.clone();
    for offset in [10.0, 16.0, 22.0, 30.0] {
        click(window, right + offset, middle);
        after = draw();
        if amber_pixels(&after, width).len() < before.len() / 2 {
            break;
        }
    }
    save("review-timezone-answered", &after)?;
    let now = amber_pixels(&after, width);
    assert!(
        now.iter().all(|p| p.1 + 20 <= bottom),
        "after Yes the question is gone ({} amber pixels remain below the zone)",
        now.iter().filter(|p| p.1 + 20 > bottom).count()
    );
    assert!(now.len() + 100 <= before.len(), "and the zone is no longer amber ({} amber pixels, were {})", now.len(), before.len());
    assert_eq!(ui.get_timezone(), SharedString::from("Europe/Berlin"), "answering Yes keeps the zone");
    println!("PASS: the installer's timezone row is amber with \"Is this your timezone?\" on first show, and both are gone after Yes");
    ui.hide()?;
    Ok(())
}

/// Where amber text is drawn: the warm, saturated orange of `Theme.amber`, which the review card
/// draws a little dimmed (a pixel of the text reads about 212, 165, 116), anti-aliasing allowed for.
fn amber_pixels(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, width: u32) -> Vec<(u32, u32)> {
    pixels
        .as_slice()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.r >= 180 && p.r as i32 - p.b as i32 >= 70 && p.g as i32 - p.b as i32 >= 25 && p.r > p.g)
        .map(|(i, _)| (i as u32 % width, i as u32 / width))
        .collect()
}
