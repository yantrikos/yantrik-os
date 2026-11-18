//! A reading: what the screen shows about a place, as plain data, and how one is fetched.
//!
//! The weather service is asked first (one `forecast` call through `yantrik_services`); when it
//! does not answer, Open-Meteo is read directly (`direct`), which is the same source the service
//! reads, and the reason is kept so the person and `describe` can see the degrade.

use yantrik_ipc_contracts::weather::Forecast;
use yantrik_services::weather::{Place, Units, WeatherApi};

use crate::prefs::SavedLocation;

/// Where the numbers came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Nothing,
    Service,
    Direct,
}

impl Source {
    /// The words `describe` says for it.
    pub fn reading_from(self) -> &'static str {
        match self {
            Source::Service => "weather-service",
            Source::Direct => "open-meteo (direct; the weather service did not answer)",
            Source::Nothing => "nothing has been fetched yet",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Current {
    pub temperature: String,
    pub feels_like: String,
    pub condition: String,
    pub icon: String,
    pub location: String,
    pub humidity: String,
    pub wind_speed: String,
    pub wind_direction: String,
    pub uv_index: String,
    pub visibility: String,
    pub pressure: String,
    pub cloud_cover: String,
    pub dew_point: String,
    pub sunrise: String,
    pub sunset: String,
    pub is_day: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hour {
    pub time: String,
    pub icon: String,
    pub temp: String,
    pub precip: String,
    pub is_current: bool,
    pub temp_value: f32,
    pub t_min: f32,
    pub t_max: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Day {
    pub day_name: String,
    pub icon: String,
    pub high: String,
    pub low: String,
    pub precip_chance: String,
    pub high_value: f32,
    pub low_value: f32,
    pub temp_range_min: f32,
    pub temp_range_max: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Alert {
    pub title: String,
    pub description: String,
    /// 0 watch, 1 warning, 2 emergency.
    pub severity: i32,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Aqi {
    pub value: String,
    pub label: String,
    pub level: i32,
}

impl Default for Aqi {
    fn default() -> Aqi {
        Aqi {
            value: "--".into(),
            label: "N/A".into(),
            level: 0,
        }
    }
}

/// Everything the screen shows about a place.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reading {
    pub current: Option<Current>,
    pub hourly: Vec<Hour>,
    pub daily: Vec<Day>,
    pub alerts: Vec<Alert>,
    pub aqi: Aqi,
    /// The fetch did not produce a reading; the sentence to show in its place.
    pub error: Option<String>,
}

impl Reading {
    pub fn failed(sentence: impl Into<String>) -> Reading {
        Reading {
            error: Some(sentence.into()),
            ..Default::default()
        }
    }

    /// The most severe alert, if any.
    pub fn severe(&self) -> Option<&Alert> {
        // The first of equal severities, as the screen always chose it.
        self.alerts.iter().rev().max_by_key(|a| a.severity)
    }
}

/// What a fetch came back with.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub reading: Reading,
    pub source: Source,
    /// Why the service was not used, when it was not.
    pub degraded: Option<String>,
}

/// The reading for `place`: the service, or Open-Meteo directly when it does not answer and
/// `fall_back` allows (a test's app never reaches the internet). Blocking: a job's work.
pub fn fetch(
    weather: &dyn WeatherApi,
    place: &SavedLocation,
    fahrenheit: bool,
    fall_back: bool,
) -> Fetched {
    let units = if fahrenheit { Units::Imperial } else { Units::Metric };
    match weather.forecast(&Place::new(&place.name, place.lat, place.lon), units, 7) {
        Ok(forecast) => Fetched {
            reading: from_forecast(forecast, &place.name, fahrenheit),
            source: Source::Service,
            degraded: None,
        },
        Err(e) if fall_back => {
            tracing::warn!(error = %e, "weather service did not answer; reading Open-Meteo directly");
            Fetched {
                reading: crate::direct::fetch(place.lat, place.lon, &place.name, fahrenheit),
                source: Source::Direct,
                degraded: Some(e.to_string()),
            }
        }
        Err(e) => Fetched {
            reading: Reading::failed(e.to_string()),
            source: Source::Nothing,
            degraded: Some(e.to_string()),
        },
    }
}

/// The service's answer as the screen shows it, in the words the app has always used.
pub fn from_forecast(f: Forecast, location_name: &str, fahrenheit: bool) -> Reading {
    let deg = if fahrenheit { "\u{00B0}F" } else { "\u{00B0}C" };
    let wind_label = if fahrenheit { "mph" } else { "km/h" };
    let today = f.daily.first();
    let c = &f.current;
    let current = Current {
        temperature: format!("{:.0}{deg}", c.temperature),
        feels_like: format!("{:.0}{deg}", c.feels_like),
        condition: c.condition.clone(),
        icon: c.icon.clone(),
        location: location_name.into(),
        humidity: format!("{}%", c.humidity),
        wind_speed: format!("{:.0} {wind_label}", c.wind_speed),
        wind_direction: c.wind_direction.clone(),
        uv_index: format!("{:.0}", c.uv_index),
        visibility: if c.visibility_km >= 10.0 {
            "Excellent"
        } else if c.visibility_km >= 4.0 {
            "Good"
        } else {
            "Reduced"
        }
        .into(),
        pressure: format!("{:.0} hPa", c.pressure_hpa),
        cloud_cover: format!("{}%", c.cloud_cover),
        dew_point: format!("{:.0}{deg}", c.dew_point),
        sunrise: today.map(|d| d.sunrise.clone()).unwrap_or_default(),
        sunset: today.map(|d| d.sunset.clone()).unwrap_or_default(),
        is_day: c.is_day,
    };

    let h_min = f.hourly.iter().map(|h| h.temperature).fold(f64::MAX, f64::min);
    let h_max = f.hourly.iter().map(|h| h.temperature).fold(f64::MIN, f64::max);
    let hourly = f
        .hourly
        .iter()
        .map(|h| Hour {
            time: h.time.clone(),
            icon: h.icon.clone(),
            temp: format!("{:.0}\u{00B0}", h.temperature),
            precip: if h.precipitation_chance > 0 {
                format!("{}%", h.precipitation_chance)
            } else {
                String::new()
            },
            is_current: h.time == "Now",
            temp_value: h.temperature as f32,
            t_min: h_min as f32,
            t_max: h_max as f32,
        })
        .collect();

    let g_min = f.daily.iter().map(|d| d.temp_low).fold(f64::MAX, f64::min);
    let g_max = f.daily.iter().map(|d| d.temp_high).fold(f64::MIN, f64::max);
    let range = (g_max - g_min).max(1.0);
    let daily = f
        .daily
        .iter()
        .enumerate()
        .map(|(i, d)| Day {
            day_name: if i == 0 {
                "Today".to_string()
            } else {
                crate::direct::day_of_week(&d.date)
            },
            icon: d.icon.clone(),
            high: format!("{:.0}\u{00B0}", d.temp_high),
            low: format!("{:.0}\u{00B0}", d.temp_low),
            precip_chance: if d.precipitation_chance > 0 {
                format!("{}%", d.precipitation_chance)
            } else {
                "0mm".into()
            },
            high_value: d.temp_high as f32,
            low_value: d.temp_low as f32,
            temp_range_min: ((d.temp_low - g_min) / range) as f32,
            temp_range_max: ((d.temp_high - g_min) / range) as f32,
        })
        .collect();

    let alerts = f
        .alerts
        .iter()
        .map(|a| Alert {
            title: a.title.clone(),
            description: a.description.clone(),
            severity: match a.severity.as_str() {
                "emergency" => 2,
                "warning" => 1,
                _ => 0,
            },
            icon: "\u{26A0}\u{FE0F}".into(),
        })
        .collect();

    Reading {
        current: Some(current),
        hourly,
        daily,
        alerts,
        aqi: Aqi {
            value: format!("{:.0}", f.air_quality.value),
            label: f.air_quality.label,
            level: f.air_quality.level,
        },
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forecast_is_read_in_the_apps_words() {
        let mut f = yantrik_services::weather::FakeWeather::empty_forecast();
        f.current.temperature = 21.4;
        f.current.visibility_km = 12.0;
        f.alerts.push(yantrik_ipc_contracts::weather::WeatherAlert {
            severity: "warning".into(),
            title: "Wind".into(),
            description: "Gusts".into(),
            expires: String::new(),
        });
        let r = from_forecast(f, "Paris", false);
        let c = r.current.clone().unwrap();
        assert_eq!(c.temperature, "21°C");
        assert_eq!(c.visibility, "Excellent");
        assert_eq!(c.location, "Paris");
        assert_eq!(r.severe().unwrap().severity, 1);
        assert_eq!(Source::Direct.reading_from(), "open-meteo (direct; the weather service did not answer)");
    }
}
