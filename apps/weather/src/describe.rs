//! The Mind's view of Weather: a projection of the committed model, in the same keys and words
//! the app has always answered, so `yos check weather` reads the same.

use serde_json::json;
use yantrik_app::control::View;

use crate::model::Model;

pub fn view(m: &Model) -> View {
    let r = &m.reading;
    if let Some(error) = &r.error {
        return View::new(format!("Weather — {error}"))
            .with("error", error.clone())
            .with("notice", if m.notice.is_empty() { fallback_sentence(m).unwrap_or_default() } else { m.notice.clone() })
            .with("reading_from", m.source.reading_from());
    }
    let now = r.current.clone().unwrap_or_default();
    let location = if now.location.is_empty() {
        m.active().name
    } else {
        now.location.clone()
    };

    let alerts: Vec<serde_json::Value> = r
        .alerts
        .iter()
        .map(|a| {
            json!({
                "title": a.title,
                "description": a.description,
                "severity": match a.severity { 2 => "severe", 1 => "moderate", _ => "info" },
            })
        })
        .collect();
    let forecast: Vec<serde_json::Value> = r
        .daily
        .iter()
        .take(7)
        .map(|d| {
            json!({
                "day": d.day_name,
                "high": d.high,
                "low": d.low,
                "precipitation": d.precip_chance,
            })
        })
        .collect();
    let saved: Vec<serde_json::Value> = m
        .prefs
        .locations
        .iter()
        .enumerate()
        .map(|(i, l)| json!({ "name": l.name, "active": i == m.prefs.active }))
        .collect();

    let summary = if m.loading {
        format!("Weather — loading {location}")
    } else if let Some(first) = alerts.first() {
        format!(
            "Weather — {} in {}, {} — {}",
            now.temperature,
            location,
            now.condition,
            first["title"].as_str().unwrap_or_default()
        )
    } else {
        format!(
            "Weather — {} in {}, {}, feels like {}",
            now.temperature, location, now.condition, now.feels_like
        )
    };

    // The notice the Mind reads: what could not be done, or, under a fallback reading, why the
    // service was not used — the screen says the fallback in its freshness line, `describe`
    // keeps the sentence it always had.
    let notice = if !m.notice.is_empty() {
        m.notice.clone()
    } else {
        fallback_sentence(m).unwrap_or_default()
    };
    View::new(summary)
        .with("location", location)
        .with("temperature", now.temperature)
        .with("feels_like", now.feels_like)
        .with("condition", now.condition)
        .with("humidity", now.humidity)
        .with("wind", format!("{} {}", now.wind_speed, now.wind_direction).trim().to_string())
        .with("uv_index", now.uv_index)
        .with("visibility", now.visibility)
        .with("pressure", now.pressure)
        .with("sunrise", now.sunrise)
        .with("sunset", now.sunset)
        .with("air_quality", format!("{} ({})", r.aqi.value, r.aqi.label))
        .with("units", if m.prefs.fahrenheit { "fahrenheit" } else { "celsius" })
        .with("last_updated", m.last_updated_text())
        .with("alerts", serde_json::Value::Array(alerts))
        .with("forecast", serde_json::Value::Array(forecast))
        .with("saved_locations", serde_json::Value::Array(saved))
        .with("reading_from", m.source.reading_from())
        .with("config_file", m.config_file.clone())
        .with("notice", notice)
}

/// `Readings came straight from Open-Meteo — the weather service did not answer (<why>).` when
/// the reading on screen came by fallback; `None` otherwise.
fn fallback_sentence(m: &Model) -> Option<String> {
    let why = m.degraded.as_ref()?;
    (m.source == crate::forecast::Source::Direct)
        .then(|| format!("{} — the weather service did not answer ({why}).", crate::model::DEGRADED_NOTICE))
}
