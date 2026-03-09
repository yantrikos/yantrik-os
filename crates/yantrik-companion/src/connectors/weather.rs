//! Weather Connector — location-aware forecasts via Open-Meteo (free, no API key).
//!
//! Polls hourly. Produces normalized `LifeEvent`s:
//! - `WeatherForecast` — daily summary with highs/lows/conditions
//! - `PrecipitationAlert` — rain/snow expected during commute or outdoor hours
//! - `TemperatureExtreme` — unusually hot or cold day
//! - `SevereWeather` — storms, high winds, extreme conditions
//!
//! Uses Open-Meteo's free API:
//! `https://api.open-meteo.com/v1/forecast?latitude=X&longitude=Y&...`
//!
//! The connector is location-aware: it uses home/work coordinates from
//! onboarding config. If geocoding is needed, Open-Meteo provides that too:
//! `https://geocoding-api.open-meteo.com/v1/search?name=CityName`

use serde::{Deserialize, Serialize};

use crate::graph_bridge::{LifeEvent, LifeEventKind};

// ── Configuration ───────────────────────────────────────────────────

/// A named location to monitor weather for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherLocation {
    pub name: String, // "home", "work", "vacation"
    pub latitude: f64,
    pub longitude: f64,
}

/// Weather connector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherConfig {
    pub locations: Vec<WeatherLocation>,
    /// Temperature unit: "celsius" or "fahrenheit"
    pub temperature_unit: String,
    /// Hours considered "commute window" (e.g., 7-9 and 17-19)
    pub commute_hours: Vec<u8>,
    /// Temperature thresholds for extreme alerts
    pub extreme_hot_c: f64,
    pub extreme_cold_c: f64,
    /// Wind speed threshold for alerts (km/h)
    pub wind_alert_kmh: f64,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            locations: vec![],
            temperature_unit: "celsius".into(),
            commute_hours: vec![7, 8, 9, 17, 18, 19],
            extreme_hot_c: 38.0,
            extreme_cold_c: 0.0,
            wind_alert_kmh: 50.0,
        }
    }
}

// ── Open-Meteo Response Types ───────────────────────────────────────

/// Parsed Open-Meteo hourly forecast response.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenMeteoResponse {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub hourly: HourlyData,
    #[serde(default)]
    pub daily: DailyData,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HourlyData {
    #[serde(default)]
    pub time: Vec<String>,
    #[serde(default)]
    pub temperature_2m: Vec<f64>,
    #[serde(default)]
    pub precipitation_probability: Vec<f64>,
    #[serde(default)]
    pub precipitation: Vec<f64>,
    #[serde(default)]
    pub weather_code: Vec<u32>,
    #[serde(default)]
    pub wind_speed_10m: Vec<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DailyData {
    #[serde(default)]
    pub time: Vec<String>,
    #[serde(default)]
    pub temperature_2m_max: Vec<f64>,
    #[serde(default)]
    pub temperature_2m_min: Vec<f64>,
    #[serde(default)]
    pub precipitation_sum: Vec<f64>,
    #[serde(default)]
    pub precipitation_probability_max: Vec<f64>,
    #[serde(default)]
    pub weather_code: Vec<u32>,
    #[serde(default)]
    pub wind_speed_10m_max: Vec<f64>,
    #[serde(default)]
    pub sunrise: Vec<String>,
    #[serde(default)]
    pub sunset: Vec<String>,
}

/// Geocoding response from Open-Meteo.
#[derive(Debug, Clone, Deserialize)]
pub struct GeocodingResponse {
    #[serde(default)]
    pub results: Vec<GeoResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeoResult {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub admin1: Option<String>,
}

// ── WMO Weather Code Descriptions ───────────────────────────────────

/// Convert WMO weather code to human-readable description.
pub fn weather_code_description(code: u32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Foggy",
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

/// Is this weather code a precipitation code?
pub fn is_precipitation_code(code: u32) -> bool {
    matches!(code, 51..=67 | 71..=77 | 80..=86 | 95..=99)
}

/// Is this a severe weather code?
pub fn is_severe_code(code: u32) -> bool {
    matches!(code, 65 | 67 | 75 | 82 | 86 | 95 | 96 | 99)
}

// ── Geocoding ───────────────────────────────────────────────────────

/// Look up coordinates for a city/place name using Open-Meteo geocoding.
pub fn geocode(place_name: &str) -> Result<GeoResult, String> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        urlencoded(place_name)
    );

    let resp: GeocodingResponse = ureq::get(&url)
        .set("User-Agent", "YantrikOS/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| format!("Geocoding HTTP error: {}", e))?
        .into_json()
        .map_err(|e| format!("Geocoding parse error: {}", e))?;

    resp.results
        .into_iter()
        .next()
        .ok_or_else(|| format!("No results for '{}'", place_name))
}

// ── Fetch Forecast ──────────────────────────────────────────────────

/// Fetch weather forecast from Open-Meteo for given coordinates.
/// Returns 2-day forecast with hourly + daily data.
pub fn fetch_forecast(lat: f64, lon: f64) -> Result<OpenMeteoResponse, String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude={lat}&longitude={lon}\
         &hourly=temperature_2m,precipitation_probability,precipitation,weather_code,wind_speed_10m\
         &daily=temperature_2m_max,temperature_2m_min,precipitation_sum,precipitation_probability_max,weather_code,wind_speed_10m_max,sunrise,sunset\
         &forecast_days=2\
         &timezone=auto"
    );

    ureq::get(&url)
        .set("User-Agent", "YantrikOS/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| format!("Weather HTTP error: {}", e))?
        .into_json()
        .map_err(|e| format!("Weather parse error: {}", e))
}

// ── Analyze Forecast → LifeEvents ──────────────────────────────────

/// Analyze a forecast response and produce relevant LifeEvents.
pub fn analyze_forecast(
    location: &WeatherLocation,
    forecast: &OpenMeteoResponse,
    config: &WeatherConfig,
) -> Vec<LifeEvent> {
    let now = now_ts();
    let mut events = Vec::new();

    // 1. Daily summary forecast
    if let Some(summary) = build_daily_summary(location, forecast) {
        events.push(LifeEvent {
            kind: LifeEventKind::WeatherForecast,
            summary,
            keywords: vec!["weather".into(), "forecast".into(), location.name.clone()],
            entities: vec![location.name.clone()],
            importance: 0.3,
            source: "weather".into(),
            data: serde_json::json!({
                "location": location.name,
                "lat": forecast.latitude,
                "lon": forecast.longitude,
                "daily_high": forecast.daily.temperature_2m_max.first(),
                "daily_low": forecast.daily.temperature_2m_min.first(),
                "weather_code": forecast.daily.weather_code.first(),
            }),
            timestamp: now,
        });
    }

    // 2. Precipitation during commute hours
    if let Some(precip_event) = check_commute_precipitation(location, forecast, config) {
        events.push(precip_event);
    }

    // 3. Temperature extremes
    if let Some(temp_event) = check_temperature_extremes(location, forecast, config) {
        events.push(temp_event);
    }

    // 4. Severe weather
    if let Some(severe_event) = check_severe_weather(location, forecast) {
        events.push(severe_event);
    }

    // 5. High wind alert
    if let Some(wind_event) = check_high_wind(location, forecast, config) {
        events.push(wind_event);
    }

    events
}

fn build_daily_summary(location: &WeatherLocation, forecast: &OpenMeteoResponse) -> Option<String> {
    let high = forecast.daily.temperature_2m_max.first()?;
    let low = forecast.daily.temperature_2m_min.first()?;
    let code = forecast.daily.weather_code.first()?;
    let desc = weather_code_description(*code);
    let precip = forecast.daily.precipitation_sum.first().unwrap_or(&0.0);

    let mut summary = format!(
        "{}: {} — {:.0}°C to {:.0}°C",
        location.name, desc, low, high
    );
    if *precip > 0.0 {
        summary.push_str(&format!(", {:.1}mm precipitation", precip));
    }
    Some(summary)
}

fn check_commute_precipitation(
    location: &WeatherLocation,
    forecast: &OpenMeteoResponse,
    config: &WeatherConfig,
) -> Option<LifeEvent> {
    let hourly = &forecast.hourly;
    if hourly.time.is_empty() {
        return None;
    }

    let mut commute_rain = false;
    let mut max_precip_prob = 0.0_f64;
    let mut rain_hours = Vec::new();

    for (i, time_str) in hourly.time.iter().enumerate() {
        // Extract hour from ISO time string "2026-03-09T07:00"
        let hour = extract_hour(time_str);
        if config.commute_hours.contains(&hour) {
            let prob = hourly.precipitation_probability.get(i).copied().unwrap_or(0.0);
            let precip = hourly.precipitation.get(i).copied().unwrap_or(0.0);
            let code = hourly.weather_code.get(i).copied().unwrap_or(0);

            if prob > 50.0 || precip > 0.5 || is_precipitation_code(code) {
                commute_rain = true;
                max_precip_prob = max_precip_prob.max(prob);
                rain_hours.push(format!("{}:00", hour));
            }
        }
    }

    if !commute_rain {
        return None;
    }

    let summary = format!(
        "Rain expected at {} during commute hours ({}). {}% chance — carry an umbrella.",
        location.name,
        rain_hours.join(", "),
        max_precip_prob as u32
    );

    Some(LifeEvent {
        kind: LifeEventKind::PrecipitationAlert,
        summary,
        keywords: vec!["weather".into(), "rain".into(), "commute".into(), "umbrella".into()],
        entities: vec![location.name.clone()],
        importance: 0.7,
        source: "weather".into(),
        data: serde_json::json!({
            "location": location.name,
            "rain_hours": rain_hours,
            "max_probability": max_precip_prob,
        }),
        timestamp: now_ts(),
    })
}

fn check_temperature_extremes(
    location: &WeatherLocation,
    forecast: &OpenMeteoResponse,
    config: &WeatherConfig,
) -> Option<LifeEvent> {
    let high = forecast.daily.temperature_2m_max.first()?;
    let low = forecast.daily.temperature_2m_min.first()?;

    if *high >= config.extreme_hot_c {
        let summary = format!(
            "Extreme heat at {} today — {:.0}°C. Stay hydrated and avoid prolonged outdoor activity.",
            location.name, high
        );
        return Some(LifeEvent {
            kind: LifeEventKind::TemperatureExtreme,
            summary,
            keywords: vec!["weather".into(), "heat".into(), "extreme".into(), "temperature".into()],
            entities: vec![location.name.clone()],
            importance: 0.8,
            source: "weather".into(),
            data: serde_json::json!({
                "location": location.name,
                "type": "heat",
                "temperature": high,
            }),
            timestamp: now_ts(),
        });
    }

    if *low <= config.extreme_cold_c {
        let summary = format!(
            "Very cold at {} today — low of {:.0}°C. Bundle up and watch for ice.",
            location.name, low
        );
        return Some(LifeEvent {
            kind: LifeEventKind::TemperatureExtreme,
            summary,
            keywords: vec!["weather".into(), "cold".into(), "freeze".into(), "temperature".into()],
            entities: vec![location.name.clone()],
            importance: 0.7,
            source: "weather".into(),
            data: serde_json::json!({
                "location": location.name,
                "type": "cold",
                "temperature": low,
            }),
            timestamp: now_ts(),
        });
    }

    None
}

fn check_severe_weather(
    location: &WeatherLocation,
    forecast: &OpenMeteoResponse,
) -> Option<LifeEvent> {
    // Check next 24 hours for severe codes
    let hourly = &forecast.hourly;
    let mut severe_hours = Vec::new();
    let mut worst_code = 0u32;

    for (i, code) in hourly.weather_code.iter().enumerate().take(24) {
        if is_severe_code(*code) {
            let hour = hourly.time.get(i).map(|t| extract_hour(t)).unwrap_or(0);
            severe_hours.push(format!("{}:00", hour));
            worst_code = worst_code.max(*code);
        }
    }

    if severe_hours.is_empty() {
        return None;
    }

    let desc = weather_code_description(worst_code);
    let summary = format!(
        "Severe weather alert for {}: {} expected around {}. Take precautions.",
        location.name, desc, severe_hours.join(", ")
    );

    Some(LifeEvent {
        kind: LifeEventKind::SevereWeather,
        summary,
        keywords: vec!["weather".into(), "severe".into(), "storm".into(), "alert".into()],
        entities: vec![location.name.clone()],
        importance: 0.9,
        source: "weather".into(),
        data: serde_json::json!({
            "location": location.name,
            "weather_code": worst_code,
            "description": desc,
            "severe_hours": severe_hours,
        }),
        timestamp: now_ts(),
    })
}

fn check_high_wind(
    location: &WeatherLocation,
    forecast: &OpenMeteoResponse,
    config: &WeatherConfig,
) -> Option<LifeEvent> {
    let max_wind = forecast.daily.wind_speed_10m_max.first()?;
    if *max_wind < config.wind_alert_kmh {
        return None;
    }

    let summary = format!(
        "High winds at {} today — gusts up to {:.0} km/h. Secure loose items outdoors.",
        location.name, max_wind
    );

    Some(LifeEvent {
        kind: LifeEventKind::SevereWeather,
        summary,
        keywords: vec!["weather".into(), "wind".into(), "storm".into()],
        entities: vec![location.name.clone()],
        importance: 0.6,
        source: "weather".into(),
        data: serde_json::json!({
            "location": location.name,
            "max_wind_kmh": max_wind,
        }),
        timestamp: now_ts(),
    })
}

// ── Full Scan ───────────────────────────────────────────────────────

/// Scan all configured locations and produce weather events.
/// This is the main entry point called by the stewardship runtime.
pub fn scan_weather(config: &WeatherConfig) -> Vec<LifeEvent> {
    let mut all_events = Vec::new();

    for location in &config.locations {
        match fetch_forecast(location.latitude, location.longitude) {
            Ok(forecast) => {
                let events = analyze_forecast(location, &forecast, config);
                tracing::info!(
                    location = %location.name,
                    event_count = events.len(),
                    "Weather scan complete"
                );
                all_events.extend(events);
            }
            Err(e) => {
                tracing::warn!(location = %location.name, error = %e, "Weather fetch failed");
            }
        }
    }

    all_events
}

// ── Helpers ─────────────────────────────────────────────────────────

fn extract_hour(time_str: &str) -> u8 {
    // "2026-03-09T07:00" → 7
    time_str
        .split('T')
        .nth(1)
        .and_then(|t| t.split(':').next())
        .and_then(|h| h.parse().ok())
        .unwrap_or(0)
}

fn urlencoded(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('+', "%2B")
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> WeatherConfig {
        WeatherConfig {
            locations: vec![WeatherLocation {
                name: "home".into(),
                latitude: 12.97,
                longitude: 77.59,
            }],
            temperature_unit: "celsius".into(),
            commute_hours: vec![7, 8, 9, 17, 18, 19],
            extreme_hot_c: 38.0,
            extreme_cold_c: 0.0,
            wind_alert_kmh: 50.0,
        }
    }

    fn test_location() -> WeatherLocation {
        WeatherLocation {
            name: "home".into(),
            latitude: 12.97,
            longitude: 77.59,
        }
    }

    fn base_forecast() -> OpenMeteoResponse {
        OpenMeteoResponse {
            latitude: 12.97,
            longitude: 77.59,
            hourly: HourlyData {
                time: (0..24).map(|h| format!("2026-03-09T{:02}:00", h)).collect(),
                temperature_2m: vec![25.0; 24],
                precipitation_probability: vec![0.0; 24],
                precipitation: vec![0.0; 24],
                weather_code: vec![0; 24],
                wind_speed_10m: vec![10.0; 24],
            },
            daily: DailyData {
                time: vec!["2026-03-09".into()],
                temperature_2m_max: vec![32.0],
                temperature_2m_min: vec![22.0],
                precipitation_sum: vec![0.0],
                precipitation_probability_max: vec![0.0],
                weather_code: vec![1],
                wind_speed_10m_max: vec![15.0],
                sunrise: vec!["2026-03-09T06:15".into()],
                sunset: vec!["2026-03-09T18:30".into()],
            },
        }
    }

    #[test]
    fn weather_code_descriptions() {
        assert_eq!(weather_code_description(0), "Clear sky");
        assert_eq!(weather_code_description(63), "Moderate rain");
        assert_eq!(weather_code_description(95), "Thunderstorm");
        assert_eq!(weather_code_description(255), "Unknown");
    }

    #[test]
    fn daily_summary_event() {
        let loc = test_location();
        let config = test_config();
        let forecast = base_forecast();

        let events = analyze_forecast(&loc, &forecast, &config);
        // Should produce at least the daily summary
        assert!(events.iter().any(|e| e.kind == LifeEventKind::WeatherForecast));

        let summary = events.iter().find(|e| e.kind == LifeEventKind::WeatherForecast).unwrap();
        assert!(summary.summary.contains("home"));
        assert!(summary.summary.contains("22°C"));
        assert!(summary.summary.contains("32°C"));
    }

    #[test]
    fn precipitation_during_commute() {
        let loc = test_location();
        let config = test_config();
        let mut forecast = base_forecast();

        // Set rain during commute hours (7, 8, 9)
        forecast.hourly.precipitation_probability[7] = 80.0;
        forecast.hourly.precipitation_probability[8] = 70.0;
        forecast.hourly.weather_code[7] = 61; // slight rain
        forecast.hourly.weather_code[8] = 63; // moderate rain

        let events = analyze_forecast(&loc, &forecast, &config);
        let precip = events.iter().find(|e| e.kind == LifeEventKind::PrecipitationAlert);
        assert!(precip.is_some(), "Should produce precipitation alert");

        let precip = precip.unwrap();
        assert!(precip.summary.contains("umbrella"));
        assert!(precip.importance >= 0.7);
        assert!(precip.keywords.contains(&"rain".to_string()));
    }

    #[test]
    fn no_precipitation_alert_when_dry() {
        let loc = test_location();
        let config = test_config();
        let forecast = base_forecast(); // all clear

        let events = analyze_forecast(&loc, &forecast, &config);
        assert!(events.iter().all(|e| e.kind != LifeEventKind::PrecipitationAlert));
    }

    #[test]
    fn extreme_heat_alert() {
        let loc = test_location();
        let config = test_config();
        let mut forecast = base_forecast();
        forecast.daily.temperature_2m_max[0] = 42.0; // 42°C

        let events = analyze_forecast(&loc, &forecast, &config);
        let heat = events.iter().find(|e| e.kind == LifeEventKind::TemperatureExtreme);
        assert!(heat.is_some(), "Should produce heat alert");
        assert!(heat.unwrap().summary.contains("42°C"));
    }

    #[test]
    fn extreme_cold_alert() {
        let loc = test_location();
        let config = test_config();
        let mut forecast = base_forecast();
        forecast.daily.temperature_2m_min[0] = -5.0;

        let events = analyze_forecast(&loc, &forecast, &config);
        let cold = events.iter().find(|e| e.kind == LifeEventKind::TemperatureExtreme);
        assert!(cold.is_some(), "Should produce cold alert");
        assert!(cold.unwrap().summary.contains("-5°C"));
    }

    #[test]
    fn severe_weather_alert() {
        let loc = test_location();
        let config = test_config();
        let mut forecast = base_forecast();

        // Thunderstorm at 14:00 and 15:00
        forecast.hourly.weather_code[14] = 95;
        forecast.hourly.weather_code[15] = 96; // thunderstorm with hail

        let events = analyze_forecast(&loc, &forecast, &config);
        let severe = events.iter().find(|e| e.kind == LifeEventKind::SevereWeather);
        assert!(severe.is_some(), "Should produce severe weather alert");
        assert!(severe.unwrap().importance >= 0.9);
    }

    #[test]
    fn high_wind_alert() {
        let loc = test_location();
        let config = test_config();
        let mut forecast = base_forecast();
        forecast.daily.wind_speed_10m_max[0] = 65.0;

        let events = analyze_forecast(&loc, &forecast, &config);
        let wind = events.iter().find(|e| e.summary.contains("wind"));
        assert!(wind.is_some(), "Should produce wind alert");
        assert!(wind.unwrap().summary.contains("65"));
    }

    #[test]
    fn no_extremes_normal_weather() {
        let loc = test_location();
        let config = test_config();
        let forecast = base_forecast(); // 22-32°C, clear, low wind

        let events = analyze_forecast(&loc, &forecast, &config);
        // Should only have the daily summary, no alerts
        assert_eq!(
            events.iter().filter(|e| e.kind != LifeEventKind::WeatherForecast).count(),
            0,
            "Normal weather should produce no alerts"
        );
    }

    #[test]
    fn extract_hour_parsing() {
        assert_eq!(extract_hour("2026-03-09T07:00"), 7);
        assert_eq!(extract_hour("2026-03-09T17:00"), 17);
        assert_eq!(extract_hour("2026-03-09T00:00"), 0);
        assert_eq!(extract_hour("invalid"), 0);
    }

    #[test]
    fn precipitation_code_checks() {
        assert!(is_precipitation_code(61)); // slight rain
        assert!(is_precipitation_code(75)); // heavy snow
        assert!(is_precipitation_code(95)); // thunderstorm
        assert!(!is_precipitation_code(0)); // clear
        assert!(!is_precipitation_code(3)); // overcast
    }
}
