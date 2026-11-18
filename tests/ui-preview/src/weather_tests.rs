use super::*;

/// The Weather screen at the app's own default window (issue #220): at 1000×720 with the
/// context rail open the content is 720px wide, which holds three measured pairs to a row; a
/// window wide enough never gets fewer. The old check watched a tile grid run under the rail;
/// the redesign has no grid to run, so the check is that the pairs keep three columns there,
/// that the content keeps its width, and that the rail is open.
pub fn run(window: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    let ui = WeatherProbe::new()?;
    fixture(&ui);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1000, 720));
    let settle = |w: u32, h: u32| {
        for _ in 0..8 {
            slint::platform::update_timers_and_animations();
            let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(w, h);
            window.request_redraw();
            window.draw_if_needed(|r| { r.render(p.make_mut_slice(), w as usize); });
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    };
    settle(1000, 720);
    let mut problems: Vec<String> = vec![];

    if !ui.get_rail_shown() {
        problems.push("at 1000x720 the context rail is hidden; it has content and the window is wide enough".into());
    }
    let columns = ui.get_pairs_columns();
    if columns != 3 {
        problems.push(format!("at 1000x720 with the rail open the measured pairs are {columns} to a row; the 720px of content holds three"));
    }
    let content = ui.get_content_width();
    if !(700.0..=740.0).contains(&content) {
        problems.push(format!("at 1000x720 the content is {content}px wide; the rail leaves 720"));
    }

    // A wide window keeps three columns and the rail.
    ui.set_canvas_width(1400.0);
    window.set_size(slint::PhysicalSize::new(1400, 720));
    settle(1400, 720);
    if ui.get_pairs_columns() != 3 || !ui.get_rail_shown() {
        problems.push("at 1400 wide the pairs or the rail changed; nothing should".into());
    }

    assert!(problems.is_empty(), "Weather at the default window:\n{}", problems.join("\n"));
    println!("PASS: Weather keeps three measured pairs to a row beside the open rail at 1000x720 and at 1400");
    Ok(())
}

fn fixture(ui: &WeatherProbe) {
    ui.set_current(WeatherCurrent {
        temperature: "22°C".into(),
        feels_like: "23°C".into(),
        condition: "Partly cloudy".into(),
        icon: "partly".into(),
        location: "London".into(),
        humidity: "76%".into(),
        wind_speed: "11 km/h".into(),
        wind_direction: "SE".into(),
        uv_index: "3".into(),
        visibility: "16 km".into(),
        pressure: "1009 hPa".into(),
        cloud_cover: "53%".into(),
        dew_point: "17°".into(),
        sunrise: "06:52".into(),
        sunset: "18:41".into(),
        is_day: false,
        is_loading: false,
        error_text: "".into(),
    });
    let hours: Vec<WeatherHourly> = (0..12)
        .map(|i| WeatherHourly {
            time: if i == 0 { "Now".into() } else { format!("{:02}:00", (21 + i) % 24).into() },
            icon: "partly".into(),
            temp: "22°".into(),
            precip: if (i * 7) % 40 >= 10 { format!("{}%", (i * 7) % 40).into() } else { "".into() },
            is_current: i == 0,
            temp_value: 22.0 - (i as f32) * 0.2,
            t_min: 19.0,
            t_max: 23.0,
        })
        .collect();
    ui.set_hourly(slint::ModelRc::new(slint::VecModel::from(hours)));
    let days: Vec<WeatherDaily> = ["Today", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .enumerate()
        .map(|(i, d)| WeatherDaily {
            day_name: (*d).into(),
            icon: "rain".into(),
            high: format!("{}°", 26 - i).into(),
            low: format!("{}°", 16 - i % 2).into(),
            precip_chance: if i % 2 == 0 { "2.4mm".into() } else { "".into() },
            high_value: 26.0 - i as f32,
            low_value: 16.0,
            temp_range_min: 0.2,
            temp_range_max: 0.9 - i as f32 * 0.1,
        })
        .collect();
    ui.set_daily(slint::ModelRc::new(slint::VecModel::from(days)));
    ui.set_agent_context(slint::ModelRc::new(slint::VecModel::from(vec![AgentContextItem {
        id: "ctx-now".into(),
        label: "22°C, Partly cloudy".into(),
        detail: "23°C".into(),
        source: "file".into(),
    }])));
}

/// Weather snapped to the layouts labwc offers (#505): a half, a quarter, a third. The measured
/// pairs fall to two a row where three would not fit, the rail steps aside under 900px, and the
/// content keeps the window's width. One still per size.
pub fn snapped(window: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = WeatherProbe::new()?;
    fixture(&ui);
    ui.show()?;
    let mut problems = Vec::new();
    for (name, w, h) in [("half", 640u32, 728u32), ("quarter", 640, 364), ("third", 426, 728), ("default", 1000, 720), ("half-whole", 640, 2000), ("third-whole", 426, 2400)] {
        ui.set_canvas_width(w as f32);
        ui.set_canvas_height(h as f32);
        window.set_size(slint::PhysicalSize::new(w, h));
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(w, h);
        for _ in 0..6 {
            slint::platform::update_timers_and_animations();
            p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(w, h);
            window.request_redraw();
            window.draw_if_needed(|r| { r.render(p.make_mut_slice(), w as usize); });
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        let path = output.replace(".png", &format!("-{name}.png"));
        let f = BufWriter::new(File::create(&path)?);
        let mut e = png::Encoder::new(f, w, h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(p.as_bytes())?;
        let columns = ui.get_pairs_columns();
        let rail = ui.get_rail_shown();
        let expect_rail = w >= 900;
        let room = w as f32 - if expect_rail { 280.0 } else { 0.0 };
        let expect_columns = if room < 560.0 + 48.0 { 2 } else { 3 };
        if rail != expect_rail {
            problems.push(format!("{name} ({w}x{h}): the rail is {}; at this width it should be {}", if rail { "shown" } else { "hidden" }, if expect_rail { "shown" } else { "hidden" }));
        }
        if columns != expect_columns {
            problems.push(format!("{name} ({w}x{h}): {columns} pairs to a row; {expect_columns} fit"));
        }
        let content = ui.get_content_width();
        if (content - room).abs() > 1.0 {
            problems.push(format!("{name} ({w}x{h}): the content is {content}px wide, not the {room}px the window leaves"));
        }
        println!("{name} {w}x{h}: {columns} pairs a row, content {content:.0}px, rail {}", if rail { "shown" } else { "hidden" });
    }
    assert!(problems.is_empty(), "Weather at snapped sizes:\n{}", problems.join("\n"));
    println!("PASS: Weather reads at a half, a quarter and a third of the screen");
    Ok(())
}
