//! Weather dashboard wiring — fetches Open-Meteo API data,
//! populates Slint models for current conditions, hourly, and daily forecasts.
//! Polls every 30 minutes and refreshes on screen open.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, WeatherCurrent, WeatherDaily, WeatherHourly};

/// Default location: London (used when no config location is available).
const DEFAULT_LAT: f64 = 51.5074;
const DEFAULT_LON: f64 = -0.1278;
const DEFAULT_LOCATION_NAME: &str = "London";

/// Shared weather data container for cross-thread communication.
#[derive(Clone, Default)]
struct WeatherData {
    current: Option<WeatherCurrent>,
    hourly: Option<Vec<WeatherHourly>>,
    daily: Option<Vec<WeatherDaily>>,
    error: Option<String>,
}

/// Wire weather dashboard callbacks and timers.
pub fn wire(ui: &App, ctx: &AppContext) {
    let data_slot: Arc<Mutex<Option<WeatherData>>> = Arc::new(Mutex::new(None));

    // ── Refresh callback ──
    let slot_for_refresh = data_slot.clone();
    let ui_weak_refresh = ui.as_weak();
    ui.on_weather_refresh(move || {
        fetch_weather_async(slot_for_refresh.clone());
        if let Some(ui) = ui_weak_refresh.upgrade() {
            ui.set_weather_current(WeatherCurrent {
                is_loading: true,
                ..ui.get_weather_current()
            });
        }
    });

    // ── Poll timer: check for results every 100ms, re-fetch every 30 min ──
    let slot_for_poll = data_slot.clone();
    let ui_weak_poll = ui.as_weak();

    // Kick off initial fetch
    fetch_weather_async(slot_for_poll.clone());

    let fetch_interval = std::cell::Cell::new(0u32); // counts 100ms ticks
    const REFETCH_TICKS: u32 = 30 * 60 * 10; // 30 min at 100ms

    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        // Check if data arrived
        {
            let mut slot = slot_for_poll.lock().unwrap();
            if let Some(data) = slot.take() {
                if let Some(ui) = ui_weak_poll.upgrade() {
                    apply_weather_data(&ui, data);
                }
            }
        }

        // Periodic re-fetch
        let count = fetch_interval.get() + 1;
        fetch_interval.set(count);
        if count >= REFETCH_TICKS {
            fetch_interval.set(0);
            fetch_weather_async(slot_for_poll.clone());
        }
    });

    std::mem::forget(poll_timer);
}

/// Apply fetched weather data to the UI.
fn apply_weather_data(ui: &App, data: WeatherData) {
    if let Some(current) = data.current {
        ui.set_weather_current(current);
    }
    if let Some(hourly) = data.hourly {
        ui.set_weather_hourly(ModelRc::new(VecModel::from(hourly)));
    }
    if let Some(daily) = data.daily {
        ui.set_weather_daily(ModelRc::new(VecModel::from(daily)));
    }
    if let Some(error) = data.error {
        let mut c = ui.get_weather_current();
        c.error_text = error.into();
        c.is_loading = false;
        ui.set_weather_current(c);
    }
}

/// Spawn a thread to fetch weather data from Open-Meteo API.
fn fetch_weather_async(slot: Arc<Mutex<Option<WeatherData>>>) {
    std::thread::spawn(move || {
        let data = fetch_weather_blocking();
        *slot.lock().unwrap() = Some(data);
    });
}

/// Blocking weather fetch using curl (available on the Alpine VM).
fn fetch_weather_blocking() -> WeatherData {
    let lat = DEFAULT_LAT;
    let lon = DEFAULT_LON;
    let location_name = DEFAULT_LOCATION_NAME.to_string();

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude={lat}&longitude={lon}\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,\
         weather_code,wind_speed_10m,wind_direction_10m,surface_pressure,\
         is_day,cloud_cover\
         &hourly=temperature_2m,weather_code,precipitation_probability\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,\
         precipitation_sum,sunrise,sunset,uv_index_max\
         &timezone=auto&forecast_days=5"
    );

    let output = match std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "15", &url])
        .output()
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            return WeatherData {
                error: Some(format!("API request failed: {}", err.trim())),
                ..Default::default()
            };
        }
        Err(e) => {
            return WeatherData {
                error: Some(format!("curl not available: {e}")),
                ..Default::default()
            };
        }
    };

    let body = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return WeatherData {
                error: Some(format!("JSON parse error: {e}")),
                ..Default::default()
            };
        }
    };

    // Parse current conditions
    let current_obj = &json["current"];
    let daily_obj = &json["daily"];

    let weather_code = current_obj["weather_code"].as_i64().unwrap_or(0) as i32;
    let is_day = current_obj["is_day"].as_i64().unwrap_or(1) == 1;
    let temp = current_obj["temperature_2m"].as_f64().unwrap_or(0.0);
    let apparent = current_obj["apparent_temperature"].as_f64().unwrap_or(0.0);
    let humidity = current_obj["relative_humidity_2m"].as_i64().unwrap_or(0);
    let wind_speed = current_obj["wind_speed_10m"].as_f64().unwrap_or(0.0);
    let wind_dir = current_obj["wind_direction_10m"].as_f64().unwrap_or(0.0);
    let pressure = current_obj["surface_pressure"].as_f64().unwrap_or(0.0);
    let cloud_cover = current_obj["cloud_cover"].as_i64().unwrap_or(0);

    // UV index from daily (today's max)
    let uv_index = daily_obj["uv_index_max"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // Sunrise/sunset from daily
    let sunrise = daily_obj["sunrise"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(|s| extract_time(s))
        .unwrap_or_else(|| "--".to_string());

    let sunset = daily_obj["sunset"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(|s| extract_time(s))
        .unwrap_or_else(|| "--".to_string());

    let current = WeatherCurrent {
        temperature: format!("{:.0}\u{00B0}C", temp).into(),
        feels_like: format!("{:.0}\u{00B0}C", apparent).into(),
        condition: wmo_description(weather_code).into(),
        icon: wmo_icon(weather_code, is_day).into(),
        location: location_name.into(),
        humidity: format!("{}%", humidity).into(),
        wind_speed: format!("{:.0} km/h", wind_speed).into(),
        wind_direction: wind_direction_str(wind_dir).into(),
        uv_index: format!("{:.0}", uv_index).into(),
        visibility: "Good".into(),
        pressure: format!("{:.0} hPa", pressure).into(),
        cloud_cover: format!("{}%", cloud_cover).into(),
        dew_point: format!("{:.0}\u{00B0}C", compute_dew_point(temp, humidity as f64)).into(),
        sunrise: sunrise.into(),
        sunset: sunset.into(),
        is_loading: false,
        error_text: "".into(),
    };

    // Parse hourly forecast (24 hours)
    let hourly_obj = &json["hourly"];
    let mut hourly = Vec::new();
    if let (Some(times), Some(temps), Some(codes)) = (
        hourly_obj["time"].as_array(),
        hourly_obj["temperature_2m"].as_array(),
        hourly_obj["weather_code"].as_array(),
    ) {
        // Determine current hour index
        let current_time_str = current_obj["time"].as_str().unwrap_or("");
        let current_hour = extract_hour(current_time_str);

        for i in 0..times.len().min(48) {
            let time_str = times[i].as_str().unwrap_or("");
            let hour = extract_hour(time_str);
            let t = temps[i].as_f64().unwrap_or(0.0);
            let code = codes[i].as_i64().unwrap_or(0) as i32;
            // Determine if this hour is daytime (rough: 6-20)
            let is_day_hour = (6..20).contains(&hour);
            let is_current = i == current_hour;

            // Only show from current hour onward, up to 24 items
            if i >= current_hour && hourly.len() < 24 {
                hourly.push(WeatherHourly {
                    time: if is_current {
                        "Now".into()
                    } else {
                        format!("{}:00", hour).into()
                    },
                    icon: wmo_icon(code, is_day_hour).into(),
                    temp: format!("{:.0}\u{00B0}", t).into(),
                    is_current,
                });
            }
        }
    }

    // Parse daily forecast
    let mut daily = Vec::new();
    if let (Some(dates), Some(codes), Some(maxes), Some(mins), Some(precips)) = (
        daily_obj["time"].as_array(),
        daily_obj["weather_code"].as_array(),
        daily_obj["temperature_2m_max"].as_array(),
        daily_obj["temperature_2m_min"].as_array(),
        daily_obj["precipitation_sum"].as_array(),
    ) {
        // Find global min/max for temperature range bars
        let mut global_min = f64::MAX;
        let mut global_max = f64::MIN;
        for i in 0..dates.len().min(5) {
            let hi = maxes[i].as_f64().unwrap_or(0.0);
            let lo = mins[i].as_f64().unwrap_or(0.0);
            if lo < global_min {
                global_min = lo;
            }
            if hi > global_max {
                global_max = hi;
            }
        }
        let range = (global_max - global_min).max(1.0);

        for i in 0..dates.len().min(5) {
            let date_str = dates[i].as_str().unwrap_or("");
            let code = codes[i].as_i64().unwrap_or(0) as i32;
            let hi = maxes[i].as_f64().unwrap_or(0.0);
            let lo = mins[i].as_f64().unwrap_or(0.0);
            let precip = precips[i].as_f64().unwrap_or(0.0);

            let day_name = if i == 0 {
                "Today".to_string()
            } else {
                day_of_week(date_str)
            };

            daily.push(WeatherDaily {
                day_name: day_name.into(),
                icon: wmo_icon(code, true).into(),
                high: format!("{:.0}\u{00B0}", hi).into(),
                low: format!("{:.0}\u{00B0}", lo).into(),
                precip_chance: if precip > 0.0 {
                    format!("{:.0}mm", precip).into()
                } else {
                    "0mm".into()
                },
                high_value: hi as f32,
                low_value: lo as f32,
                temp_range_min: ((lo - global_min) / range) as f32,
                temp_range_max: ((hi - global_min) / range) as f32,
            });
        }
    }

    WeatherData {
        current: Some(current),
        hourly: Some(hourly),
        daily: Some(daily),
        error: None,
    }
}

/// Map WMO weather code to icon string.
fn wmo_icon(code: i32, is_day: bool) -> &'static str {
    match code {
        0 => {
            if is_day {
                "\u{2600}\u{FE0F}" // sunny
            } else {
                "\u{1F319}" // crescent moon
            }
        }
        1 | 2 => {
            if is_day {
                "\u{26C5}" // sun behind cloud
            } else {
                "\u{1F319}" // moon
            }
        }
        3 => "\u{2601}\u{FE0F}",           // cloudy
        45 | 48 => "\u{1F32B}\u{FE0F}",    // fog
        51 | 53 | 55 => "\u{1F326}\u{FE0F}", // drizzle
        56 | 57 => "\u{1F327}\u{FE0F}",     // freezing drizzle
        61 | 63 => "\u{1F327}\u{FE0F}",     // rain
        65 => "\u{1F327}\u{FE0F}",          // heavy rain
        66 | 67 => "\u{1F327}\u{FE0F}",     // freezing rain
        71 | 73 => "\u{1F328}\u{FE0F}",     // snow
        75 | 77 => "\u{1F328}\u{FE0F}",     // heavy snow
        80 | 81 | 82 => "\u{1F327}\u{FE0F}", // rain showers
        85 | 86 => "\u{1F328}\u{FE0F}",     // snow showers
        95 => "\u{26C8}\u{FE0F}",           // thunderstorm
        96 | 99 => "\u{26C8}\u{FE0F}",      // thunderstorm with hail
        _ => "\u{2601}\u{FE0F}",            // default cloudy
    }
}

/// Map WMO weather code to description string.
fn wmo_description(code: i32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Depositing rime fog",
        51 => "Light drizzle",
        53 => "Moderate drizzle",
        55 => "Dense drizzle",
        56 | 57 => "Freezing drizzle",
        61 => "Slight rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Slight snow",
        73 => "Moderate snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Slight rain showers",
        81 => "Moderate rain showers",
        82 => "Violent rain showers",
        85 => "Slight snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}

/// Convert wind direction in degrees to compass string.
fn wind_direction_str(degrees: f64) -> &'static str {
    let dirs = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let idx = ((degrees + 11.25) / 22.5) as usize % 16;
    dirs[idx]
}

/// Extract time portion (HH:MM) from ISO datetime string.
fn extract_time(s: &str) -> String {
    // Format: "2026-03-06T07:30" -> "07:30"
    if let Some(pos) = s.find('T') {
        s[pos + 1..].to_string()
    } else {
        s.to_string()
    }
}

/// Extract hour index from ISO datetime string relative to day start.
fn extract_hour(s: &str) -> usize {
    // "2026-03-06T14:00" -> 14
    if let Some(pos) = s.find('T') {
        let time_part = &s[pos + 1..];
        time_part
            .split(':')
            .next()
            .and_then(|h| h.parse::<usize>().ok())
            .unwrap_or(0)
    } else {
        0
    }
}

/// Approximate dew point from temperature and humidity using Magnus formula.
fn compute_dew_point(temp_c: f64, humidity: f64) -> f64 {
    let a = 17.27;
    let b = 237.7;
    let gamma = (a * temp_c) / (b + temp_c) + (humidity / 100.0).ln();
    (b * gamma) / (a - gamma)
}

/// Extract day of week from date string "YYYY-MM-DD".
fn day_of_week(date_str: &str) -> String {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() != 3 {
        return date_str.to_string();
    }
    let year: i32 = parts[0].parse().unwrap_or(2026);
    let month: u32 = parts[1].parse().unwrap_or(1);
    let day: u32 = parts[2].parse().unwrap_or(1);

    // Zeller-like day of week calculation
    let (y, m) = if month <= 2 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let q = day as i32;
    let k = y % 100;
    let j = y / 100;
    let h = (q + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 - 2 * j) % 7;
    let h = ((h + 7) % 7) as usize;

    // h: 0=Sat, 1=Sun, 2=Mon, ...
    let names = ["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"];
    names[h].to_string()
}
