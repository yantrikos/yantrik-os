//! Yantrik Weather — standalone app binary.
//!
//! Communicates with `weather-service` via JSON-RPC IPC.
//! Falls back to direct Open-Meteo API calls when service is unavailable.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_transport::SyncRpcClient;

mod state;
use state::{SavedLocation, WeatherState, FROM_DIRECT, FROM_SERVICE};

slint::include_modules!();

/// Fill the agent rail from the conditions already on screen.
///
/// The readings are the context. A forecast app's one genuinely useful question is not what the
/// numbers ARE -- they are right there -- but what to do about them.
fn refresh_agent_rail(ui: &WeatherApp) {
    let c = ui.get_current();
    let mut context: Vec<AgentContextItem> = Vec::new();
    if !c.temperature.is_empty() {
        context.push(AgentContextItem {
            id: "now".into(),
            label: format!("{}, {}", c.temperature, c.condition).into(),
            detail: c.feels_like.clone(),
            source: "file".into(),
        });
    }
    let updated = ui.get_weather_last_updated().to_string();
    if !updated.is_empty() {
        context.push(AgentContextItem {
            id: "updated".into(),
            label: updated.into(),
            detail: "last updated".into(),
            source: "file".into(),
        });
    }
    ui.set_agent_context(ModelRc::new(VecModel::from(context)));

    let reach = companion::reach();
    let mut next: Vec<AgentSuggestion> = Vec::new();
    if reach == companion::Reach::Ready && !c.temperature.is_empty() {
        next.push(AgentSuggestion {
            id: "advise".into(),
            label: "What should I plan for?".into(),
            detail: "reads today's conditions".into(),
            icon: "spark".into(),
            running: ui.get_proposal_working(),
            proposes: false,
        });
    }
    ui.set_agent_suggestions(ModelRc::new(VecModel::from(next)));
    ui.set_agent_unavailable(match reach.hint() {
        Some(hint) => hint.into(),
        None => SharedString::new(),
    });
}

/// The one question worth asking about a forecast, built from the reading on screen.
///
/// Shared by the rail's suggestion and the header's AI Insights button so the two cannot come
/// to differ; the button used to log a line and do nothing at all.
fn conditions_prompt(c: &WeatherCurrent) -> String {
    format!(
        "It is {} at {}, feels like {}. In at most three short lines say what to plan for \
         today. Use only these conditions.",
        c.condition, c.temperature, c.feels_like
    )
}

fn main() {
    init_tracing("yantrik-weather");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("weather") else { return };

    let app = WeatherApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    // ── The agent layer ──
    {
        let weak = app.as_weak();
        app.on_agent_suggestion_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if id != "advise" {
                return;
            }
            let prompt = conditions_prompt(&ui.get_current());
            ui.set_proposal_working(true);
            ui.set_proposal(AgentProposal {
                title: "Today outside".into(),
                source: "from today's conditions".into(),
                ..Default::default()
            });
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_proposal_working(false);
                    match outcome {
                        Ok(text) => ui.set_proposal(AgentProposal {
                            title: "Today outside".into(),
                            body: text.into(),
                            source: "from today's conditions".into(),
                            verb: "Close".into(),
                            ..Default::default()
                        }),
                        Err(e) => ui.set_proposal(AgentProposal {
                            title: "The companion did not answer".into(),
                            body: format!("{e}").into(),
                            verb: "Close".into(),
                            ..Default::default()
                        }),
                    }
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_proposal_dismissed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
    app.on_proposal_applied(|| {});
    app.on_agent_context_activated(|_| {});
    // The rail follows the app's state on a timer.
    //
    // Calling it once at startup was not enough: at that moment Weather has no reading yet and
    // Image Viewer has no file, so both rails computed "nothing to say", collapsed, and stayed
    // collapsed for the life of the process. Every app loads its content on some path of its own
    // and hooking each one is how a refresh gets missed; asking every few seconds is cheap and
    // cannot be forgotten.
    let rail_timer = slint::Timer::default();
    {
        let weak = app.as_weak();
        rail_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(4),
            move || {
                if let Some(ui) = weak.upgrade() {
                    refresh_agent_rail(&ui);
                }
            },
        );
    }
    refresh_agent_rail(&app);

    run_until_closed(&app, "yantrik-weather");
}

// ── Shared data ──────────────────────────────────────────────────────

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
    /// [`FROM_SERVICE`] or [`FROM_DIRECT`], and empty on a reading that never arrived.
    source: &'static str,
    /// Why the service was not used, when it was not. The fallback itself is fine — it reads
    /// the same Open-Meteo the service reads — but `Err(_) => fetch_weather_direct(…)` threw
    /// the reason away, so a person watching a service go down in the machine rail saw
    /// temperatures appear as usual and had no way to connect the two.
    degraded: Option<String>,
}

/// The first words of the strip the fetch path writes, so that a later good reading can clear
/// its own message without wiping one that is still true. A location that could not be added
/// is not news the next half-hourly refresh gets to throw away.
const DEGRADED_NOTICE: &str = "Readings came straight from Open-Meteo";

/// The saved places, in the shape the panel draws them.
fn to_slint_locations(state: &WeatherState) -> Vec<WeatherSavedLocation> {
    let active = state.active_index();
    state
        .locations()
        .iter()
        .enumerate()
        .map(|(i, loc)| WeatherSavedLocation {
            name: SharedString::from(&loc.name),
            lat: loc.lat as f32,
            lon: loc.lon as f32,
            is_active: i == active,
        })
        .collect()
}

// ── Service wrappers ─────────────────────────────────────────────────

fn fetch_via_service(lat: f64, lon: f64, location_name: &str, use_fahrenheit: bool) -> Result<WeatherData, String> {
    let client = SyncRpcClient::for_service("weather");
    let params = serde_json::json!({
        "lat": lat, "lon": lon, "name": location_name, "fahrenheit": use_fahrenheit, "days": 7,
    });

    let current_json = client.call("weather.current", params.clone()).map_err(|e| e.message)?;
    let svc_current: yantrik_ipc_contracts::weather::CurrentWeather =
        serde_json::from_value(current_json).map_err(|e| e.to_string())?;

    let hourly_json = client.call("weather.hourly", params.clone()).map_err(|e| e.message)?;
    let svc_hourly: Vec<yantrik_ipc_contracts::weather::HourlyForecast> =
        serde_json::from_value(hourly_json).map_err(|e| e.to_string())?;

    let daily_json = client.call("weather.daily", params.clone()).map_err(|e| e.message)?;
    let svc_daily: Vec<yantrik_ipc_contracts::weather::DailyForecast> =
        serde_json::from_value(daily_json).map_err(|e| e.to_string())?;

    let alerts_json = client.call("weather.alerts", params.clone()).map_err(|e| e.message)?;
    let svc_alerts: Vec<yantrik_ipc_contracts::weather::WeatherAlert> =
        serde_json::from_value(alerts_json).map_err(|e| e.to_string())?;

    let aqi_json = client.call("weather.air_quality", params).map_err(|e| e.message)?;
    let svc_aqi: yantrik_ipc_contracts::weather::AirQuality =
        serde_json::from_value(aqi_json).map_err(|e| e.to_string())?;

    let deg_symbol = if use_fahrenheit { "\u{00B0}F" } else { "\u{00B0}C" };
    let wind_label = if use_fahrenheit { "mph" } else { "km/h" };

    let today = svc_daily.first();
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
        visibility: if svc_current.visibility_km >= 10.0 { "Excellent" }
                    else if svc_current.visibility_km >= 4.0 { "Good" }
                    else { "Reduced" }.into(),
        pressure: format!("{:.0} hPa", svc_current.pressure_hpa).into(),
        cloud_cover: format!("{}%", svc_current.cloud_cover).into(),
        dew_point: format!("{:.0}{}", svc_current.dew_point, deg_symbol).into(),
        sunrise: today.map(|d| d.sunrise.clone()).unwrap_or_default().into(),
        sunset: today.map(|d| d.sunset.clone()).unwrap_or_default().into(),
        is_day: svc_current.is_day,
        is_loading: false,
        error_text: "".into(),
    };

    let h_min = svc_hourly.iter().map(|h| h.temperature).fold(f64::MAX, f64::min);
    let h_max = svc_hourly.iter().map(|h| h.temperature).fold(f64::MIN, f64::max);
    let hourly: Vec<WeatherHourly> = svc_hourly.iter().map(|h| WeatherHourly {
        time: h.time.clone().into(),
        icon: h.icon.clone().into(),
        temp: format!("{:.0}\u{00B0}", h.temperature).into(),
        precip: if h.precipitation_chance > 0 {
            format!("{}%", h.precipitation_chance).into()
        } else { "".into() },
        is_current: h.time == "Now",
        temp_value: h.temperature as f32,
        t_min: h_min as f32,
        t_max: h_max as f32,
    }).collect();

    let mut global_min = f64::MAX;
    let mut global_max = f64::MIN;
    for d in &svc_daily {
        if d.temp_low < global_min { global_min = d.temp_low; }
        if d.temp_high > global_max { global_max = d.temp_high; }
    }
    let range = (global_max - global_min).max(1.0);

    let daily: Vec<WeatherDaily> = svc_daily.iter().enumerate().map(|(i, d)| {
        let day_name = if i == 0 { "Today".to_string() } else { day_of_week(&d.date) };
        WeatherDaily {
            day_name: day_name.into(),
            icon: d.icon.clone().into(),
            high: format!("{:.0}\u{00B0}", d.temp_high).into(),
            low: format!("{:.0}\u{00B0}", d.temp_low).into(),
            precip_chance: if d.precipitation_chance > 0 {
                format!("{}%", d.precipitation_chance).into()
            } else { "0mm".into() },
            high_value: d.temp_high as f32,
            low_value: d.temp_low as f32,
            temp_range_min: ((d.temp_low - global_min) / range) as f32,
            temp_range_max: ((d.temp_high - global_min) / range) as f32,
        }
    }).collect();

    let alerts: Vec<WeatherAlert> = svc_alerts.iter().map(|a| {
        let severity = match a.severity.as_str() {
            "emergency" => 2, "warning" => 1, _ => 0,
        };
        WeatherAlert {
            title: a.title.clone().into(),
            description: a.description.clone().into(),
            severity,
            icon: "\u{26A0}\u{FE0F}".into(),
        }
    }).collect();

    Ok(WeatherData {
        current: Some(current),
        hourly: Some(hourly),
        daily: Some(daily),
        alerts: Some(alerts),
        aqi_value: Some(format!("{:.0}", svc_aqi.value)),
        aqi_label: Some(svc_aqi.label),
        aqi_level: Some(svc_aqi.level),
        error: None,
        source: FROM_SERVICE,
        degraded: None,
    })
}

fn fetch_weather_direct(lat: f64, lon: f64, location_name: &str, use_fahrenheit: bool) -> WeatherData {
    let temp_unit = if use_fahrenheit { "&temperature_unit=fahrenheit" } else { "" };
    let wind_unit = if use_fahrenheit { "&wind_speed_unit=mph" } else { "" };

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

    let resp = match ureq::get(&url).call() {
        Ok(r) => r,
        Err(e) => {
            return WeatherData {
                error: Some(format!("API request failed: {e}")),
                source: FROM_DIRECT,
                ..Default::default()
            };
        }
    };

    let body: String = match resp.into_string() {
        Ok(b) => b,
        Err(e) => {
            return WeatherData {
                error: Some(format!("Read error: {e}")),
                source: FROM_DIRECT,
                ..Default::default()
            };
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return WeatherData {
                error: Some(format!("JSON parse error: {e}")),
                source: FROM_DIRECT,
                ..Default::default()
            };
        }
    };

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
    let uv_index = daily_obj["uv_index_max"].as_array()
        .and_then(|a| a.first()).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let sunrise = daily_obj["sunrise"].as_array()
        .and_then(|a| a.first()).and_then(|v| v.as_str())
        .map(|s| extract_time(s)).unwrap_or_else(|| "--".to_string());
    let sunset = daily_obj["sunset"].as_array()
        .and_then(|a| a.first()).and_then(|v| v.as_str())
        .map(|s| extract_time(s)).unwrap_or_else(|| "--".to_string());

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
        dew_point: "".into(),
        sunrise: sunrise.into(),
        sunset: sunset.into(),
        is_day,
        is_loading: false,
        error_text: "".into(),
    };

    // Parse hourly
    let hourly_obj = &json["hourly"];
    let mut hourly = Vec::new();
    let precip_probs = hourly_obj["precipitation_probability"].as_array();
    if let (Some(times), Some(temps), Some(codes)) = (
        hourly_obj["time"].as_array(),
        hourly_obj["temperature_2m"].as_array(),
        hourly_obj["weather_code"].as_array(),
    ) {
        let current_hour = current_obj["time"].as_str()
            .map(|s| extract_hour(s)).unwrap_or(0);
        for i in 0..times.len().min(48) {
            let hour = extract_hour(times[i].as_str().unwrap_or(""));
            let t = temps[i].as_f64().unwrap_or(0.0);
            let code = codes[i].as_i64().unwrap_or(0) as i32;
            let is_day_hour = (6..20).contains(&hour);
            let is_current = i == current_hour;
            if i >= current_hour && hourly.len() < 24 {
                let chance = precip_probs
                    .and_then(|a| a.get(i))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                hourly.push(WeatherHourly {
                    time: if is_current { "Now".into() }
                          else { format!("{}:00", hour).into() },
                    icon: wmo_icon(code, is_day_hour).into(),
                    temp: format!("{:.0}\u{00B0}", t).into(),
                    precip: if chance > 0 { format!("{}%", chance).into() }
                            else { "".into() },
                    is_current,
                    temp_value: t as f32,
                    // Filled once the whole strip is known, just below.
                    t_min: 0.0,
                    t_max: 0.0,
                });
            }
        }
    }

    // Each column draws where its hour sits between the strip's coldest and
    // warmest, so the range can only be stamped once every hour is collected.
    let h_min = hourly.iter().map(|h| h.temp_value).fold(f32::MAX, f32::min);
    let h_max = hourly.iter().map(|h| h.temp_value).fold(f32::MIN, f32::max);
    for h in hourly.iter_mut() {
        h.t_min = h_min;
        h.t_max = h_max;
    }

    // Parse daily
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
            if lo < g_min { g_min = lo; }
            if hi > g_max { g_max = hi; }
        }
        let range = (g_max - g_min).max(1.0);

        for i in 0..dates.len().min(5) {
            let date_str = dates[i].as_str().unwrap_or("");
            let code = codes[i].as_i64().unwrap_or(0) as i32;
            let hi = maxes[i].as_f64().unwrap_or(0.0);
            let lo = mins[i].as_f64().unwrap_or(0.0);
            let precip = precips[i].as_f64().unwrap_or(0.0);
            let day_name = if i == 0 { "Today".to_string() } else { day_of_week(date_str) };
            daily.push(WeatherDaily {
                day_name: day_name.into(),
                icon: wmo_icon(code, true).into(),
                high: format!("{:.0}\u{00B0}", hi).into(),
                low: format!("{:.0}\u{00B0}", lo).into(),
                precip_chance: if precip > 0.0 { format!("{:.0}mm", precip).into() }
                               else { "0mm".into() },
                high_value: hi as f32,
                low_value: lo as f32,
                temp_range_min: ((lo - g_min) / range) as f32,
                temp_range_max: ((hi - g_min) / range) as f32,
            });
        }
    }

    WeatherData {
        current: Some(current),
        hourly: Some(hourly),
        daily: Some(daily),
        alerts: Some(Vec::new()),
        aqi_value: Some("--".to_string()),
        aqi_label: Some("N/A".to_string()),
        aqi_level: Some(0),
        error: None,
        source: FROM_DIRECT,
        degraded: None,
    }
}

// ── Geocoding ────────────────────────────────────────────────────────

/// How long a single geocoding attempt may take.
///
/// This is not a guess about a good network. An `app.act` handler runs on the UI thread and the
/// RPC side stops waiting after three seconds, so a lookup that outlasts that budget is reported
/// to the caller as "the app did not answer" — while the location goes on being added a moment
/// later. That is the same fabricated outcome in reverse, and the fix is for the lookup to fit
/// inside the budget or say it could not. Two attempts at 1.2s leaves room for the rest.
/// The direct call previously had no timeout at all and could hang the window indefinitely.
const GEOCODE_TIMEOUT: Duration = Duration::from_millis(1200);

/// Why a place name could not be turned into coordinates.
///
/// The distinction is the point. "There is no such place" is the person's typo and they can fix
/// it; "the geocoder could not be reached" is the machine's problem and the same spelling will
/// work later. Collapsing both into `None` — which is what `.ok()?` did — meant the app could
/// not tell them apart, so it told the caller neither.
enum GeocodeFailure {
    NotFound(String),
    Unreachable(String),
}

impl std::fmt::Display for GeocodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(q) => write!(f, "no place called \"{q}\" was found"),
            Self::Unreachable(why) => write!(f, "the geocoder could not be reached: {why}"),
        }
    }
}

fn geocode_location(query: &str) -> Result<SavedLocation, GeocodeFailure> {
    // The service first, because it is the one owner of this domain when it is up. Its "not
    // found" is an answer and is taken as one; anything else means it did not answer at all,
    // and Open-Meteo is asked directly rather than reporting a missing service as a missing city.
    match geocode_via_service(query) {
        Ok(loc) => {
            return Ok(SavedLocation {
                name: loc.name,
                lat: loc.lat,
                lon: loc.lon,
            })
        }
        Err(e) if e.contains("not found") => {
            return Err(GeocodeFailure::NotFound(query.to_string()))
        }
        Err(e) => tracing::debug!(error = %e, "weather service did not geocode; asking Open-Meteo"),
    }

    let encoded = query.replace(' ', "+");
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        encoded
    );
    let resp = ureq::get(&url)
        .timeout(GEOCODE_TIMEOUT)
        .call()
        .map_err(|e| GeocodeFailure::Unreachable(e.to_string()))?;
    let body: String = resp
        .into_string()
        .map_err(|e| GeocodeFailure::Unreachable(e.to_string()))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| GeocodeFailure::Unreachable(format!("its answer did not parse: {e}")))?;

    // An empty `results` is how Open-Meteo says it knows of no such place, and a name with no
    // coordinates is not a location whatever else it is. Both are the person's answer, not the
    // network's.
    let first = json["results"]
        .as_array()
        .and_then(|r| r.first().cloned())
        .ok_or_else(|| GeocodeFailure::NotFound(query.to_string()))?;
    let (lat, lon) = match (first["latitude"].as_f64(), first["longitude"].as_f64()) {
        (Some(lat), Some(lon)) => (lat, lon),
        _ => return Err(GeocodeFailure::NotFound(query.to_string())),
    };
    let name = first["name"].as_str().unwrap_or(query).to_string();
    let country = first["country"].as_str().unwrap_or("");
    let resolved = if country.is_empty() { name } else { format!("{}, {}", name, country) };
    Ok(SavedLocation { name: resolved, lat, lon })
}

fn geocode_via_service(query: &str) -> Result<yantrik_ipc_contracts::weather::Location, String> {
    let client = SyncRpcClient::for_service("weather").with_timeout(GEOCODE_TIMEOUT);
    let result = client.call("weather.geocode", serde_json::json!({ "query": query }))
        .map_err(|e| e.message)?;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

// ── The one path for each change worth keeping ───────────────────────

/// What actually happened when a place was added, rather than what was asked for.
///
/// The name and the coordinates are the geocoder's: "paris" comes back as "Paris, France" at
/// 48.85/2.35, and a caller told only what it typed has learned nothing about what was stored.
struct Added {
    name: String,
    lat: f64,
    lon: f64,
}

/// Look a place up, keep it, and show it — or say which of those did not happen.
///
/// The single path behind the panel's Add button and the `add_location` action, so neither can
/// report an outcome it did not get. The action used to answer `{"added": name}` in the
/// statement after `invoke_weather_add_location`, and the callback under it was
/// `if let Some(…) = geocode_location(…)` with no else: a misspelt city added nothing at all,
/// silently, and the caller was told it had worked.
///
/// Free of Slint so that both callers can use it from the thread they are already on — the
/// button from a worker, the action inline.
fn add_location(state: &WeatherState, query: &str) -> Result<Added, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("a location needs a name".into());
    }
    let found = geocode_location(query).map_err(|e| e.to_string())?;
    let name = found.name.clone();
    let (lat, lon) = (found.lat, found.lon);
    state.add_location(found)?;
    Ok(Added { name, lat, lon })
}

/// Show temperatures in the other scale, and remember that.
///
/// Both halves in the order the switch does them, and the store written before anyone is told.
/// The Slint toggle flips `weather-use-fahrenheit` itself and then calls the callback, so the
/// property is set here as well: setting it only when it differs keeps the action idempotent —
/// asking twice for fahrenheit must not land back on celsius.
fn apply_units(
    ui: &WeatherApp,
    state: &WeatherState,
    slot: &Arc<Mutex<Option<WeatherData>>>,
    fahrenheit: bool,
) -> Result<bool, String> {
    state.set_fahrenheit(fahrenheit)?;
    if ui.get_weather_use_fahrenheit() != fahrenheit {
        ui.set_weather_use_fahrenheit(fahrenheit);
    }
    let loc = state.active_location();
    fetch_weather_async(slot.clone(), loc.lat, loc.lon, &loc.name, fahrenheit);
    let mut c = ui.get_current();
    c.is_loading = true;
    ui.set_current(c);
    Ok(fahrenheit)
}

/// Redraw the saved-locations panel from the store, which is the only thing that knows.
fn show_saved_locations(ui: &WeatherApp, state: &WeatherState) {
    ui.set_weather_saved_locations(ModelRc::new(VecModel::from(to_slint_locations(state))));
}

/// Show one of the saved places, behind both the list row and the `show_location` action.
///
/// Which place is being shown is in the prefs file and is restored on start, so it has to be
/// written when it changes; it never was, and every restart came back to whichever location the
/// file had recorded last — which, before this change, was none of them.
fn select_location(
    ui: &WeatherApp,
    state: &WeatherState,
    slot: &Arc<Mutex<Option<WeatherData>>>,
    idx: usize,
) -> Result<SavedLocation, String> {
    let loc = state.select_location(idx)?;
    show_saved_locations(ui, state);
    let mut c = ui.get_current();
    c.is_loading = true;
    ui.set_current(c);
    fetch_weather_async(slot.clone(), loc.lat, loc.lon, &loc.name, state.is_fahrenheit());
    Ok(loc)
}

/// Forget a saved place, behind the list row's delete button.
///
/// Returns which one went, so the notice on a failure can name it. The old handler returned
/// early on a bad index and on the last remaining location without a word either way.
fn remove_location(
    ui: &WeatherApp,
    state: &WeatherState,
    slot: &Arc<Mutex<Option<WeatherData>>>,
    idx: usize,
) -> Result<SavedLocation, String> {
    let was_active = state.active_index() == idx;
    let removed = state.remove_location(idx)?;
    show_saved_locations(ui, state);
    if was_active {
        let loc = state.active_location();
        let mut c = ui.get_current();
        c.is_loading = true;
        ui.set_current(c);
        fetch_weather_async(slot.clone(), loc.lat, loc.lon, &loc.name, state.is_fahrenheit());
    }
    Ok(removed)
}

// ── Async fetch ──────────────────────────────────────────────────────

fn fetch_weather_async(
    slot: Arc<Mutex<Option<WeatherData>>>,
    lat: f64, lon: f64, location_name: &str, use_fahrenheit: bool,
) {
    let name = location_name.to_string();
    std::thread::spawn(move || {
        let data = match fetch_via_service(lat, lon, &name, use_fahrenheit) {
            Ok(d) => d,
            Err(e) => {
                // Falling back is right: Open-Meteo is the same source the service reads, and a
                // stopped service is no reason to show nothing. Keeping the reason is the part
                // that was missing — it is what turns a silent degrade into something the
                // person and `describe` can both see.
                tracing::warn!(error = %e, "weather service did not answer; reading Open-Meteo directly");
                let mut d = fetch_weather_direct(lat, lon, &name, use_fahrenheit);
                d.degraded = Some(e);
                d
            }
        };
        *slot.lock().unwrap() = Some(data);
    });
}

// ── Apply data to UI ─────────────────────────────────────────────────

fn apply_weather_data(ui: &WeatherApp, data: WeatherData) {
    // Where these numbers came from, said once on screen. Cleared again only when this same
    // message is what is showing: a notice about a location that could not be saved outlives a
    // refresh, and a good reading has no business wiping it.
    match data.degraded {
        Some(ref why) => ui.set_notice(
            format!("{DEGRADED_NOTICE} — the weather service did not answer ({why}).").into(),
        ),
        None if ui.get_notice().starts_with(DEGRADED_NOTICE) => ui.set_notice(SharedString::new()),
        None => {}
    }
    if let Some(current) = data.current { ui.set_current(current); }
    if let Some(hourly) = data.hourly { ui.set_hourly(ModelRc::new(VecModel::from(hourly))); }
    if let Some(daily) = data.daily { ui.set_daily(ModelRc::new(VecModel::from(daily))); }
    if let Some(ref alerts) = data.alerts {
        let mut most_severe: Option<&WeatherAlert> = None;
        let mut max_sev = -1;
        for a in alerts.iter() {
            if a.severity > max_sev { max_sev = a.severity; most_severe = Some(a); }
        }
        if let Some(alert) = most_severe {
            ui.set_weather_severe_alert_active(true);
            ui.set_weather_severe_alert_text(alert.description.clone());
            let level = match alert.severity { 0 => "watch", 1 => "warning", _ => "emergency" };
            ui.set_weather_severe_alert_level(SharedString::from(level));
        } else {
            ui.set_weather_severe_alert_active(false);
            ui.set_weather_severe_alert_text(SharedString::default());
            ui.set_weather_severe_alert_level(SharedString::default());
        }
        ui.set_weather_alerts(ModelRc::new(VecModel::from(alerts.clone())));
    }
    if let Some(ref val) = data.aqi_value { ui.set_weather_aqi_value(SharedString::from(val.as_str())); }
    if let Some(ref label) = data.aqi_label { ui.set_weather_aqi_label(SharedString::from(label.as_str())); }
    if let Some(level) = data.aqi_level { ui.set_weather_aqi_level(level); }
    if let Some(error) = data.error {
        let mut c = ui.get_current();
        c.error_text = error.into();
        c.is_loading = false;
        ui.set_current(c);
    }
}

// ── Wire callbacks ───────────────────────────────────────────────────

// ── The control surface ──────────────────────────────────────────────
//
// There is a `get_weather` tool that calls an API. This is different and worth having as well:
// it reports what the person is actually looking at — their location, their units, the alert on
// their screen — rather than what a fresh query would return.

fn publish_control(
    app: &WeatherApp,
    state: WeatherState,
    slot: Arc<Mutex<Option<WeatherData>>>,
) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let st = state.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Weather — closing");
            };
            let now = ui.get_current();

            if !now.error_text.is_empty() {
                return View::new(format!("Weather — {}", now.error_text))
                    .with("error", now.error_text.to_string())
                    .with("notice", ui.get_notice().to_string())
                    .with("reading_from", reading_from(&st));
            }

            let alert_model = ui.get_weather_alerts();
            let alerts: Vec<serde_json::Value> = (0..alert_model.row_count())
                .filter_map(|i| alert_model.row_data(i))
                .map(|a| {
                    serde_json::json!({
                        "title": a.title.to_string(),
                        "description": a.description.to_string(),
                        "severity": match a.severity { 2 => "severe", 1 => "moderate", _ => "info" },
                    })
                })
                .collect();

            let daily_model = ui.get_daily();
            let forecast: Vec<serde_json::Value> = (0..daily_model.row_count().min(7))
                .filter_map(|i| daily_model.row_data(i))
                .map(|d| {
                    serde_json::json!({
                        "day": d.day_name.to_string(),
                        "high": d.high.to_string(),
                        "low": d.low.to_string(),
                        "precipitation": d.precip_chance.to_string(),
                    })
                })
                .collect();

            let saved_model = ui.get_weather_saved_locations();
            let saved: Vec<serde_json::Value> = (0..saved_model.row_count())
                .filter_map(|i| saved_model.row_data(i))
                .map(|l| {
                    serde_json::json!({ "name": l.name.to_string(), "active": l.is_active })
                })
                .collect();

            let summary = if now.is_loading {
                format!("Weather — loading {}", now.location)
            } else if let Some(first) = alerts.first() {
                format!(
                    "Weather — {} in {}, {} — {}",
                    now.temperature,
                    now.location,
                    now.condition,
                    first["title"].as_str().unwrap_or_default()
                )
            } else {
                format!(
                    "Weather — {} in {}, {}, feels like {}",
                    now.temperature, now.location, now.condition, now.feels_like
                )
            };

            View::new(summary)
                .with("location", now.location.to_string())
                .with("temperature", now.temperature.to_string())
                .with("feels_like", now.feels_like.to_string())
                .with("condition", now.condition.to_string())
                .with("humidity", now.humidity.to_string())
                .with("wind", format!("{} {}", now.wind_speed, now.wind_direction).trim().to_string())
                .with("uv_index", now.uv_index.to_string())
                .with("visibility", now.visibility.to_string())
                .with("pressure", now.pressure.to_string())
                .with("sunrise", now.sunrise.to_string())
                .with("sunset", now.sunset.to_string())
                .with("air_quality", format!("{} ({})", ui.get_weather_aqi_value(), ui.get_weather_aqi_label()))
                .with("units", if ui.get_weather_use_fahrenheit() { "fahrenheit" } else { "celsius" })
                .with("last_updated", ui.get_weather_last_updated().to_string())
                .with("alerts", serde_json::Value::Array(alerts))
                .with("forecast", serde_json::Value::Array(forecast))
                .with("saved_locations", serde_json::Value::Array(saved))
                // Which of the two paths produced what is on screen. A caller reading these
                // temperatures is entitled to know the weather service did not produce them.
                .with("reading_from", reading_from(&st))
                .with("config_file", st.path().display().to_string())
                // What the person is being told went wrong, if anything. A caller that just
                // failed to add a location should read the reason rather than infer it.
                .with("notice", ui.get_notice().to_string())
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Weather window is gone".to_string());

    let refresh_ui = ui_for.clone();
    let select_ui = ui_for.clone();
    let add_ui = ui_for.clone();
    let units_ui = ui_for;

    let select_state = state.clone();
    let add_state = state.clone();
    let units_state = state;
    let select_slot = slot.clone();
    let add_slot = slot.clone();
    let units_slot = slot;

    App::new("weather")
        .describe(describe)
        .action(Action::new("refresh", "Fetch the current conditions again"), move |_| {
            let ui = refresh_ui()?;
            ui.invoke_refresh_pressed();
            Ok(serde_json::json!({ "refreshing": ui.get_current().location.to_string() }))
        })
        .action(
            // Grade: standard. It moves which saved place is on screen and writes that choice
            // to this app's own prefs file. Nothing outside Weather changes and the previous
            // place is one call away, which is what keeps it below `sensitive`.
            Action::new("show_location", "Switch to one of the saved locations")
                .risk("standard")
                .arg(Param::text("name")),
            move |args| {
                let ui = select_ui()?;
                let want = args["name"].as_str().unwrap_or_default().trim().to_lowercase();
                let saved = select_state.locations();
                let row = saved
                    .iter()
                    .position(|l| l.name.to_lowercase().contains(&want))
                    .ok_or_else(|| {
                        let names: Vec<String> = saved.iter().map(|l| l.name.clone()).collect();
                        if names.is_empty() {
                            "no locations are saved yet; add one first".to_string()
                        } else {
                            format!("no saved location matches \"{want}\"; there is: {}", names.join(", "))
                        }
                    })?;
                let loc = match select_location(&ui, &select_state, &select_slot, row) {
                    Ok(loc) => loc,
                    Err(e) => {
                        ui.set_notice(format!("Could not switch location: {e}").into());
                        return Err(e);
                    }
                };
                ui.set_notice(SharedString::new());
                Ok(serde_json::json!({ "showing": loc.name, "lat": loc.lat, "lon": loc.lon }))
            },
        )
        .action(
            // Grade: sensitive. The geocoding lookup is one public request, and a place added
            // in error is one removal away — but what the action leaves behind is a line in
            // this app's prefs file, and that line outlives the turn that wrote it: the place
            // is still saved when the window closes and after the machine restarts. Writing
            // stored configuration is something the person should see first, however small
            // the thing written; `standard` is for changes the moment takes back. `show_location`
            // stays `standard` beside it because it only moves a saved place onto the screen —
            // the prefs line it touches is which place was being shown, not what is saved.
            Action::new("add_location", "Look a place up and save it")
                .risk("sensitive")
                .arg(Param::text("name")),
            move |args| {
                let ui = add_ui()?;
                let name = args["name"].as_str().unwrap_or_default().trim().to_string();
                if name.is_empty() {
                    return Err("`name` is empty".into());
                }
                // The same path the panel's Add button takes, run to completion before this
                // answers. It used to hand the name to the window and report success in the
                // next statement, while the lookup was still in flight on another thread.
                let added = match add_location(&add_state, &name) {
                    Ok(a) => a,
                    Err(e) => {
                        ui.set_notice(format!("Could not add \u{201C}{name}\u{201D}: {e}").into());
                        return Err(e);
                    }
                };
                ui.set_notice(SharedString::new());
                show_saved_locations(&ui, &add_state);
                let loc = add_state.active_location();
                fetch_weather_async(add_slot.clone(), loc.lat, loc.lon, &loc.name, add_state.is_fahrenheit());
                let mut c = ui.get_current();
                c.is_loading = true;
                ui.set_current(c);
                // What the geocoder resolved, not what was asked for, and the file it is now in.
                Ok(serde_json::json!({
                    "added": added.name,
                    "lat": added.lat,
                    "lon": added.lon,
                    "saved_to": add_state.path().display().to_string(),
                    "saved_locations": add_state.locations().len(),
                }))
            },
        )
        .action(
            // Grade: sensitive. The flip on screen is instant and reversible, but the choice
            // is also written to this app's prefs file, and that file decides the units every
            // future reading arrives in, across restarts. The effect outlives the turn that
            // made it, which is what puts a stored setting above `standard`; and `safe` in
            // this vocabulary stays for reads.
            Action::new("set_units", "Show temperatures in Celsius or Fahrenheit")
                .risk("sensitive")
                .arg(Param::text("units").describe("celsius | fahrenheit")),
            move |args| {
                let ui = units_ui()?;
                let want_f = match args["units"].as_str().unwrap_or_default().to_lowercase().as_str() {
                    "fahrenheit" | "f" | "imperial" => true,
                    "celsius" | "c" | "metric" => false,
                    other => return Err(format!("units are celsius or fahrenheit, not `{other}`")),
                };
                if let Err(e) = apply_units(&ui, &units_state, &units_slot, want_f) {
                    ui.set_notice(format!("Could not keep the unit choice: {e}").into());
                    return Err(e);
                }
                ui.set_notice(SharedString::new());
                Ok(serde_json::json!({
                    "units": if want_f { "fahrenheit" } else { "celsius" },
                    "saved_to": units_state.path().display().to_string(),
                }))
            },
        )
        .serve();
}

/// Where the readings on screen came from, in words a caller can read.
fn reading_from(state: &WeatherState) -> String {
    match state.reading_source() {
        FROM_SERVICE => "weather-service".to_string(),
        FROM_DIRECT => "open-meteo (direct; the weather service did not answer)".to_string(),
        _ => "nothing has been fetched yet".to_string(),
    }
}

fn wire(app: &WeatherApp) {
    let data_slot: Arc<Mutex<Option<WeatherData>>> = Arc::new(Mutex::new(None));
    let state = WeatherState::load(WeatherState::default_path());

    show_saved_locations(app, &state);
    app.set_weather_use_fahrenheit(state.is_fahrenheit());

    publish_control(app, state.clone(), data_slot.clone());

    // ── Refresh ──
    {
        let slot = data_slot.clone();
        let weak = app.as_weak();
        let st = state.clone();
        app.on_refresh_pressed(move || {
            let loc = st.active_location();
            let fahrenheit = st.is_fahrenheit();
            fetch_weather_async(slot.clone(), loc.lat, loc.lon, &loc.name, fahrenheit);
            if let Some(ui) = weak.upgrade() {
                let mut c = ui.get_current();
                c.is_loading = true;
                ui.set_current(c);
            }
        });
    }

    // ── Select location ──
    {
        let slot = data_slot.clone();
        let st = state.clone();
        let weak = app.as_weak();
        app.on_weather_select_location(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            match select_location(&ui, &st, &slot, idx.max(0) as usize) {
                Ok(_) => ui.set_notice(SharedString::new()),
                Err(e) => ui.set_notice(format!("Could not switch location: {e}").into()),
            }
        });
    }

    // ── Add location ──
    {
        let st = state.clone();
        let slot = data_slot.clone();
        let weak = app.as_weak();
        app.on_weather_add_location(move |name| {
            let query = name.to_string().trim().to_string();
            if query.is_empty() { return; }
            let st_c = st.clone();
            let slot_c = slot.clone();
            let ui_w = weak.clone();
            // On a worker, because the lookup is a network call and this is the UI thread.
            // The same `add_location` the action calls, so the button and the mind cannot
            // drift apart about what "added" means.
            std::thread::spawn(move || {
                let outcome = add_location(&st_c, &query);
                let rows = to_slint_locations(&st_c);
                let fetch = outcome.is_ok().then(|| st_c.active_location());
                if let Some(loc) = &fetch {
                    fetch_weather_async(slot_c, loc.lat, loc.lon, &loc.name, st_c.is_fahrenheit());
                }
                let _ = ui_w.upgrade_in_event_loop(move |ui| match outcome {
                    Ok(_) => {
                        ui.set_notice(SharedString::new());
                        ui.set_weather_saved_locations(ModelRc::new(VecModel::from(rows)));
                        let mut c = ui.get_current(); c.is_loading = true; ui.set_current(c);
                    }
                    // Said on screen, because the panel's input has already cleared itself and
                    // the person would otherwise be looking at a list their city is not in
                    // with nothing to explain why.
                    Err(e) => ui.set_notice(
                        format!("Could not add \u{201C}{query}\u{201D}: {e}").into(),
                    ),
                });
            });
        });
    }

    // ── Remove location ──
    {
        let st = state.clone();
        let slot = data_slot.clone();
        let weak = app.as_weak();
        app.on_weather_remove_location(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            match remove_location(&ui, &st, &slot, idx.max(0) as usize) {
                Ok(_) => ui.set_notice(SharedString::new()),
                Err(e) => ui.set_notice(format!("Could not remove that location: {e}").into()),
            }
        });
    }

    // ── Toggle units ──
    {
        let st = state.clone();
        let slot = data_slot.clone();
        let weak = app.as_weak();
        app.on_weather_toggle_units(move || {
            let Some(ui) = weak.upgrade() else { return };
            // The toggle has already flipped the property; `apply_units` is given what it now
            // says and is the same path `set_units` takes.
            let want = ui.get_weather_use_fahrenheit();
            match apply_units(&ui, &st, &slot, want) {
                Ok(_) => ui.set_notice(SharedString::new()),
                Err(e) => {
                    // The choice did not stick, so the switch must not look as though it did.
                    ui.set_weather_use_fahrenheit(st.is_fahrenheit());
                    ui.set_notice(format!("Could not keep the unit choice: {e}").into());
                }
            }
        });
    }

    // ── Back pressed ──
    //
    // There is nowhere to go back to from a window of its own, which is why this wrapper sets
    // `show-back: false` and the arrow is not drawn. The handler stays as the explicit no-op
    // for a control this window does not have; the shell's embedding of the same screen, where
    // back does mean something, wires its own.
    app.on_back_pressed(|| {});

    // ── AI Insights ──
    {
        let weak = app.as_weak();
        app.on_ai_explain_pressed(move || {
            let Some(ui) = weak.upgrade() else { return };
            let c = ui.get_current();
            // Nothing to be insightful about yet. Saying so beats asking a model to comment on
            // three empty strings and presenting whatever it invents as a reading of the sky.
            if c.temperature.is_empty() || c.is_loading {
                ui.set_ai_response("There is no reading yet — refresh first.".into());
                return;
            }
            if let Some(hint) = companion::reach().hint() {
                ui.set_ai_response(hint.into());
                return;
            }
            ui.set_ai_is_working(true);
            ui.set_ai_response(SharedString::new());
            let prompt = conditions_prompt(&c);
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
                    match outcome {
                        Ok(text) => ui.set_ai_response(text.into()),
                        Err(e) => {
                            tracing::warn!(error = %e, "companion call failed");
                            ui.set_ai_response(e.to_string().into());
                        }
                    }
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_ai_dismiss(move || {
            if let Some(ui) = weak.upgrade() { ui.set_ai_panel_open(false); }
        });
    }

    // ── Poll timer ──
    let slot_poll = data_slot.clone();
    let ui_weak_poll = app.as_weak();
    let state_poll = state.clone();

    // Initial fetch
    {
        let loc = state.active_location();
        fetch_weather_async(slot_poll.clone(), loc.lat, loc.lon, &loc.name, state.is_fahrenheit());
    }

    let fetch_interval = std::cell::Cell::new(0u32);
    const REFETCH_TICKS: u32 = 30 * 60 * 10;
    const UPDATE_TEXT_TICKS: u32 = 300;

    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        {
            let mut slot = slot_poll.lock().unwrap();
            if let Some(data) = slot.take() {
                state_poll.record_fetch_time();
                // Remembered here rather than inferred later: by the time `describe` is asked,
                // the reading is just numbers and nothing else says which path produced them.
                state_poll.set_reading_source(data.source);
                if let Some(ui) = ui_weak_poll.upgrade() {
                    apply_weather_data(&ui, data);
                    ui.set_weather_last_updated(SharedString::from(state_poll.last_updated_text()));
                }
            }
        }
        let count = fetch_interval.get() + 1;
        fetch_interval.set(count);
        if count % UPDATE_TEXT_TICKS == 0 {
            if let Some(ui) = ui_weak_poll.upgrade() {
                ui.set_weather_last_updated(SharedString::from(state_poll.last_updated_text()));
            }
        }
        if count >= REFETCH_TICKS {
            fetch_interval.set(0);
            let loc = state_poll.active_location();
            fetch_weather_async(slot_poll.clone(), loc.lat, loc.lon, &loc.name, state_poll.is_fahrenheit());
        }
    });

    std::mem::forget(poll_timer);
}

// ── Helper functions ─────────────────────────────────────────────────

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
        0 => "Clear sky", 1 => "Mainly clear", 2 => "Partly cloudy", 3 => "Overcast",
        45 => "Fog", 48 => "Depositing rime fog",
        51 => "Light drizzle", 53 => "Moderate drizzle", 55 => "Dense drizzle",
        56 | 57 => "Freezing drizzle",
        61 => "Slight rain", 63 => "Moderate rain", 65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Slight snow", 73 => "Moderate snow", 75 => "Heavy snow", 77 => "Snow grains",
        80 => "Slight rain showers", 81 => "Moderate rain showers", 82 => "Violent rain showers",
        85 => "Slight snow showers", 86 => "Heavy snow showers",
        95 => "Thunderstorm", 96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}

fn wind_direction_str(degrees: f64) -> &'static str {
    let dirs = ["N","NNE","NE","ENE","E","ESE","SE","SSE","S","SSW","SW","WSW","W","WNW","NW","NNW"];
    let idx = ((degrees + 11.25) / 22.5) as usize % 16;
    dirs[idx]
}

fn extract_time(s: &str) -> String {
    if let Some(pos) = s.find('T') { s[pos + 1..].to_string() } else { s.to_string() }
}

fn extract_hour(s: &str) -> usize {
    if let Some(pos) = s.find('T') {
        s[pos + 1..].split(':').next().and_then(|h| h.parse().ok()).unwrap_or(0)
    } else { 0 }
}

fn day_of_week(date_str: &str) -> String {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() != 3 { return date_str.to_string(); }
    let year: i32 = parts[0].parse().unwrap_or(2026);
    let month: u32 = parts[1].parse().unwrap_or(1);
    let day: u32 = parts[2].parse().unwrap_or(1);
    let (y, m) = if month <= 2 { (year - 1, month + 12) } else { (year, month) };
    let q = day as i32;
    let k = y % 100;
    let j = y / 100;
    let h = (q + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 - 2 * j) % 7;
    let h = ((h + 7) % 7) as usize;
    let names = ["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"];
    names[h].to_string()
}

#[cfg(test)]
mod grade_tests {
    //! #48 named this app: `add_location` and `set_units` write choices into
    //! `~/.config/yantrik/weather.json` that outlive the turn — they are still standing when
    //! the window closes and after the machine restarts — and both were graded `standard`,
    //! the grade for what the moment takes back, the same grade as opening a window.
    //!
    //! The handlers need a live Slint window and a geocoder to run, so the grades are pinned
    //! against the source the way `lock_grade_tests` in the shell pins `lock`.
    use std::path::Path;

    /// One action's declaration and handler, from its quoted name to where the next `.action(`
    /// begins (or `.serve()` ends the chain for the last one), as written above the tests.
    fn declaration(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        let quoted = format!("\"{name}\"");
        let from = src
            .find(&quoted)
            .unwrap_or_else(|| panic!("weather no longer publishes {name}"));
        let rest = &src[from..];
        let end = rest
            .find(".action(")
            .or_else(|| rest.find(".serve()"))
            .unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// What the action leaves in the prefs file is graded above what it shows on screen.
    ///
    /// `add_location` stores a place; `set_units` decides the units every future reading
    /// arrives in. Both effects outlive the turn and survive a restart, so both ask first
    /// (#48). `refresh` fetches and changes nothing stored, and `show_location` moves an
    /// already-saved place onto the screen — the prefs line it touches is which place was
    /// being shown, not what is saved — so neither may become a card in `ask` mode.
    #[test]
    fn what_outlives_the_turn_asks_first() {
        for name in ["add_location", "set_units"] {
            let declaration = declaration(name);
            assert!(
                declaration.contains(".risk(\"sensitive\")"),
                "`weather.{name}` must be graded sensitive: it writes a choice into this app's \
                 prefs file that outlives the turn and survives a restart (#48), and a stored \
                 setting graded `standard` runs unasked in the default mode. Declaration as \
                 written:\n{declaration}"
            );
            assert!(
                !declaration.contains(".risk(\"standard\")"),
                "`weather.{name}` is graded standard again (#48). Declaration as \
                 written:\n{declaration}"
            );
        }
        for name in ["refresh", "show_location"] {
            let declaration = declaration(name);
            assert!(
                !declaration.contains(".risk(\"sensitive\")"),
                "`weather.{name}` shows or reads and stores no new setting: fetching again, or \
                 moving an already-saved place onto the screen, is the common person-driven \
                 flow that must not become a card in `ask` mode (#48 keeps show/read/open at \
                 `standard` or `safe`). Declaration as written:\n{declaration}"
            );
        }
        // `show_location` declares its grade rather than taking the default, so the judgement
        // that keeps it below its two writing neighbours stays on the page.
        assert!(
            declaration("show_location").contains(".risk(\"standard\")"),
            "`weather.show_location` declares standard on purpose; if the declaration is gone \
             the reason beside it has gone with it"
        );
    }
}
