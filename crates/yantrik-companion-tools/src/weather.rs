//! Weather tool — get_weather.
//! Uses wttr.in (free, no API key needed).
//!
//! With no place named, wttr.in finds the caller from its IP address, which tells it that
//! address. So a call that names no place asks first (`location_consent::ASK`) unless the person
//! has already turned location lookups on, and `YANTRIK_NO_GEOLOCATE=1` refuses it outright.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use yantrik_ipc_contracts::location_consent;

/// What a call's place comes to.
#[derive(Debug, PartialEq)]
enum Place {
    /// Ask wttr.in about this place; empty means "where my address says".
    Fetch(String),
    /// Say this to the person instead, and send nothing.
    Say(String),
}

/// Decide the place before any request. `yes` is the call's `my_location` flag, which the model
/// sets only after the person agreed; `on` is the standing switch.
fn resolve(location: &str, yes: bool, forbidden: bool, on: bool) -> Place {
    if !location.is_empty() {
        return Place::Fetch(location.to_string());
    }
    if forbidden {
        return Place::Say("Location lookups are switched off on this machine. Name a city.".into());
    }
    if yes || on {
        return Place::Fetch(String::new());
    }
    Place::Say(format!("No place named. {}", location_consent::ASK))
}

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(GetWeatherTool));
}

// ── Get Weather ──

pub struct GetWeatherTool;

impl Tool for GetWeatherTool {
    fn name(&self) -> &'static str { "get_weather" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "weather" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get current weather and forecast for a location",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "City name or coordinates (e.g. 'London', 'New York', '48.8566,2.3522'). Leave empty only after the person agreed to find their location from their internet address, and then set my_location."
                        },
                        "my_location": {
                            "type": "boolean",
                            "description": "True only if the person said yes to finding their location from their internet address (that address is sent to wttr.in)."
                        },
                        "format": {
                            "type": "string",
                            "enum": ["brief", "full"],
                            "description": "brief = current conditions only, full = 3-day forecast (default: brief)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let location = args.get("location").and_then(|v| v.as_str()).unwrap_or("");
        let yes = args.get("my_location").and_then(|v| v.as_bool()).unwrap_or(false);
        let location = match resolve(location.trim(), yes, location_consent::env_forbids(), location_consent::allowed()) {
            Place::Fetch(place) => place,
            Place::Say(words) => return words,
        };
        let location = location.as_str();
        let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("brief");

        // Validate location — no shell metacharacters
        if location.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&' || c == '<' || c == '>') {
            return "Error: invalid location characters".to_string();
        }

        let url = if format == "full" {
            if location.is_empty() {
                "https://wttr.in/?format=v2".to_string()
            } else {
                format!("https://wttr.in/{}?format=v2", location.replace(' ', "+"))
            }
        } else {
            // Brief: one-line format
            // Format: location, condition, temp, humidity, wind
            let fmt = "%l:+%C+%t+humidity:%h+wind:%w";
            if location.is_empty() {
                format!("https://wttr.in/?format={}", fmt)
            } else {
                format!("https://wttr.in/{}?format={}", location.replace(' ', "+"), fmt)
            }
        };

        match std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "10", &url])
            .env("LANG", "en_US.UTF-8")
            .output()
        {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if text.trim().is_empty() {
                    "Could not fetch weather data.".to_string()
                } else if text.len() > 3000 {
                    format!("{}", &text[..text.floor_char_boundary(3000)])
                } else {
                    text.to_string()
                }
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("Weather fetch failed: {}", err.trim())
            }
            Err(e) => format!("Error (curl not available?): {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_city_is_fetched_without_a_question() {
        assert_eq!(resolve("Oslo", false, false, false), Place::Fetch("Oslo".into()));
    }

    #[test]
    fn no_place_and_no_consent_asks_and_sends_nothing() {
        let Place::Say(words) = resolve("", false, false, false) else { panic!("must ask") };
        assert!(words.contains("internet address") && words.contains("wttr.in"), "{words}");
    }

    #[test]
    fn the_lookup_is_taken_only_after_a_yes_or_the_switch() {
        assert_eq!(resolve("", true, false, false), Place::Fetch(String::new()));
        assert_eq!(resolve("", false, false, true), Place::Fetch(String::new()));
    }

    #[test]
    fn the_environment_refuses_even_after_a_yes() {
        for (yes, on) in [(true, false), (false, true), (true, true)] {
            assert!(matches!(resolve("", yes, true, on), Place::Say(_)), "yes={yes} on={on}");
        }
        assert_eq!(resolve("Oslo", true, true, true), Place::Fetch("Oslo".into()), "a typed city is the person's own words");
    }
}
