//! Weather dashboard wiring — fetches Open-Meteo API data,
//! populates Slint models for current conditions, hourly, and daily forecasts.
//! Polls every 30 minutes and refreshes on screen open.
//! Supports saved locations, unit toggle (°C/°F), weather alerts, AQI, last-updated.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, WeatherAlert, WeatherCurrent, WeatherDaily, WeatherHourly, WeatherSavedLocation};

/// Shared weather data container for cross-thread communication.
#[derive(Clone, Default)]
struct WeatherData {
    current: Option<WeatherCurrent>,
    hourly: Option<Vec<WeatherHourly>>,
    daily: Option<Vec<WeatherDaily>>,
    alerts: Option<Vec<WeatherAlert>>,
    aqi_value: Option<String>,
    aqi_label: Option<String>,
    aqi_level: Option<i32>,
    error: Option<String>,
}

/// A saved location with coordinates.
#[derive(Clone, Debug)]
struct SavedLocation {
    name: String,
    lat: f64,
    lon: f64,
}

/// Shared state for locations, units, timestamps.
#[derive(Clone)]
struct WeatherState {
    locations: Arc<Mutex<Vec<SavedLocation>>>,
    active_index: Arc<Mutex<usize>>,
    use_fahrenheit: Arc<Mutex<bool>>,
    last_fetch_time: Arc<Mutex<Option<Instant>>>,
}

impl WeatherState {
    fn new() -> Self {
        let locations = vec![SavedLocation {
            name: DEFAULT_LOCATION_NAME.to_string(),
            lat: DEFAULT_LAT,
            lon: DEFAULT_LON,
        }];
        Self {
            locations: Arc::new(Mutex::new(locations)),
            active_index: Arc::new(Mutex::new(0)),
            use_fahrenheit: Arc::new(Mutex::new(false)),
            last_fetch_time: Arc::new(Mutex::new(None)),
        }
    }

    fn active_location(&self) -> SavedLocation {
        let locs = self.locations.lock().unwrap();
        let idx = *self.active_index.lock().unwrap();
        locs.get(idx)
            .cloned()
            .unwrap_or(SavedLocation {
                name: DEFAULT_LOCATION_NAME.to_string(),
                lat: DEFAULT_LAT,
                lon: DEFAULT_LON,
            })
    }

    fn is_fahrenheit(&self) -> bool {
        *self.use_fahrenheit.lock().unwrap()
    }

    fn set_fahrenheit(&self, v: bool) {
        *self.use_fahrenheit.lock().unwrap() = v;
    }

    fn record_fetch_time(&self) {
        *self.last_fetch_time.lock().unwrap() = Some(Instant::now());
    }

    fn last_updated_text(&self) -> String {
        let guard = self.last_fetch_time.lock().unwrap();
        match *guard {
            None => String::new(),
            Some(t) => {
                let elapsed = t.elapsed().as_secs();
                if elapsed < 60 {
                    "Updated just now".to_string()
                } else if elapsed < 3600 {
                    let mins = elapsed / 60;
                    if mins == 1 {
                        "Updated 1 min ago".to_string()
                    } else {
                        format!("Updated {} min ago", mins)
                    }
                } else {
                    let hours = elapsed / 3600;
                    if hours == 1 {
                        "Updated 1 hr ago".to_string()
                    } else {
                        format!("Updated {} hr ago", hours)
                    }
                }
            }
        }
    }

    fn to_slint_locations(&self) -> Vec<WeatherSavedLocation> {
        let locs = self.locations.lock().unwrap();
        let active = *self.active_index.lock().unwrap();
        locs.iter()
            .enumerate()
            .map(|(i, loc)| WeatherSavedLocation {
                name: SharedString::from(&loc.name),
                lat: loc.lat as f32,
                lon: loc.lon as f32,
                is_active: i == active,
            })
            .collect()
    }
}

/// Default location: London (used when no config location is available).
const DEFAULT_LAT: f64 = 51.5074;
const DEFAULT_LON: f64 = -0.1278;
const DEFAULT_LOCATION_NAME: &str = "London";

/// Wire weather dashboard callbacks and timers.
pub fn wire(ui: &App, ctx: &AppContext) {
    let data_slot: Arc<Mutex<Option<WeatherData>>> = Arc::new(Mutex::new(None));
    let state = WeatherState::new();

    // Set initial locations model
    ui.set_weather_saved_locations(ModelRc::new(VecModel::from(state.to_slint_locations())));

    // ── Refresh callback ──
    let slot_for_refresh = data_slot.clone();
    let ui_weak_refresh = ui.as_weak();
    let state_refresh = state.clone();
    ui.on_weather_refresh(move || {
        let loc = state_refresh.active_location();
        let fahrenheit = state_refresh.is_fahrenheit();
        fetch_weather_async(slot_for_refresh.clone(), loc.lat, loc.lon, &loc.name, fahrenheit);
        if let Some(ui) = ui_weak_refresh.upgrade() {
            ui.set_weather_current(WeatherCurrent {
                is_loading: true,
                ..ui.get_weather_current()
            });
        }
    });

    // ── Select location callback ──
    let slot_sel = data_slot.clone();
    let state_sel = state.clone();
    let ui_weak_sel = ui.as_weak();
    ui.on_weather_select_location(move |idx| {
        let idx = idx as usize;
        {
            let locs = state_sel.locations.lock().unwrap();
            if idx >= locs.len() {
                return;
            }
        }
        *state_sel.active_index.lock().unwrap() = idx;
        if let Some(ui) = ui_weak_sel.upgrade() {
            ui.set_weather_saved_locations(ModelRc::new(VecModel::from(
                state_sel.to_slint_locations(),
            )));
            ui.set_weather_current(WeatherCurrent {
                is_loading: true,
                ..ui.get_weather_current()
            });
        }
        let loc = state_sel.active_location();
        let fahrenheit = state_sel.is_fahrenheit();
        fetch_weather_async(slot_sel.clone(), loc.lat, loc.lon, &loc.name, fahrenheit);
    });

    // ── Add location callback ──
    let state_add = state.clone();
    let slot_add = data_slot.clone();
    let ui_weak_add = ui.as_weak();
    ui.on_weather_add_location(move |name| {
        let name_str = name.to_string().trim().to_string();
        if name_str.is_empty() {
            return;
        }
        // Geocode the location in a background thread, then add it
        let state_c = state_add.clone();
        let slot_c = slot_add.clone();
        let ui_w = ui_weak_add.clone();
        let name_owned = name_str.clone();
        std::thread::spawn(move || {
            if let Some((lat, lon, resolved_name)) = geocode_location(&name_owned) {
                {
                    let mut locs = state_c.locations.lock().unwrap();
                    // Check for duplicate
                    if locs.iter().any(|l| {
                        (l.lat - lat).abs() < 0.01 && (l.lon - lon).abs() < 0.01
                    }) {
                        return;
                    }
                    locs.push(SavedLocation {
                        name: resolved_name,
                        lat,
                        lon,
                    });
                    let new_idx = locs.len() - 1;
                    *state_c.active_index.lock().unwrap() = new_idx;
                }
                // Fetch weather for the new location
                let loc = state_c.active_location();
                let fahrenheit = state_c.is_fahrenheit();
                fetch_weather_async(slot_c, loc.lat, loc.lon, &loc.name, fahrenheit);
                // Update UI locations model from main thread
                let locs_slint = state_c.to_slint_locations();
                let _ = ui_w.upgrade_in_event_loop(move |ui| {
                    ui.set_weather_saved_locations(ModelRc::new(VecModel::from(locs_slint)));
                    ui.set_weather_current(WeatherCurrent {
                        is_loading: true,
                        ..ui.get_weather_current()
                    });
                });
            }
        });
    });

    // ── Remove location callback ──
    let state_rm = state.clone();
    let slot_rm = data_slot.clone();
    let ui_weak_rm = ui.as_weak();
    ui.on_weather_remove_location(move |idx| {
        let idx = idx as usize;
        let need_refetch;
        {
            let mut locs = state_rm.locations.lock().unwrap();
            if idx >= locs.len() || locs.len() <= 1 {
                return; // Don't remove the last location
            }
            locs.remove(idx);
            let mut active = state_rm.active_index.lock().unwrap();
            if *active >= locs.len() {
                *active = locs.len() - 1;
            }
            need_refetch = idx == *active || *active >= locs.len();
        }
        if let Some(ui) = ui_weak_rm.upgrade() {
            ui.set_weather_saved_locations(ModelRc::new(VecModel::from(
                state_rm.to_slint_locations(),
            )));
        }
        if need_refetch {
            let loc = state_rm.active_location();
            let fahrenheit = state_rm.is_fahrenheit();
            fetch_weather_async(slot_rm.clone(), loc.lat, loc.lon, &loc.name, fahrenheit);
        }
    });

    // ── Toggle units callback ──
    let state_units = state.clone();
    let slot_units = data_slot.clone();
    let ui_weak_units = ui.as_weak();
    ui.on_weather_toggle_units(move || {
        let new_val = if let Some(ui) = ui_weak_units.upgrade() {
            ui.get_weather_use_fahrenheit()
        } else {
            return;
        };
        state_units.set_fahrenheit(new_val);
        // Re-fetch with new units
        let loc = state_units.active_location();
        fetch_weather_async(
            slot_units.clone(),
            loc.lat,
            loc.lon,
            &loc.name,
            new_val,
        );
    });

    // ── Poll timer: check for results every 100ms, re-fetch every 30 min ──
    let slot_for_poll = data_slot.clone();
    let ui_weak_poll = ui.as_weak();
    let state_poll = state.clone();

    // Kick off initial fetch
    {
        let loc = state.active_location();
        let fahrenheit = state.is_fahrenheit();
        fetch_weather_async(slot_for_poll.clone(), loc.lat, loc.lon, &loc.name, fahrenheit);
    }

    let fetch_interval = std::cell::Cell::new(0u32); // counts 100ms ticks
    const REFETCH_TICKS: u32 = 30 * 60 * 10; // 30 min at 100ms
    // Update "last updated" text every 30 seconds (300 ticks)
    const UPDATE_TEXT_TICKS: u32 = 300;

    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        // Check if data arrived
        {
            let mut slot = slot_for_poll.lock().unwrap();
            if let Some(data) = slot.take() {
                state_poll.record_fetch_time();
                if let Some(ui) = ui_weak_poll.upgrade() {
                    apply_weather_data(&ui, data);
                    ui.set_weather_last_updated(
                        SharedString::from(state_poll.last_updated_text()),
                    );
                }
            }
        }

        // Periodic update of "last updated" text
        let count = fetch_interval.get() + 1;
        fetch_interval.set(count);

        if count % UPDATE_TEXT_TICKS == 0 {
            if let Some(ui) = ui_weak_poll.upgrade() {
                ui.set_weather_last_updated(
                    SharedString::from(state_poll.last_updated_text()),
                );
            }
        }

        // Periodic re-fetch
        if count >= REFETCH_TICKS {
            fetch_interval.set(0);
            let loc = state_poll.active_location();
            let fahrenheit = state_poll.is_fahrenheit();
            fetch_weather_async(
                slot_for_poll.clone(),
                loc.lat,
                loc.lon,
                &loc.name,
                fahrenheit,
            );
        }
    });

    // ── AI Insights callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_weather_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let current = ui.get_weather_current();
        let context = format!(
            "Location: {}\nTemperature: {} (feels like {})\nCondition: {}\nHumidity: {}\nWind: {} {}\nUV Index: {}\nPressure: {}\nCloud Cover: {}\nSunrise: {}, Sunset: {}",
            current.location,
            current.temperature,
            current.feels_like,
            current.condition,
            current.humidity,
            current.wind_speed,
            current.wind_direction,
            current.uv_index,
            current.pressure,
            current.cloud_cover,
            current.sunrise,
            current.sunset,
        );
        let prompt = super::ai_assist::weather_insights_prompt(&context);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_weather_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_weather_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_weather_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_weather_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_weather_ai_panel_open(false);
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
    if let Some(ref alerts) = data.alerts {
        // Determine the most severe alert for the banner
        let mut most_severe: Option<&WeatherAlert> = None;
        let mut max_severity = -1;
        for a in alerts.iter() {
            if a.severity > max_severity {
                max_severity = a.severity;
                most_severe = Some(a);
            }
        }

        if let Some(alert) = most_severe {
            ui.set_weather_severe_alert_active(true);
            ui.set_weather_severe_alert_text(alert.description.clone());
            let level = match alert.severity {
                0 => "watch",
                1 => "warning",
                _ => "emergency",
            };
            ui.set_weather_severe_alert_level(SharedString::from(level));
        } else {
            ui.set_weather_severe_alert_active(false);
            ui.set_weather_severe_alert_text(SharedString::default());
            ui.set_weather_severe_alert_level(SharedString::default());
        }

        ui.set_weather_alerts(ModelRc::new(VecModel::from(alerts.clone())));
    }
    if let Some(ref val) = data.aqi_value {
        ui.set_weather_aqi_value(SharedString::from(val.as_str()));
    }
    if let Some(ref label) = data.aqi_label {
        ui.set_weather_aqi_label(SharedString::from(label.as_str()));
    }
    if let Some(level) = data.aqi_level {
        ui.set_weather_aqi_level(level);
    }
    if let Some(error) = data.error {
        let mut c = ui.get_weather_current();
        c.error_text = error.into();
        c.is_loading = false;
        ui.set_weather_current(c);
    }
}

/// Spawn a thread to fetch weather data.
/// Tries the weather-service via JSON-RPC first, falls back to direct Open-Meteo API.
fn fetch_weather_async(
    slot: Arc<Mutex<Option<WeatherData>>>,
    lat: f64,
    lon: f64,
    location_name: &str,
    use_fahrenheit: bool,
) {
    let name = location_name.to_string();
    std::thread::spawn(move || {
        let data = match fetch_via_service(lat, lon, &name, use_fahrenheit) {
            Ok(d) => d,
            Err(_) => {
                // Service not running — fall back to direct API
                fetch_weather_blocking(lat, lon, &name, use_fahrenheit)
            }
        };
        *slot.lock().unwrap() = Some(data);
    });
}

/// Try fetching weather data via the weather-service JSON-RPC.
fn fetch_via_service(
    lat: f64,
    lon: f64,
    location_name: &str,
    use_fahrenheit: bool,
) -> Result<WeatherData, String> {
    use yantrik_ipc_transport::SyncRpcClient;

    let client = SyncRpcClient::for_service("weather");
    let params = serde_json::json!({
        "lat": lat,
        "lon": lon,
        "name": location_name,
        "fahrenheit": use_fahrenheit,
    });

    // Fetch current
    let current_json = client
        .call("weather.current", params.clone())
        .map_err(|e| e.message)?;
    let svc_current: yantrik_ipc_contracts::weather::CurrentWeather =
        serde_json::from_value(current_json).map_err(|e| e.to_string())?;

    // Fetch hourly
    let hourly_json = client
        .call("weather.hourly", params.clone())
        .map_err(|e| e.message)?;
    let svc_hourly: Vec<yantrik_ipc_contracts::weather::HourlyForecast> =
        serde_json::from_value(hourly_json).map_err(|e| e.to_string())?;

    // Fetch daily
    let daily_json = client
        .call("weather.daily", params.clone())
        .map_err(|e| e.message)?;
    let svc_daily: Vec<yantrik_ipc_contracts::weather::DailyForecast> =
        serde_json::from_value(daily_json).map_err(|e| e.to_string())?;

    // Fetch alerts
    let alerts_json = client
        .call("weather.alerts", params.clone())
        .map_err(|e| e.message)?;
    let svc_alerts: Vec<yantrik_ipc_contracts::weather::WeatherAlert> =
        serde_json::from_value(alerts_json).map_err(|e| e.to_string())?;

    // Fetch AQI
    let aqi_json = client
        .call("weather.air_quality", params)
        .map_err(|e| e.message)?;
    let svc_aqi: yantrik_ipc_contracts::weather::AirQuality =
        serde_json::from_value(aqi_json).map_err(|e| e.to_string())?;

    // Convert service types → Slint UI types
    let deg_symbol = if use_fahrenheit { "\u{00B0}F" } else { "\u{00B0}C" };
    let wind_label = if use_fahrenheit { "mph" } else { "km/h" };

    let temp_c = if use_fahrenheit {
        (svc_current.temperature - 32.0) * 5.0 / 9.0
    } else {
        svc_current.temperature
    };
    let dew_point_val = {
        let dp_c = compute_dew_point(temp_c, svc_current.humidity as f64);
        if use_fahrenheit {
            dp_c * 9.0 / 5.0 + 32.0
        } else {
            dp_c
        }
    };

    let current = WeatherCurrent {
        temperature: format!("{:.0}{}", svc_current.temperature, deg_symbol).into(),
        feels_like: format!("{:.0}{}", svc_current.feels_like, deg_symbol).into(),
        condition: svc_current.condition.into(),
        icon: svc_current.icon.into(),
        location: location_name.into(),
        humidity: format!("{}%", svc_current.humidity).into(),
        wind_speed: format!("{:.0} {}", svc_current.wind_speed, wind_label).into(),
        wind_direction: svc_current.wind_direction.into(),
        uv_index: format!("{:.0}", svc_current.uv_index).into(),
        visibility: "Good".into(),
        pressure: format!("{:.0} hPa", svc_current.pressure_hpa).into(),
        cloud_cover: "".into(),
        dew_point: format!("{:.0}{}", dew_point_val, deg_symbol).into(),
        sunrise: "".into(),
        sunset: "".into(),
        is_loading: false,
        error_text: "".into(),
    };

    let hourly: Vec<WeatherHourly> = svc_hourly
        .iter()
        .map(|h| WeatherHourly {
            time: h.time.clone().into(),
            icon: h.icon.clone().into(),
            temp: format!("{:.0}\u{00B0}", h.temperature).into(),
            is_current: h.time == "Now",
        })
        .collect();

    // Compute global min/max for daily temperature range bars
    let mut global_min = f64::MAX;
    let mut global_max = f64::MIN;
    for d in &svc_daily {
        if d.temp_low < global_min {
            global_min = d.temp_low;
        }
        if d.temp_high > global_max {
            global_max = d.temp_high;
        }
    }
    let range = (global_max - global_min).max(1.0);

    let daily: Vec<WeatherDaily> = svc_daily
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let day_name = if i == 0 {
                "Today".to_string()
            } else {
                day_of_week(&d.date)
            };
            WeatherDaily {
                day_name: day_name.into(),
                icon: d.icon.clone().into(),
                high: format!("{:.0}\u{00B0}", d.temp_high).into(),
                low: format!("{:.0}\u{00B0}", d.temp_low).into(),
                precip_chance: if d.precipitation_chance > 0 {
                    format!("{}%", d.precipitation_chance).into()
                } else {
                    "0mm".into()
                },
                high_value: d.temp_high as f32,
                low_value: d.temp_low as f32,
                temp_range_min: ((d.temp_low - global_min) / range) as f32,
                temp_range_max: ((d.temp_high - global_min) / range) as f32,
            }
        })
        .collect();

    let alerts: Vec<WeatherAlert> = svc_alerts
        .iter()
        .map(|a| {
            let severity = match a.severity.as_str() {
                "emergency" => 2,
                "warning" => 1,
                _ => 0,
            };
            WeatherAlert {
                title: a.title.clone().into(),
                description: a.description.clone().into(),
                severity,
                icon: "\u{26A0}\u{FE0F}".into(),
            }
        })
        .collect();

    let aqi_level = svc_aqi.level;
    let aqi_value = format!("{:.0}", svc_aqi.value);
    let aqi_label = svc_aqi.label;

    Ok(WeatherData {
        current: Some(current),
        hourly: Some(hourly),
        daily: Some(daily),
        alerts: Some(alerts),
        aqi_value: Some(aqi_value),
        aqi_label: Some(aqi_label),
        aqi_level: Some(aqi_level),
        error: None,
    })
}

/// Geocode a location name. Tries weather-service first, falls back to direct API.
fn geocode_location(query: &str) -> Option<(f64, f64, String)> {
    // Try service first
    if let Ok(loc) = geocode_via_service(query) {
        return Some((loc.lat, loc.lon, loc.name));
    }

    // Fallback: direct API call
    let encoded = query.replace(' ', "+");
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        encoded
    );

    let output = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "10", &url])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;

    let results = json["results"].as_array()?;
    let first = results.first()?;
    let lat = first["latitude"].as_f64()?;
    let lon = first["longitude"].as_f64()?;
    let name = first["name"].as_str().unwrap_or(query).to_string();
    let country = first["country"].as_str().unwrap_or("");

    let resolved = if country.is_empty() {
        name
    } else {
        format!("{}, {}", name, country)
    };

    Some((lat, lon, resolved))
}

fn geocode_via_service(
    query: &str,
) -> Result<yantrik_ipc_contracts::weather::Location, String> {
    use yantrik_ipc_transport::SyncRpcClient;

    let client = SyncRpcClient::for_service("weather");
    let result = client
        .call("weather.geocode", serde_json::json!({ "query": query }))
        .map_err(|e| e.message)?;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

/// Blocking weather fetch using curl (available on the Alpine VM).
fn fetch_weather_blocking(
    lat: f64,
    lon: f64,
    location_name: &str,
    use_fahrenheit: bool,
) -> WeatherData {
    let temp_unit = if use_fahrenheit {
        "&temperature_unit=fahrenheit"
    } else {
        ""
    };
    let wind_unit = if use_fahrenheit {
        "&wind_speed_unit=mph"
    } else {
        ""
    };

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude={lat}&longitude={lon}\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,\
         weather_code,wind_speed_10m,wind_direction_10m,surface_pressure,\
         is_day,cloud_cover\
         &hourly=temperature_2m,weather_code,precipitation_probability\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,\
         precipitation_sum,sunrise,sunset,uv_index_max\
         &timezone=auto&forecast_days=5{temp_unit}{wind_unit}"
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

    let deg_symbol = if use_fahrenheit { "\u{00B0}F" } else { "\u{00B0}C" };
    let wind_label = if use_fahrenheit { "mph" } else { "km/h" };

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

    // Compute dew point in the original unit
    let dew_point_val = if use_fahrenheit {
        // Convert to Celsius, compute, convert back
        let temp_c = (temp - 32.0) * 5.0 / 9.0;
        let dp_c = compute_dew_point(temp_c, humidity as f64);
        dp_c * 9.0 / 5.0 + 32.0
    } else {
        compute_dew_point(temp, humidity as f64)
    };

    let current = WeatherCurrent {
        temperature: format!("{:.0}{}", temp, deg_symbol).into(),
        feels_like: format!("{:.0}{}", apparent, deg_symbol).into(),
        condition: wmo_description(weather_code).into(),
        icon: wmo_icon(weather_code, is_day).into(),
        location: location_name.into(),
        humidity: format!("{}%", humidity).into(),
        wind_speed: format!("{:.0} {}", wind_speed, wind_label).into(),
        wind_direction: wind_direction_str(wind_dir).into(),
        uv_index: format!("{:.0}", uv_index).into(),
        visibility: "Good".into(),
        pressure: format!("{:.0} hPa", pressure).into(),
        cloud_cover: format!("{}%", cloud_cover).into(),
        dew_point: format!("{:.0}{}", dew_point_val, deg_symbol).into(),
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
        let current_time_str = current_obj["time"].as_str().unwrap_or("");
        let current_hour = extract_hour(current_time_str);

        for i in 0..times.len().min(48) {
            let time_str = times[i].as_str().unwrap_or("");
            let hour = extract_hour(time_str);
            let t = temps[i].as_f64().unwrap_or(0.0);
            let code = codes[i].as_i64().unwrap_or(0) as i32;
            let is_day_hour = (6..20).contains(&hour);
            let is_current = i == current_hour;

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

    // Generate weather alerts from current conditions
    let alerts = generate_weather_alerts(weather_code, wind_speed, uv_index, use_fahrenheit);

    // Fetch AQI data
    let (aqi_value, aqi_label, aqi_level) = fetch_aqi(lat, lon);

    WeatherData {
        current: Some(current),
        hourly: Some(hourly),
        daily: Some(daily),
        alerts: Some(alerts),
        aqi_value: Some(aqi_value),
        aqi_label: Some(aqi_label),
        aqi_level: Some(aqi_level),
        error: None,
    }
}

/// Generate weather alerts based on current conditions.
fn generate_weather_alerts(
    weather_code: i32,
    wind_speed: f64,
    uv_index: f64,
    use_fahrenheit: bool,
) -> Vec<WeatherAlert> {
    let mut alerts = Vec::new();

    // Thunderstorm alerts
    match weather_code {
        95 => alerts.push(WeatherAlert {
            title: "Thunderstorm Warning".into(),
            description: "Thunderstorm activity in the area. Seek shelter indoors.".into(),
            severity: 2,
            icon: "\u{26A0}\u{FE0F}".into(),
        }),
        96 | 99 => alerts.push(WeatherAlert {
            title: "Severe Thunderstorm".into(),
            description: "Thunderstorm with hail expected. Stay indoors and away from windows."
                .into(),
            severity: 2,
            icon: "\u{26A0}\u{FE0F}".into(),
        }),
        _ => {}
    }

    // Heavy precipitation alerts
    match weather_code {
        65 | 82 => alerts.push(WeatherAlert {
            title: "Heavy Rain Alert".into(),
            description: "Heavy rainfall expected. Potential for localized flooding.".into(),
            severity: 1,
            icon: "\u{1F327}\u{FE0F}".into(),
        }),
        75 | 77 => alerts.push(WeatherAlert {
            title: "Heavy Snow Warning".into(),
            description: "Heavy snowfall expected. Travel may be hazardous.".into(),
            severity: 2,
            icon: "\u{1F328}\u{FE0F}".into(),
        }),
        56 | 57 | 66 | 67 => alerts.push(WeatherAlert {
            title: "Freezing Precipitation".into(),
            description: "Freezing rain/drizzle. Icy conditions on roads and surfaces.".into(),
            severity: 2,
            icon: "\u{2744}\u{FE0F}".into(),
        }),
        _ => {}
    }

    // High wind alert (threshold differs by unit)
    let wind_threshold = if use_fahrenheit { 40.0 } else { 60.0 }; // mph vs km/h
    if wind_speed > wind_threshold {
        alerts.push(WeatherAlert {
            title: "High Wind Advisory".into(),
            description: format!(
                "Wind speeds of {:.0} {}. Secure loose objects.",
                wind_speed,
                if use_fahrenheit { "mph" } else { "km/h" }
            )
            .into(),
            severity: 1,
            icon: "\u{1F4A8}".into(),
        });
    }

    // Extreme UV alert
    if uv_index >= 8.0 {
        alerts.push(WeatherAlert {
            title: "Very High UV Index".into(),
            description: format!(
                "UV index of {:.0}. Limit sun exposure and wear sunscreen.",
                uv_index
            )
            .into(),
            severity: if uv_index >= 11.0 { 2 } else { 1 },
            icon: "\u{2600}\u{FE0F}".into(),
        });
    }

    // Fog advisory
    if weather_code == 45 || weather_code == 48 {
        alerts.push(WeatherAlert {
            title: "Fog Advisory".into(),
            description: "Reduced visibility due to fog. Drive with caution.".into(),
            severity: 0,
            icon: "\u{1F32B}\u{FE0F}".into(),
        });
    }

    alerts
}

/// Fetch Air Quality Index from Open-Meteo Air Quality API.
fn fetch_aqi(lat: f64, lon: f64) -> (String, String, i32) {
    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?\
         latitude={lat}&longitude={lon}\
         &current=european_aqi"
    );

    let output = match std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "10", &url])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return ("--".to_string(), "N/A".to_string(), 0),
    };

    let body = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return ("--".to_string(), "N/A".to_string(), 0),
    };

    let aqi = json["current"]["european_aqi"]
        .as_f64()
        .unwrap_or(-1.0) as i32;

    if aqi < 0 {
        return ("--".to_string(), "N/A".to_string(), 0);
    }

    let (label, level) = match aqi {
        0..=20 => ("Good", 0),
        21..=40 => ("Fair", 0),
        41..=60 => ("Moderate", 1),
        61..=80 => ("Poor", 1),
        81..=100 => ("Very Poor", 2),
        _ => ("Hazardous", 2),
    };

    (format!("{}", aqi), label.to_string(), level)
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
    if let Some(pos) = s.find('T') {
        s[pos + 1..].to_string()
    } else {
        s.to_string()
    }
}

/// Extract hour index from ISO datetime string relative to day start.
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

    let names = ["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"];
    names[h].to_string()
}
