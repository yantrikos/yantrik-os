//! Calendar at its real size with a full day. On VM 520 a day with 30 events stretched the
//! month grid's rows to ~300px each, so two weeks filled the window and the rest of the month
//! was below the bottom edge.

use super::*;

fn days() -> slint::ModelRc<CalendarDay> {
    // September 2026 starts on a Tuesday: two leading blanks, 30 days, blanks to 42.
    let rows: Vec<CalendarDay> = (0..42)
        .map(|i| {
            let n = i as i32 - 1;
            let real = (1..=30).contains(&n);
            CalendarDay {
                day_number: if real { n } else { 0 },
                is_today: n == 30,
                is_selected: n == 30,
                is_current_month: real,
                has_events: n == 30,
                event_count: if n == 30 { 30 } else { 0 },
            }
        })
        .collect();
    Rc::new(slint::VecModel::from(rows)).into()
}

fn events(n: usize) -> slint::ModelRc<CalendarEvent> {
    let rows: Vec<CalendarEvent> = (0..n)
        .map(|i| CalendarEvent {
            id: i as i32,
            title: format!("Load test {i}").into(),
            date_text: "Sep 30, 2026".into(),
            time_text: "08:00 – 08:30".into(),
            color: slint::Color::from_rgb_u8(0x2d, 0xd4, 0xbf),
            is_all_day: false,
        })
        .collect();
    Rc::new(slint::VecModel::from(rows)).into()
}

fn context(n: usize) -> slint::ModelRc<AgentContextItem> {
    let rows: Vec<AgentContextItem> = (0..n)
        .map(|i| AgentContextItem {
            id: format!("e{i}").into(),
            label: format!("Load test {i}").into(),
            detail: "08:00 – 08:30".into(),
            source: "calendar".into(),
        })
        .collect();
    Rc::new(slint::VecModel::from(rows)).into()
}

fn save(p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let f = BufWriter::new(File::create(path)?);
    let mut e = png::Encoder::new(f, 1100, 720);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = CalendarProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1100, 720));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1100, 720);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), 1100); });
        }
        p
    };
    ui.set_days(days());
    let stem = output.trim_end_matches(".png");
    for (label, n_events, n_context, notice) in [
        ("empty", 0, 0, ""),
        ("events", 30, 0, ""),
        ("rail", 0, 30, ""),
        ("both", 30, 30, "Kept “Load test 0”. refuse.py asked to delete it, but on its own it may only delete events it made. Delete it yourself if you want it gone."),
    ] {
        ui.set_events(events(n_events));
        ui.set_context(context(n_context));
        ui.set_notice(notice.into());
        // A refusal is about one event, so it draws as information with "Open event".
        ui.set_notice_event(if notice.is_empty() { "" } else { "evt-load-0" }.into());
        let p = draw();
        save(&p, &format!("{stem}-{label}.png"))?;
        println!("{label}: screen {}px", ui.get_screen_h());
    }
    Ok(())
}
