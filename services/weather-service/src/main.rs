//! Weather service — standalone process exposing Open-Meteo data via JSON-RPC.
//!
//! Methods:
//!   weather.current   { lat, lon, fahrenheit? }  → CurrentWeather
//!   weather.hourly    { lat, lon, hours?, fahrenheit? }  → Vec<HourlyForecast>
//!   weather.daily     { lat, lon, days?, fahrenheit? }   → Vec<DailyForecast>
//!   weather.alerts    { lat, lon, fahrenheit? }  → Vec<WeatherAlert>
//!   weather.air_quality { lat, lon }             → AirQuality
//!   weather.geocode   { query }                  → Location

use yantrik_ipc_contracts::weather::*;
use yantrik_service_sdk::prelude::*;

fn main() {
    ServiceBuilder::new("weather")
        .handler(WeatherHandler)
        .run();
}

struct WeatherHandler;

impl ServiceHandler for WeatherHandler {
    fn service_id(&self) -> &str {
        "weather"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "weather.current" => {
                let loc = parse_location(&params)?;
                let fahrenheit = params["fahrenheit"].as_bool().unwrap_or(false);
                let result = fetch_current(&loc, fahrenheit)?;
                Ok(serde_json::to_value(result).unwrap())
            }
            "weather.hourly" => {
                let loc = parse_location(&params)?;
                let hours = params["hours"].as_u64().unwrap_or(24) as u32;
                let fahrenheit = params["fahrenheit"].as_bool().unwrap_or(false);
                let result = fetch_hourly(&loc, hours, fahrenheit)?;
                Ok(serde_json::to_value(result).unwrap())
            }
            "weather.daily" => {
                let loc = parse_location(&params)?;
                let days = params["days"].as_u64().unwrap_or(5) as u32;
                let fahrenheit = params["fahrenheit"].as_bool().unwrap_or(false);
                let result = fetch_daily(&loc, days, fahrenheit)?;
                Ok(serde_json::to_value(result).unwrap())
            }
            "weather.alerts" => {
                let loc = parse_location(&params)?;
                let fahrenheit = params["fahrenheit"].as_bool().unwrap_or(false);
                let result = fetch_alerts(&loc, fahrenheit)?;
                Ok(serde_json::to_value(result).unwrap())
            }
            "weather.air_quality" => {
                let loc = parse_location(&params)?;
                let result = fetch_air_quality(&loc)?;
                Ok(serde_json::to_value(result).unwrap())
            }
            "weather.geocode" => {
                let query = params["query"]
                    .as_str()
                    .ok_or_else(|| ServiceError {
                        code: -32602,
                        message: "Missing 'query' parameter".to_string(),
                    })?;
                let result = geocode(query)?;
                Ok(serde_json::to_value(result).unwrap())
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
}

// ── Parameter parsing ────────────────────────────────────────────────

fn parse_location(params: &serde_json::Value) -> Result<Location, ServiceError> {
    let lat = params["lat"]
        .as_f64()
        .ok_or_else(|| ServiceError {
            code: -32602,
            message: "Missing 'lat' parameter".to_string(),
        })?;
    let lon = params["lon"]
        .as_f64()
        .ok_or_else(|| ServiceError {
            code: -32602,
            message: "Missing 'lon' parameter".to_string(),
        })?;
    let name = params["name"].as_str().unwrap_or("Unknown").to_string();
    Ok(Location { name, lat, lon })
}

// ── Open-Meteo API fetching ──────────────────────────────────────────

fn open_meteo_forecast(
    loc: &Location,
    fahrenheit: bool,
) -> Result<serde_json::Value, ServiceError> {
    let temp_unit = if fahrenheit { "&temperature_unit=fahrenheit" } else { "" };
    let wind_unit = if fahrenheit { "&wind_speed_unit=mph" } else { "" };

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude={}&longitude={}\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,\
         weather_code,wind_speed_10m,wind_direction_10m,surface_pressure,\
         is_day,cloud_cover\
         &hourly=temperature_2m,weather_code,precipitation_probability\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,\
         precipitation_sum,sunrise,sunset,uv_index_max\
         &timezone=auto&forecast_days=7{temp_unit}{wind_unit}",
        loc.lat, loc.lon
    );

    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("Open-Meteo API error: {e}"),
        })?;

    resp.into_json::<serde_json::Value>().map_err(|e| ServiceError {
        code: -32000,
        message: format!("JSON parse error: {e}"),
    })
}

fn fetch_current(loc: &Location, fahrenheit: bool) -> Result<CurrentWeather, ServiceError> {
    let json = open_meteo_forecast(loc, fahrenheit)?;
    let c = &json["current"];
    let d = &json["daily"];

    let weather_code = c["weather_code"].as_i64().unwrap_or(0) as i32;
    let is_day = c["is_day"].as_i64().unwrap_or(1) == 1;

    let uv_index = d["uv_index_max"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    Ok(CurrentWeather {
        temperature: c["temperature_2m"].as_f64().unwrap_or(0.0),
        feels_like: c["apparent_temperature"].as_f64().unwrap_or(0.0),
        humidity: c["relative_humidity_2m"].as_i64().unwrap_or(0) as i32,
        wind_speed: c["wind_speed_10m"].as_f64().unwrap_or(0.0),
        wind_direction: wind_direction_str(c["wind_direction_10m"].as_f64().unwrap_or(0.0))
            .to_string(),
        condition: wmo_description(weather_code).to_string(),
        icon: wmo_icon(weather_code, is_day).to_string(),
        uv_index,
        visibility_km: 10.0, // Open-Meteo free tier doesn't provide visibility
        pressure_hpa: c["surface_pressure"].as_f64().unwrap_or(0.0),
    })
}

fn fetch_hourly(
    loc: &Location,
    hours: u32,
    fahrenheit: bool,
) -> Result<Vec<HourlyForecast>, ServiceError> {
    let json = open_meteo_forecast(loc, fahrenheit)?;
    let h = &json["hourly"];

    let (times, temps, codes) = match (
        h["time"].as_array(),
        h["temperature_2m"].as_array(),
        h["weather_code"].as_array(),
    ) {
        (Some(t), Some(te), Some(c)) => (t, te, c),
        _ => return Ok(Vec::new()),
    };

    let precip_probs = h["precipitation_probability"].as_array();

    let current_hour = json["current"]["time"]
        .as_str()
        .map(extract_hour)
        .unwrap_or(0);

    let mut result = Vec::new();
    for i in current_hour..times.len().min(current_hour + hours as usize) {
        let time_str = times[i].as_str().unwrap_or("");
        let hour = extract_hour(time_str);
        let code = codes[i].as_i64().unwrap_or(0) as i32;
        let is_day_hour = (6..20).contains(&hour);
        let precip = precip_probs
            .and_then(|p| p.get(i))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        result.push(HourlyForecast {
            time: if i == current_hour {
                "Now".to_string()
            } else {
                format!("{:02}:00", hour)
            },
            temperature: temps[i].as_f64().unwrap_or(0.0),
            condition: wmo_description(code).to_string(),
            icon: wmo_icon(code, is_day_hour).to_string(),
            precipitation_chance: precip,
        });
    }

    Ok(result)
}

fn fetch_daily(
    loc: &Location,
    days: u32,
    fahrenheit: bool,
) -> Result<Vec<DailyForecast>, ServiceError> {
    let json = open_meteo_forecast(loc, fahrenheit)?;
    let d = &json["daily"];

    let (dates, codes, maxes, mins, precips, sunrises, sunsets) = match (
        d["time"].as_array(),
        d["weather_code"].as_array(),
        d["temperature_2m_max"].as_array(),
        d["temperature_2m_min"].as_array(),
        d["precipitation_sum"].as_array(),
        d["sunrise"].as_array(),
        d["sunset"].as_array(),
    ) {
        (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f), Some(g)) => (a, b, c, d, e, f, g),
        _ => return Ok(Vec::new()),
    };

    let mut result = Vec::new();
    for i in 0..dates.len().min(days as usize) {
        let code = codes[i].as_i64().unwrap_or(0) as i32;
        let precip = precips[i].as_f64().unwrap_or(0.0);

        result.push(DailyForecast {
            date: dates[i].as_str().unwrap_or("").to_string(),
            temp_high: maxes[i].as_f64().unwrap_or(0.0),
            temp_low: mins[i].as_f64().unwrap_or(0.0),
            condition: wmo_description(code).to_string(),
            icon: wmo_icon(code, true).to_string(),
            precipitation_chance: if precip > 0.0 { (precip * 10.0) as i32 } else { 0 },
            sunrise: sunrises[i]
                .as_str()
                .map(extract_time)
                .unwrap_or_else(|| "--".to_string()),
            sunset: sunsets[i]
                .as_str()
                .map(extract_time)
                .unwrap_or_else(|| "--".to_string()),
        });
    }

    Ok(result)
}

fn fetch_alerts(loc: &Location, fahrenheit: bool) -> Result<Vec<WeatherAlert>, ServiceError> {
    let json = open_meteo_forecast(loc, fahrenheit)?;
    let c = &json["current"];
    let d = &json["daily"];

    let weather_code = c["weather_code"].as_i64().unwrap_or(0) as i32;
    let wind_speed = c["wind_speed_10m"].as_f64().unwrap_or(0.0);
    let uv_index = d["uv_index_max"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    Ok(generate_alerts(weather_code, wind_speed, uv_index, fahrenheit))
}

fn fetch_air_quality(loc: &Location) -> Result<AirQuality, ServiceError> {
    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?\
         latitude={}&longitude={}&current=european_aqi",
        loc.lat, loc.lon
    );

    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("AQI API error: {e}"),
        })?;

    let json: serde_json::Value = resp.into_json().map_err(|e| ServiceError {
        code: -32000,
        message: format!("AQI JSON error: {e}"),
    })?;

    let aqi = json["current"]["european_aqi"]
        .as_f64()
        .unwrap_or(-1.0) as i32;

    if aqi < 0 {
        return Ok(AirQuality {
            value: 0.0,
            label: "N/A".to_string(),
            level: 0,
        });
    }

    let (label, level) = match aqi {
        0..=20 => ("Good", 0),
        21..=40 => ("Fair", 0),
        41..=60 => ("Moderate", 1),
        61..=80 => ("Poor", 1),
        81..=100 => ("Very Poor", 2),
        _ => ("Hazardous", 2),
    };

    Ok(AirQuality {
        value: aqi as f64,
        label: label.to_string(),
        level,
    })
}

fn geocode(query: &str) -> Result<Location, ServiceError> {
    let encoded = query.replace(' ', "+");
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={encoded}&count=1&language=en&format=json"
    );

    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| ServiceError {
            code: -32000,
            message: format!("Geocoding error: {e}"),
        })?;

    let json: serde_json::Value = resp.into_json().map_err(|e| ServiceError {
        code: -32000,
        message: format!("Geocoding JSON error: {e}"),
    })?;

    let results = json["results"]
        .as_array()
        .ok_or_else(|| ServiceError {
            code: -32000,
            message: "No geocoding results".to_string(),
        })?;

    let first = results.first().ok_or_else(|| ServiceError {
        code: -32000,
        message: format!("Location not found: {query}"),
    })?;

    let lat = first["latitude"].as_f64().unwrap_or(0.0);
    let lon = first["longitude"].as_f64().unwrap_or(0.0);
    let name = first["name"].as_str().unwrap_or(query);
    let country = first["country"].as_str().unwrap_or("");

    let resolved = if country.is_empty() {
        name.to_string()
    } else {
        format!("{name}, {country}")
    };

    Ok(Location {
        name: resolved,
        lat,
        lon,
    })
}

// ── Alert generation ─────────────────────────────────────────────────

fn generate_alerts(
    weather_code: i32,
    wind_speed: f64,
    uv_index: f64,
    fahrenheit: bool,
) -> Vec<WeatherAlert> {
    let mut alerts = Vec::new();

    match weather_code {
        95 => alerts.push(WeatherAlert {
            severity: "warning".to_string(),
            title: "Thunderstorm Warning".to_string(),
            description: "Thunderstorm activity in the area. Seek shelter indoors.".to_string(),
            expires: String::new(),
        }),
        96 | 99 => alerts.push(WeatherAlert {
            severity: "emergency".to_string(),
            title: "Severe Thunderstorm".to_string(),
            description: "Thunderstorm with hail expected. Stay indoors.".to_string(),
            expires: String::new(),
        }),
        _ => {}
    }

    match weather_code {
        65 | 82 => alerts.push(WeatherAlert {
            severity: "watch".to_string(),
            title: "Heavy Rain Alert".to_string(),
            description: "Heavy rainfall expected. Potential for localized flooding.".to_string(),
            expires: String::new(),
        }),
        75 | 77 => alerts.push(WeatherAlert {
            severity: "warning".to_string(),
            title: "Heavy Snow Warning".to_string(),
            description: "Heavy snowfall expected. Travel may be hazardous.".to_string(),
            expires: String::new(),
        }),
        56 | 57 | 66 | 67 => alerts.push(WeatherAlert {
            severity: "warning".to_string(),
            title: "Freezing Precipitation".to_string(),
            description: "Freezing rain/drizzle. Icy conditions on roads.".to_string(),
            expires: String::new(),
        }),
        _ => {}
    }

    let wind_threshold = if fahrenheit { 40.0 } else { 60.0 };
    if wind_speed > wind_threshold {
        alerts.push(WeatherAlert {
            severity: "watch".to_string(),
            title: "High Wind Advisory".to_string(),
            description: format!(
                "Wind speeds of {:.0} {}. Secure loose objects.",
                wind_speed,
                if fahrenheit { "mph" } else { "km/h" }
            ),
            expires: String::new(),
        });
    }

    if uv_index >= 8.0 {
        alerts.push(WeatherAlert {
            severity: if uv_index >= 11.0 { "warning" } else { "watch" }.to_string(),
            title: "Very High UV Index".to_string(),
            description: format!("UV index of {uv_index:.0}. Limit sun exposure and wear sunscreen."),
            expires: String::new(),
        });
    }

    if weather_code == 45 || weather_code == 48 {
        alerts.push(WeatherAlert {
            severity: "watch".to_string(),
            title: "Fog Advisory".to_string(),
            description: "Reduced visibility due to fog. Drive with caution.".to_string(),
            expires: String::new(),
        });
    }

    alerts
}

// ── WMO code helpers (moved from wire/weather.rs) ────────────────────

fn wmo_icon(code: i32, is_day: bool) -> &'static str {
    match code {
        0 => if is_day { "\u{2600}\u{FE0F}" } else { "\u{1F319}" },
        1 | 2 => if is_day { "\u{26C5}" } else { "\u{1F319}" },
        3 => "\u{2601}\u{FE0F}",
        45 | 48 => "\u{1F32B}\u{FE0F}",
        51 | 53 | 55 => "\u{1F326}\u{FE0F}",
        56 | 57 => "\u{1F327}\u{FE0F}",
        61 | 63 => "\u{1F327}\u{FE0F}",
        65 => "\u{1F327}\u{FE0F}",
        66 | 67 => "\u{1F327}\u{FE0F}",
        71 | 73 => "\u{1F328}\u{FE0F}",
        75 | 77 => "\u{1F328}\u{FE0F}",
        80 | 81 | 82 => "\u{1F327}\u{FE0F}",
        85 | 86 => "\u{1F328}\u{FE0F}",
        95 => "\u{26C8}\u{FE0F}",
        96 | 99 => "\u{26C8}\u{FE0F}",
        _ => "\u{2601}\u{FE0F}",
    }
}

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

fn wind_direction_str(degrees: f64) -> &'static str {
    let dirs = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE",
        "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW",
    ];
    let idx = ((degrees + 11.25) / 22.5) as usize % 16;
    dirs[idx]
}

fn extract_time(s: &str) -> String {
    if let Some(pos) = s.find('T') {
        s[pos + 1..].to_string()
    } else {
        s.to_string()
    }
}

fn extract_hour(s: &str) -> usize {
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
