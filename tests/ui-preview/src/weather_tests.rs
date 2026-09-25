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

    assert!(problems.is_empty(), "Weather hero problems:\n{}", problems.join("\n"));
    println!("PASS: Weather hero tiles stay out from under the context rail at the default 1000x720 and return to their design width in a wide window");
    Ok(())
}
