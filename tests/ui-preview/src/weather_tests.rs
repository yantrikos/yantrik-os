use super::*;

/// The weather hero at the app's own default window (issue #220): at 1000×720 with the
/// context rail open, the UV index and cloud cover tiles used to run under the rail
/// because the tile grid had a fixed 499px design width that could not shrink. The grid
/// now shrinks inside the card at the default size and returns to its design width once
/// the window is wide enough to hold it.
pub fn run(window: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    let ui = WeatherProbe::new()?;
    ui.set_current(WeatherCurrent {
        temperature: "21°".into(),
        feels_like: "19°".into(),
        condition: "Partly cloudy".into(),
        icon: "partly".into(),
        location: "Fixture Bay".into(),
        humidity: "92%".into(),
        wind_speed: "13 km/h".into(),
        wind_direction: "SW".into(),
        uv_index: "4".into(),
        visibility: "9 km".into(),
        pressure: "1020 hPa".into(),
        cloud_cover: "64%".into(),
        dew_point: "18°".into(),
        sunrise: "06:12".into(),
        sunset: "18:40".into(),
        is_day: true,
        is_loading: false,
        error_text: "".into(),
    });
    // The running app hands the rail today's context, so the rail is open — 280px — at
    // the default window size. That is exactly the situation the tiles used to run under.
    ui.set_agent_context(slint::ModelRc::new(slint::VecModel::from(vec![
        AgentContextItem {
            id: "ctx-today".into(),
            label: "Today".into(),
            detail: "Partly cloudy, 21°".into(),
            source: "calendar".into(),
        },
    ])));
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

    // Both edges are absolute: the tile grid's right edge against the right edge of the
    // space the rail leaves. Measuring the tiles against their own row would not see the
    // defect — the row itself overflows the card when the grid cannot shrink.
    let space = ui.get_content_space_right();
    let right = ui.get_hero_tiles_abs_right();
    if right > space {
        problems.push(format!(
            "at 1000x720 the stat tiles end at {right}px, past the {space}px the open context rail leaves: the UV index and cloud cover tiles are under the rail"
        ));
    }
    let tiles = ui.get_hero_tiles_width();
    if tiles < 256.0 {
        problems.push(format!(
            "the tile grid shrank to {tiles}px: three tiles need at least 256px between them to stay readable"
        ));
    }

    // A window that can hold the design grid gets it back: the tiles stop shrinking and
    // return to their 161px ceiling each (3×161 + 2×8 = 499).
    ui.set_canvas_width(1400.);
    ui.set_canvas_height(900.);
    window.set_size(slint::PhysicalSize::new(1400, 900));
    settle(1400, 900);
    let tiles_wide = ui.get_hero_tiles_width();
    if tiles_wide < 490.0 || tiles_wide > 500.0 {
        problems.push(format!(
            "at 1400 wide the tile grid is {tiles_wide}px; with room to spare it should return to its 499px design width"
        ));
    }
    if ui.get_hero_tiles_abs_right() > ui.get_content_space_right() {
        problems.push(format!(
            "at 1400 wide the stat tiles end at {}px, past the {}px of content space",
            ui.get_hero_tiles_abs_right(),
            ui.get_content_space_right()
        ));
    }

    // The hourly strip reads in the forecast location's local time, not the machine's, so the
    // section says whose time it is (#220). The label renders when the app supplies one and
    // draws nothing — zero width — when it does not.
    ui.set_hourly_tz_label("Fixture Bay time".into());
    settle(1400, 900);
    if ui.get_hourly_tz_width() <= 0.0 {
        problems.push(
            "the hourly section did not render its \"Fixture Bay time\" label even though one was set".into(),
        );
    }
    ui.set_hourly_tz_label("".into());
    settle(1400, 900);
    if ui.get_hourly_tz_width() > 0.0 {
        problems.push("the hourly section drew a timezone label with none set".into());
    }

    assert!(problems.is_empty(), "Weather hero problems:\n{}", problems.join("\n"));
    println!("PASS: Weather hero tiles stay out from under the context rail at the default 1000x720 and return to their design width in a wide window");
    Ok(())
}

fn fixture(ui: &WeatherProbe) {
    ui.set_current(WeatherCurrent {
        temperature: "22°".into(),
        feels_like: "23°".into(),
        condition: "Partly cloudy".into(),
        icon: "partly".into(),
        location: "London".into(),
        humidity: "76%".into(),
        wind_speed: "11 km/h".into(),
        wind_direction: "SE".into(),
        uv_index: "3".into(),
        visibility: "Excellent".into(),
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
            precip: format!("{}%", (i * 7) % 40).into(),
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
            precip_chance: "1%".into(),
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

/// Weather snapped to the layouts labwc offers (#505): a half, a quarter, a third. Every stat
/// tile must stay wide enough to read its label and value, and nothing may run past the window.
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
        let tile = ui.get_hero_tile_width();
        if tile < 110.0 {
            problems.push(format!("{name} ({w}x{h}): a stat tile is {tile}px wide; below 110 its label and value do not fit"));
        }
        if ui.get_hero_tiles_abs_right() > ui.get_content_space_right() + 0.5 {
            problems.push(format!("{name} ({w}x{h}): the tiles run past the content"));
        }
        println!("{name} {w}x{h}: tile {tile:.0}px, rail {}", if ui.get_rail_shown() { "shown" } else { "hidden" });
    }
    assert!(problems.is_empty(), "Weather at snapped sizes:\n{}", problems.join("\n"));
    println!("PASS: Weather reads at a half, a quarter and a third of the screen");
    Ok(())
}
