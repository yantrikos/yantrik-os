//! Weather Watch instinct — proactive alerts for notable weather conditions.
//!
//! Unlike generic "it's sunny today" small talk, this instinct only fires when
//! weather is actionable: storms, extreme temperatures, rain expected, etc.
//! Fetches weather once per check interval (default 2 hours) and analyzes
//! conditions against alert thresholds.
//!
//! # The alert that fired on a clear day
//!
//! It did not do any of that. It asked wttr.in for `%C %t humidity:%h wind:%w` and then tested
//! the WHOLE reply against a keyword list containing "wind" — so the word it had just asked the
//! server to print back was the word it treated as evidence of a storm. Every check matched.
//! On a machine reporting `Clear +87°F humidity:45% wind:↑9mph` the desktop said
//! "Notable weather: Clear +87°F humidity:45% wind:↑9mph", which is not notable and is not
//! even a sentence — it is the query echoed back with an adjective in front of it.
//!
//! A proactive surface that interrupts on nothing teaches people to ignore it, and this was the
//! ONLY producer feeding the desktop's card region, so it was most of what that surface ever
//! said. Two changes: the reply is parsed into fields before anything is matched, so a label can
//! never be mistaken for a condition, and wind has to actually be strong.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

/// Keywords that indicate actionable weather worth alerting about.
///
/// Matched against the CONDITION field alone — never the whole reply. "wind" belongs here
/// because "Windy" is a condition; it stopped being safe only when it was tested against a
/// string that carried the server's own field labels.
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

/// Conditions that put a person in danger, not merely in the rain: a message about one is a
/// safety signal and is never throttled (#754).
const DANGER_CONDITIONS: &[&str] = &["tornado", "hurricane", "blizzard", "flood"];

fn is_dangerous(condition_lower: &str) -> bool {
    DANGER_CONDITIONS.iter().any(|kw| condition_lower.contains(kw))
}

/// Temperature thresholds (Fahrenheit) that warrant a heads-up.
const TEMP_HOT_F: f64 = 95.0;
const TEMP_COLD_F: f64 = 32.0;

/// Wind worth mentioning, in mph. Below this it is just weather.
///
/// 25 mph is roughly where loose objects start moving and an umbrella stops working — the point
/// at which a person would change what they were about to do, which is the only test that makes
/// something worth interrupting for.
const WIND_ALERT_MPH: f64 = 25.0;

/// What wttr.in is asked for: condition, temperature, humidity, wind — pipe separated.
///
/// Delimited deliberately. The previous format embedded `humidity:` and `wind:` as literal text
/// in the reply, and the reply was then keyword-matched, so the format string and the alert list
/// shared a namespace without anyone noticing.
const WTTR_FORMAT: &str = "%C|%t|%h|%w";

/// The four fields, as they came back.
struct Reading {
    condition: String,
    temp: String,
    wind: String,
}

/// Split the reply. `None` if it is not the shape we asked for — silence beats a false alarm.
fn parse_reading(text: &str) -> Option<Reading> {
    let parts: Vec<&str> = text.split('|').map(str::trim).collect();
    if parts.len() < 4 || parts[0].is_empty() {
        return None;
    }
    Some(Reading {
        condition: parts[0].to_string(),
        temp: parts[1].to_string(),
        wind: parts[3].to_string(),
    })
}

/// Wind speed in mph, whatever unit it came in.
fn wind_mph(wind: &str) -> Option<f64> {
    let lower = wind.to_lowercase();
    let n = extract_number(&lower)?;
    if lower.contains("km/h") || lower.contains("kmph") {
        return Some(n * 0.621_371);
    }
    Some(n)
}

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

        let Some(reading) = parse_reading(&weather) else {
            return vec![];
        };

        // Each field is tested on its own terms.
        let condition_lower = reading.condition.to_lowercase();
        let condition_alert = ALERT_CONDITIONS
            .iter()
            .any(|kw| condition_lower.contains(kw))
            .then(|| reading.condition.clone());
        let temp_alert = parse_temp_alert(&reading.temp.to_lowercase());
        let wind_alert = wind_mph(&reading.wind)
            .filter(|mph| *mph >= WIND_ALERT_MPH)
            .map(|mph| format!("wind at {mph:.0} mph"));

        // Nothing is happening. That is the normal answer and it is not a failure.
        let mut parts: Vec<String> = Vec::new();
        parts.extend(condition_alert);
        parts.extend(temp_alert);
        parts.extend(wind_alert);
        if parts.is_empty() {
            return vec![];
        }

        // Read as a sentence, not as a dump of the reply. What was said before was the raw
        // query result with "Notable weather:" in front of it.
        let alert = parts.join(", ");

        let mut context = serde_json::Map::new();
        context.insert(
            "weather_alert".into(),
            serde_json::Value::String(alert.clone()),
        );
        context.insert(
            "weather_detail".into(),
            serde_json::Value::String(weather.trim().to_string()),
        );

        // Weather that can hurt someone is a safety signal: no throttle holds it (#754). It keeps
        // a cooldown key of its own: on the shared key a tornado only boosted a rain urge already
        // waiting (a stopped Weather kind keeps one waiting), and was skipped for 2 h after rain.
        let dangerous = is_dangerous(&condition_lower);
        let spec = UrgeSpec::new(
            "weather_watch",
            &format!("Notable weather: {}", alert),
            0.55,
        )
        .with_cooldown(if dangerous { DANGER_COOLDOWN_KEY } else { "weather_watch:alert" })
        .with_context(serde_json::Value::Object(context));
        vec![if dangerous { spec.safety() } else { spec }]
    }
}

/// The cooldown key of weather that can hurt someone, apart from ordinary notable weather's.
pub const DANGER_COOLDOWN_KEY: &str = "weather_watch:danger";

/// Fetch brief weather from wttr.in.
fn fetch_weather() -> Option<String> {
    let url = format!("https://wttr.in/?format={WTTR_FORMAT}");
    let url = url.as_str();
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

#[cfg(test)]
mod danger_tests {
    use super::*;

    /// Weather that can hurt someone is a safety signal; rain is not (#754).
    #[test]
    fn only_dangerous_weather_is_a_safety_signal() {
        assert!(is_dangerous("tornado warning"));
        assert!(is_dangerous("flood watch"));
        assert!(!is_dangerous("light rain"));
        assert!(!is_dangerous("windy"));
    }
}
