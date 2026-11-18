//! The only code that writes the window: the model, drawn, for the parts that changed.

use slint::SharedString;
use yantrik_app::app::Changed;
use yantrik_app::rail::{self, RailView as _};
use yantrik_app::rows;

use crate::model::{Model, PART_NOTICE, PART_PLACES, PART_READING, PART_UPDATED};
use crate::{WeatherAlert, WeatherApp, WeatherCurrent, WeatherDaily, WeatherHourly, WeatherSavedLocation};

pub fn render(ui: &WeatherApp, m: &Model, changed: &Changed) {
    if changed.has(PART_READING) {
        reading(ui, m);
    }
    if changed.has(PART_PLACES) {
        let active = m.prefs.active;
        let places: Vec<WeatherSavedLocation> = m
            .prefs
            .locations
            .iter()
            .enumerate()
            .map(|(i, loc)| WeatherSavedLocation {
                name: SharedString::from(&loc.name),
                lat: loc.lat as f32,
                lon: loc.lon as f32,
                is_active: i == active,
            })
            .collect();
        rows::set(ui.get_weather_saved_locations(), places, |list| ui.set_weather_saved_locations(list));
        if ui.get_weather_use_fahrenheit() != m.prefs.fahrenheit {
            ui.set_weather_use_fahrenheit(m.prefs.fahrenheit);
        }
    }
    if changed.has(PART_NOTICE) {
        ui.set_notice(m.notice.as_str().into());
    }
    if changed.has(PART_UPDATED) || changed.has(PART_READING) {
        let fresh = m.freshness();
        ui.set_freshness(fresh.line.as_str().into());
        ui.set_stale(fresh.stale);
    }
    if changed.has(rail::PART) {
        ui.render_rail(&m.rail);
    }
}

fn reading(ui: &WeatherApp, m: &Model) {
    let r = &m.reading;
    let current = match &r.current {
        Some(c) => WeatherCurrent {
            temperature: c.temperature.as_str().into(),
            feels_like: c.feels_like.as_str().into(),
            condition: c.condition.as_str().into(),
            icon: c.icon.as_str().into(),
            location: c.location.as_str().into(),
            humidity: c.humidity.as_str().into(),
            wind_speed: c.wind_speed.as_str().into(),
            wind_direction: c.wind_direction.as_str().into(),
            uv_index: c.uv_index.as_str().into(),
            visibility: visibility_figure(&c.visibility).into(),
            pressure: c.pressure.as_str().into(),
            cloud_cover: c.cloud_cover.as_str().into(),
            dew_point: c.dew_point.as_str().into(),
            sunrise: c.sunrise.as_str().into(),
            sunset: c.sunset.as_str().into(),
            is_day: c.is_day,
            is_loading: m.loading,
            error_text: r.error.clone().unwrap_or_default().into(),
        },
        None => WeatherCurrent {
            location: m.active().name.as_str().into(),
            is_loading: m.loading,
            error_text: r.error.clone().unwrap_or_default().into(),
            ..ui.get_current()
        },
    };
    ui.set_place_label(place_label(&current.location).into());
    let (number, unit) = split_unit(&current.temperature);
    ui.set_reading_number(number.into());
    ui.set_reading_unit(unit.into());
    ui.set_current(current);

    if r.current.is_some() {
        let now = r.hourly.iter().position(|h| h.is_current).unwrap_or(0);
        ui.set_hourly_now(now as i32);
        let hourly: Vec<WeatherHourly> = r
            .hourly
            .iter()
            .map(|h| WeatherHourly {
                time: h.time.as_str().into(),
                icon: h.icon.as_str().into(),
                temp: h.temp.as_str().into(),
                precip: precip_figure(&h.precip).into(),
                is_current: h.is_current,
                temp_value: h.temp_value,
                t_min: h.t_min,
                t_max: h.t_max,
            })
            .collect();
        rows::set(ui.get_hourly(), hourly, |list| ui.set_hourly(list));
        let daily: Vec<WeatherDaily> = r
            .daily
            .iter()
            .map(|d| WeatherDaily {
                day_name: d.day_name.as_str().into(),
                icon: d.icon.as_str().into(),
                high: d.high.as_str().into(),
                low: d.low.as_str().into(),
                precip_chance: if is_nothing(&d.precip_chance) { SharedString::new() } else { d.precip_chance.as_str().into() },
                high_value: d.high_value,
                low_value: d.low_value,
                temp_range_min: d.temp_range_min,
                temp_range_max: d.temp_range_max,
            })
            .collect();
        rows::set(ui.get_daily(), daily, |list| ui.set_daily(list));
        let alerts: Vec<WeatherAlert> = r
            .alerts
            .iter()
            .map(|a| WeatherAlert {
                title: a.title.as_str().into(),
                description: a.description.as_str().into(),
                severity: a.severity,
                icon: a.icon.as_str().into(),
            })
            .collect();
        rows::set(ui.get_weather_alerts(), alerts, |list| ui.set_weather_alerts(list));
        ui.set_air_quality(air_quality_line(&r.aqi.value, &r.aqi.label).into());
        ui.set_air_quality_unhealthy(r.aqi.level >= 2);
    }
}

/// A distance, or `—`: a qualitative word from the source belongs on the condition line, not
/// among measurements (brief §3.2).
fn visibility_figure(v: &str) -> String {
    let v = v.trim();
    if v.is_empty() || v == "--" || !v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        "—".to_string()
    } else {
        v.to_string()
    }
}

/// The place as the title bar names it: the part before the geocoder's `, Country`.
fn place_label(location: &str) -> String {
    location.split(',').next().unwrap_or(location).trim().to_string()
}

/// `16°C` → (`16°`, `C`); a reading without a unit letter keeps its text and an empty unit.
fn split_unit(temperature: &str) -> (String, String) {
    match temperature.strip_suffix('C').or_else(|| temperature.strip_suffix('F')) {
        Some(number) if number.ends_with('°') => (number.to_string(), temperature[number.len()..].to_string()),
        _ => (temperature.to_string(), String::new()),
    }
}

/// The chance of rain is shown from ten percent; under that the slot stays and the figure goes.
fn precip_figure(precip: &str) -> String {
    let percent: u32 = precip.trim_end_matches('%').trim().parse().unwrap_or(0);
    if percent >= 10 {
        precip.to_string()
    } else {
        String::new()
    }
}

/// `0mm`, `0 mm`, `--`, `` : nothing to say.
fn is_nothing(figure: &str) -> bool {
    let f = figure.trim();
    f.is_empty() || f == "--" || f.trim_end_matches("mm").trim() == "0" || f == "0%"
}

/// `Air quality 42 · Good`; nothing when the source gave nothing.
fn air_quality_line(value: &str, label: &str) -> String {
    if value.trim().is_empty() || value.trim() == "--" {
        return String::new();
    }
    format!("Air quality {value} · {label}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_that_say_nothing_are_left_out() {
        assert_eq!(precip_figure("40%"), "40%");
        assert_eq!(precip_figure("5%"), "");
        assert_eq!(precip_figure(""), "");
        assert!(is_nothing("0mm") && is_nothing("0 mm") && is_nothing("--") && is_nothing(""));
        assert!(!is_nothing("2.4mm"));
        assert_eq!(air_quality_line("--", "N/A"), "");
        assert_eq!(air_quality_line("42", "Good"), "Air quality 42 · Good");
        assert_eq!(visibility_figure("Good"), "—");
        assert_eq!(visibility_figure("16 km"), "16 km");
        assert_eq!(place_label("Llanfairpwllgwyngyll, United Kingdom"), "Llanfairpwllgwyngyll");
        assert_eq!(split_unit("16°C"), ("16°".to_string(), "C".to_string()));
        assert_eq!(split_unit("--"), ("--".to_string(), String::new()));
    }
}
