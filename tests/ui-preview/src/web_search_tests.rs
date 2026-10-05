//! Settings → Network → Web search, in dark and light: the built-in line, then SearXNG with its
//! description, a tested address, Save offered, and the egress button. The page must change where
//! the group sits, and Save must be drawn disabled until a test found results.

use super::*;

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 900u32);
    let ui = WebSearchProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width as usize); });
        }
        p
    };
    let save = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str| -> Result<(), Box<dyn std::error::Error>> {
        let f = BufWriter::new(File::create(path)?);
        let mut e = png::Encoder::new(f, width, height);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(p.as_bytes())?;
        Ok(())
    };
    let g = ui.global::<WebSearchState>();
    for light in [false, true] {
        ui.set_light(light);
        g.set_service("builtin".into());
        g.set_url("".into());
        g.set_test_line("".into());
        g.set_can_save(false);
        g.set_egress_line("".into());
        g.set_egress_offer(false);
        let builtin = draw();
        g.set_service("searxng".into());
        g.set_url("http://192.168.4.42:8888".into());
        g.set_test_ok(true);
        g.set_test_line("12 results from brave, duckduckgo, wikipedia. Unresponsive: google (CAPTCHA).".into());
        g.set_can_save(true);
        g.set_saved_line("Saved: SearXNG at http://192.168.4.42:8888 · 5 Oct 2026, 14:02".into());
        g.set_egress_line("The egress proxy is enforcing and no rule lets minds reach 192.168.4.42:8888 over plain http on your local network, so their searches there are refused. The button adds one rule for exactly that.".into());
        g.set_egress_offer(true);
        let searxng = draw();
        let changed = (300..height as usize)
            .flat_map(|y| (300..1200usize).map(move |x| y * width as usize + x))
            .filter(|&i| builtin.as_slice()[i] != searxng.as_slice()[i])
            .count();
        let name = if light { "light" } else { "dark" };
        let path = output.replace(".png", &format!("-{name}.png"));
        save(&searxng, &path)?;
        assert!(changed > 40_000, "the SearXNG half did not draw in {name} ({changed} pixels changed)");
        println!("PASS web search ({name}): the SearXNG half draws under Network ({changed} pixels) → {path}");
    }
    Ok(())
}
