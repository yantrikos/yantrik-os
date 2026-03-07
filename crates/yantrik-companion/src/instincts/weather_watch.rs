//! Weather Watch instinct — proactive alerts for notable weather conditions.
//!
//! Unlike generic "it's sunny today" small talk, this instinct only fires when
//! weather is actionable: storms, extreme temperatures, rain expected, etc.
//! Fetches weather once per check interval (default 2 hours) and analyzes
//! conditions against alert thresholds.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Keywords that indicate actionable weather worth alerting about.
const ALERT_CONDITIONS: &[&str] = &[
    "rain",
    "storm",
    "thunder",
    "snow",
    "sleet",
    "hail",
    "fog",
    "freezing",
    "ice",
    "tornado",
    "hurricane",
    "wind",
    "blizzard",
    "extreme",
    "advisory",
    "warning",
    "flood",
];

/// Temperature thresholds (Fahrenheit) that warrant a heads-up.
const TEMP_HOT_F: f64 = 95.0;
const TEMP_COLD_F: f64 = 32.0;

pub struct WeatherWatchInstinct {
    /// Seconds between weather checks. Default: 7200 (2 hours).
    check_interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl WeatherWatchInstinct {
    pub fn new() -> Self {
        Self {
            check_interval_secs: 7200.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for WeatherWatchInstinct {
    fn name(&self) -> &str {
        "weather_watch"
    }

    fn evaluate(&self, _state: &CompanionState) -> Vec<UrgeSpec> {
        let now = now_ts();

        // Rate-limit weather API calls
        {
            let last = self.last_check_ts.lock().unwrap();
            if now - *last < self.check_interval_secs {
                return vec![];
            }
        }

        // Update check timestamp
        {
            let mut last = self.last_check_ts.lock().unwrap();
            *last = now;
        }

        // Fetch current weather
        let weather = match fetch_weather() {
            Some(w) => w,
            None => return vec![],
        };

        let lower = weather.to_lowercase();

        // Check for alert-worthy conditions
        let is_alertable = ALERT_CONDITIONS.iter().any(|kw| lower.contains(kw));
        let temp_alert = parse_temp_alert(&lower);

        if !is_alertable && temp_alert.is_none() {
            return vec![];
        }

        // Build the alert message
        let alert = if let Some(temp_msg) = &temp_alert {
            if is_alertable {
                format!("{} and {}", weather.trim(), temp_msg)
            } else {
                temp_msg.clone()
            }
        } else {
            weather.trim().to_string()
        };

        let mut context = serde_json::Map::new();
        context.insert(
            "weather_alert".into(),
            serde_json::Value::String(alert.clone()),
        );
        context.insert(
            "weather_detail".into(),
            serde_json::Value::String(weather.trim().to_string()),
        );

        vec![UrgeSpec::new(
            "weather_watch",
            &format!("Notable weather: {}", alert),
            0.55,
        )
        .with_cooldown("weather_watch:alert")
        .with_context(serde_json::Value::Object(context))]
    }
}

/// Fetch brief weather from wttr.in.
fn fetch_weather() -> Option<String> {
    let url = "https://wttr.in/?format=%C+%t+humidity:%h+wind:%w";
    let output = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "5", "--connect-timeout", "3", url])
        .env("LANG", "en_US.UTF-8")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() || text.contains("Unknown") {
        return None;
    }
    Some(text)
}

/// Try to parse temperature from weather string and check thresholds.
fn parse_temp_alert(weather_lower: &str) -> Option<String> {
    // wttr.in format includes things like "+95°F" or "-5°C"
    // Look for temperature patterns
    for word in weather_lower.split_whitespace() {
        // Try Fahrenheit
        if word.contains('f') || word.contains("°f") {
            if let Some(temp) = extract_number(word) {
                if temp >= TEMP_HOT_F {
                    return Some(format!("it's {:.0}°F — extreme heat", temp));
                }
                if temp <= TEMP_COLD_F {
                    return Some(format!("it's {:.0}°F — freezing conditions", temp));
                }
            }
        }
        // Try Celsius (convert to F for threshold comparison)
        if word.contains('c') || word.contains("°c") {
            if let Some(temp_c) = extract_number(word) {
                let temp_f = temp_c * 9.0 / 5.0 + 32.0;
                if temp_f >= TEMP_HOT_F {
                    return Some(format!("it's {:.0}°C — extreme heat", temp_c));
                }
                if temp_f <= TEMP_COLD_F {
                    return Some(format!("it's {:.0}°C — freezing conditions", temp_c));
                }
            }
        }
    }
    None
}

/// Extract a number (possibly negative, with +/- prefix) from a string like "+95°F".
fn extract_number(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    cleaned.parse().ok()
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
