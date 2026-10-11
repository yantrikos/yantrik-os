//! Settings → Accounts. It was four greyed-out "Unavailable" rows while the accounts a person had
//! signed in to (Claude Max on VM 520) showed only in the Minds panel, so Settings said nothing
//! about the one account the panel called active. It now draws the panel's own list.

use super::*;
use crate::minds_tests::{account, group, meter};
use slint::{ModelRc, VecModel};

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = AccountsPageProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1280, 800);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), 1280); });
        }
        p
    };
    let empty = draw();
    assert!(ui.get_shown(), "opening the page must ask for the accounts to be read again");
    ui.set_groups(ModelRc::new(VecModel::from(vec![
        group("claude", "Claude", vec![account("claude:primary", "Main", "Max 20x", "active", "", vec![meter("Today", None, "0 tokens")])]),
        group("codex", "Codex", vec![account("codex:primary", "Main", "Pro", "ready", "", vec![meter("Session", Some(0.37), "4h 42m")])]),
    ])));
    let filled = draw();
    // The list sits under the page heading, in the content column right of the category rail.
    let (x0, x1, y0, y1) = (300, 1200, 200, 420);
    let changed = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| y * 1280 + x))
        .filter(|&i| empty.as_slice()[i] != filled.as_slice()[i])
        .count();
    let f = BufWriter::new(File::create(output)?);
    let mut e = png::Encoder::new(f, 1280, 800);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(filled.as_bytes())?;
    assert!(changed > 15_000, "the accounts did not draw on Settings → Accounts ({changed} pixels changed)");
    println!("PASS accounts page: the panel's accounts drawn in Settings ({changed} pixels), refreshed on open");
    Ok(())
}
