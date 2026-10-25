//! Open-Meteo, read directly: the same source the weather service reads, for when the service
//! does not answer. Moved whole from the old `main.rs`; the words and numbers are unchanged.

use std::time::Duration;

use yantrik_ipc_contracts::weather::codes;

use crate::forecast::{Aqi, Current, Day, Hour, Reading};

/// A provider that has not answered in this long is not going to.
pub const TIMEOUT: Duration = Duration::from_secs(15);

pub fn fetch(lat: f64, lon: f64, location_name: &str, use_fahrenheit: bool) -> Reading {
    let temp_unit = if use_fahrenheit { "&temperature_unit=fahrenheit" } else { "" };
    let wind_unit = if use_fahrenheit { "&wind_speed_unit=mph" } else { "" };

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude={lat}&longitude={lon}\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,\
         weather_code,wind_speed_10m,wind_direction_10m,surface_pressure,\
         is_day,cloud_cover\
         &hourly=temperature_2m,weather_code,precipitation_probability,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,\
         precipitation_sum,sunrise,sunset,uv_index_max\
         &timezone=auto&forecast_days=5{temp_unit}{wind_unit}"
    );

    let resp = match ureq::get(&url).timeout(TIMEOUT).call() {
        Ok(r) => r,
        Err(e) => return Reading::failed(format!("API request failed: {e}")),
    };
    let body: String = match resp.into_string() {
        Ok(b) => b,
        Err(e) => return Reading::failed(format!("Read error: {e}")),
    };
    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return Reading::failed(format!("JSON parse error: {e}")),
    };
    read(&json, location_name, use_fahrenheit)
}

/// The provider's JSON as a reading. Split from the request so a test can feed it a document.
pub fn read(json: &serde_json::Value, location_name: &str, use_fahrenheit: bool) -> Reading {
    let deg_symbol = if use_fahrenheit { "\u{00B0}F" } else { "\u{00B0}C" };
    let wind_label = if use_fahrenheit { "mph" } else { "km/h" };

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
    let uv_index = daily_obj["uv_index_max"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let sunrise = daily_obj["sunrise"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(extract_time)
        .unwrap_or_else(|| "--".to_string());
    let sunset = daily_obj["sunset"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(extract_time)
        .unwrap_or_else(|| "--".to_string());

    let current = Current {
        temperature: format!("{:.0}{}", temp, deg_symbol),
        feels_like: format!("{:.0}{}", apparent, deg_symbol),
        condition: codes::description(weather_code).into(),
        icon: codes::icon_kind(weather_code, is_day).into(),
        location: location_name.into(),
        humidity: format!("{}%", humidity),
        wind_speed: format!("{:.0} {}", wind_speed, wind_label),
        wind_direction: wind_direction_str(wind_dir).into(),
        uv_index: format!("{:.0}", uv_index),
        visibility: "Good".into(),
        pressure: format!("{:.0} hPa", pressure),
        cloud_cover: format!("{}%", cloud_cover),
        dew_point: String::new(),
        sunrise,
        sunset,
        is_day,
    };

    let hourly_obj = &json["hourly"];
    let mut hourly = Vec::new();
    let precip_probs = hourly_obj["precipitation_probability"].as_array();
    if let (Some(times), Some(temps), Some(codes)) = (
        hourly_obj["time"].as_array(),
        hourly_obj["temperature_2m"].as_array(),
        hourly_obj["weather_code"].as_array(),
    ) {
        let current_hour = current_obj["time"].as_str().map(extract_hour).unwrap_or(0);
        for i in 0..times.len().min(48) {
            let hour = extract_hour(times[i].as_str().unwrap_or(""));
            let t = temps[i].as_f64().unwrap_or(0.0);
            let code = codes[i].as_i64().unwrap_or(0) as i32;
            let is_day_hour = codes::hour_is_day(json, i);
            let is_current = i == current_hour;
            if i >= current_hour && hourly.len() < 24 {
                let chance = precip_probs
                    .and_then(|a| a.get(i))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                hourly.push(Hour {
                    time: if is_current { "Now".into() } else { format!("{}:00", hour) },
                    icon: codes::icon_kind(code, is_day_hour).into(),
                    temp: format!("{:.0}\u{00B0}", t),
                    precip: if chance > 0 { format!("{}%", chance) } else { String::new() },
                    is_current,
                    temp_value: t as f32,
                    t_min: 0.0,
                    t_max: 0.0,
                });
            }
        }
    }
    let h_min = hourly.iter().map(|h| h.temp_value).fold(f32::MAX, f32::min);
    let h_max = hourly.iter().map(|h| h.temp_value).fold(f32::MIN, f32::max);
    for h in hourly.iter_mut() {
        h.t_min = h_min;
        h.t_max = h_max;
    }

    let mut daily = Vec::new();
    if let (Some(dates), Some(codes), Some(maxes), Some(mins), Some(precips)) = (
        daily_obj["time"].as_array(),
        daily_obj["weather_code"].as_array(),
        daily_obj["temperature_2m_max"].as_array(),
        daily_obj["temperature_2m_min"].as_array(),
        daily_obj["precipitation_sum"].as_array(),
    ) {
        let mut g_min = f64::MAX;
        let mut g_max = f64::MIN;
        for i in 0..dates.len().min(5) {
            let hi = maxes[i].as_f64().unwrap_or(0.0);
            let lo = mins[i].as_f64().unwrap_or(0.0);
            if lo < g_min {
                g_min = lo;
            }
            if hi > g_max {
                g_max = hi;
            }
        }
        let range = (g_max - g_min).max(1.0);
        for i in 0..dates.len().min(5) {
            let date_str = dates[i].as_str().unwrap_or("");
            let code = codes[i].as_i64().unwrap_or(0) as i32;
            let hi = maxes[i].as_f64().unwrap_or(0.0);
            let lo = mins[i].as_f64().unwrap_or(0.0);
            let precip = precips[i].as_f64().unwrap_or(0.0);
            daily.push(Day {
                day_name: if i == 0 { "Today".to_string() } else { day_of_week(date_str) },
                icon: codes::icon_kind(code, true).into(),
                high: format!("{:.0}\u{00B0}", hi),
                low: format!("{:.0}\u{00B0}", lo),
                precip_chance: if precip > 0.0 { format!("{:.0}mm", precip) } else { "0mm".into() },
                high_value: hi as f32,
                low_value: lo as f32,
                temp_range_min: ((lo - g_min) / range) as f32,
                temp_range_max: ((hi - g_min) / range) as f32,
            });
        }
    }

    Reading {
        current: Some(current),
        hourly,
        daily,
        alerts: Vec::new(),
        aqi: Aqi::default(),
        error: None,
    }
}

pub fn wind_direction_str(degrees: f64) -> &'static str {
    const DIRS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    DIRS[((degrees + 11.25) / 22.5) as usize % 16]
}

pub fn extract_time(s: &str) -> String {
    match s.find('T') {
        Some(pos) => s[pos + 1..].to_string(),
        None => s.to_string(),
    }
}

pub fn extract_hour(s: &str) -> usize {
    match s.find('T') {
        Some(pos) => s[pos + 1..]
            .split(':')
            .next()
            .and_then(|h| h.parse().ok())
            .unwrap_or(0),
        None => 0,
    }
}

pub fn day_of_week(date_str: &str) -> String {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() != 3 {
        return date_str.to_string();
    }
    let year: i32 = parts[0].parse().unwrap_or(2026);
    let month: u32 = parts[1].parse().unwrap_or(1);
    let day: u32 = parts[2].parse().unwrap_or(1);
    let (y, m) = if month <= 2 { (year - 1, month + 12) } else { (year, month) };
    let q = day as i32;
    let k = y % 100;
    let j = y / 100;
    let h = (q + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 - 2 * j) % 7;
    let h = ((h + 7) % 7) as usize;
    ["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"][h].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_providers_document_is_read_as_before() {
        let json = serde_json::json!({
            "current": { "time": "2026-10-21T01:00", "temperature_2m": 12.6, "apparent_temperature": 10.1,
                         "relative_humidity_2m": 71, "weather_code": 61, "wind_speed_10m": 18.4,
                         "wind_direction_10m": 250.0, "surface_pressure": 1008.2, "is_day": 1, "cloud_cover": 90 },
            // The provider's list starts at midnight; the reader indexes it by the current hour.
            "hourly": { "time": ["2026-10-21T00:00", "2026-10-21T01:00", "2026-10-21T02:00"],
                        "temperature_2m": [12.0, 12.6, 13.1], "weather_code": [61, 61, 3],
                        "precipitation_probability": [40, 60, 10] },
            "daily": { "time": ["2026-10-21", "2026-10-22"], "weather_code": [61, 3],
                       "temperature_2m_max": [13.5, 15.0], "temperature_2m_min": [8.0, 9.0],
                       "precipitation_sum": [2.4, 0.0], "sunrise": ["2026-10-21T07:41"],
                       "sunset": ["2026-10-21T18:02"], "uv_index_max": [2.3] }
        });
        let r = read(&json, "London", false);
        let c = r.current.unwrap();
        assert_eq!((c.temperature.as_str(), c.condition.as_str(), c.wind_direction.as_str()), ("13°C", "Slight rain", "WSW"));
        assert_eq!((c.sunrise.as_str(), c.sunset.as_str(), c.uv_index.as_str()), ("07:41", "18:02", "2"));
        assert_eq!(r.hourly.len(), 2, "from the current hour on");
        assert!(r.hourly[0].is_current && r.hourly[0].time == "Now");
        assert_eq!(r.daily[0].day_name, "Today");
        assert_eq!(r.daily[1].day_name, "Thu");
        assert_eq!(r.daily[0].precip_chance, "2mm");
        assert_eq!(day_of_week("2026-10-20"), "Tue");
        assert_eq!(codes::description(95), "Thunderstorm");
        // The icons are the kit's names, not emoji the kit would draw as a cloud.
        assert_eq!(c.icon, "rain");
        assert_eq!((r.daily[0].icon.as_str(), r.daily[1].icon.as_str()), ("rain", "cloud"));
    }

    /// Bentonville at 18:00 on 21 Oct, sunset 18:32: the clear 19:00 is drawn as night, though
    /// it is before 20:00; and where the provider flags each hour, its flag is the hour's word.
    #[test]
    fn the_hourly_row_draws_night_after_sunset() {
        // The provider's day, from midnight, clear all day; the reader starts at the current hour.
        let times: Vec<String> = (0..24).map(|h| format!("2026-10-21T{h:02}:00")).collect();
        let mut json = serde_json::json!({
            "current": { "time": "2026-10-21T18:00", "weather_code": 0, "is_day": 1 },
            "hourly": { "time": times, "temperature_2m": vec![15.0; 24], "weather_code": vec![0; 24] },
            "daily": { "time": ["2026-10-21"], "weather_code": [0], "temperature_2m_max": [24.0],
                       "temperature_2m_min": [9.0], "precipitation_sum": [0.0],
                       "sunrise": ["2026-10-21T07:24"], "sunset": ["2026-10-21T18:32"], "uv_index_max": [4.0] }
        });
        let icons = |json: &serde_json::Value| -> Vec<String> {
            read(json, "Bentonville", false).hourly.iter().take(3).map(|h| format!("{} {}", h.time, h.icon)).collect()
        };
        assert_eq!(icons(&json), ["Now sun", "19:00 moon", "20:00 moon"], "by the sun's own times");
        let flags: Vec<i64> = (0..24).map(|h| i64::from((8..18).contains(&h))).collect();
        json["hourly"]["is_day"] = serde_json::json!(flags);
        assert_eq!(icons(&json), ["Now moon", "19:00 moon", "20:00 moon"], "by the provider's flag");
    }

    /// The kit's icon source, which the Weather UI's `Icons.weather-kind()` calls compile from.
    fn kit_icon_source() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/yantrik-ui-kit/slint/icon.slint");
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The names `Icons.weather-kind()` matches, read from the kit's source, each with the glyph
    /// it draws. Anything else it is handed falls through to the cloud.
    fn kit_weather_kinds(src: &str) -> Vec<(String, String)> {
        let start = src.find("public pure function weather-kind(").expect("the kit has weather-kind");
        let body = &src[start..start + src[start..].find("\n    }").expect("weather-kind ends")];
        body.lines()
            .filter_map(|line| {
                let (kind, rest) = line[line.find("kind == \"")? + 9..].split_once('"')?;
                let glyph = rest.split_once("? root.")?.1.trim();
                Some((kind.to_string(), glyph.to_string()))
            })
            .collect()
    }

    #[test]
    fn every_wmo_code_draws_a_glyph_the_kit_has_by_day_and_night() {
        let src = kit_icon_source();
        let kit = kit_weather_kinds(&src);
        let names: Vec<&str> = kit.iter().map(|(k, _)| k.as_str()).collect();
        // The two lists are one list: a kind the kit drops, or one it adds, fails here.
        assert_eq!(names, codes::ICON_KINDS, "codes::ICON_KINDS and the kit's weather-kind differ");
        for (kind, glyph) in &kit {
            assert!(src.contains(&format!("out property <string> {glyph}:")), "`{kind}` draws root.{glyph}, which the kit lacks");
        }
        for code in 0..=99 {
            for is_day in [true, false] {
                let kind = codes::icon_kind(code, is_day);
                assert!(names.contains(&kind), "WMO {code} (day {is_day}) names `{kind}`, which weather-kind does not match");
            }
        }
    }
}
