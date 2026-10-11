//! The context rail (the kit's AgentRail, beside every app): a long context list scrolls inside
//! the rail. It used to be the rail's minimum height, which a layout passes up — a calendar day
//! with 41 events laid the Calendar window out two thousand pixels tall and stretched its month
//! grid across the screen with most of the month below the bottom edge.

use super::*;

fn items(n: usize) -> slint::ModelRc<AgentContextItem> {
    let rows: Vec<AgentContextItem> = (0..n)
        .map(|i| AgentContextItem {
            id: format!("e{i}").into(),
            label: format!("Arena event {i}").into(),
            detail: "15:00 – 15:30".into(),
            source: "calendar".into(),
        })
        .collect();
    Rc::new(slint::VecModel::from(rows)).into()
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = RailProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(900, 500));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(900, 500);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), 900); });
        }
        p
    };
    let next: Vec<AgentSuggestion> = vec![AgentSuggestion {
        id: "shape".into(),
        label: "What does this day look like?".into(),
        detail: "shape of the day".into(),
        icon: "sparkle".into(),
        running: false,
        proposes: false,
    }];
    ui.set_next(Rc::new(slint::VecModel::from(next)).into());

    ui.set_context(items(3));
    draw();
    assert_eq!(ui.get_body_h(), 460.0, "a short list: the app body is the window less its header");

    ui.set_context(items(41));
    let p = draw();
    assert_eq!(ui.get_body_h(), 460.0, "41 rows scroll in the rail; the app body stays the window's height");
    assert_eq!(ui.get_rail_h(), 460.0, "and the rail is no taller than the body beside it");
    {
        let f = BufWriter::new(File::create(output)?);
        let mut e = png::Encoder::new(f, 900, 500);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(p.as_bytes())?;
    }
    println!("PASS context rail: a long list scrolls inside it and never stretches the window");
    Ok(())
}
