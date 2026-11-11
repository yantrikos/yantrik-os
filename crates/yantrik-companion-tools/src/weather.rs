//! Weather tool — get_weather.
//! Uses wttr.in (free, no API key needed).
//!
//! With no place named, wttr.in finds the caller from its IP address, which tells it that
//! address. So a call that names no place does not send anything until the person has said yes,
//! and the yes is theirs: the shell puts up its own approval card (`allow_location_lookup`), and
//! only their Allow there writes the switch. The model's arguments cannot carry a yes (there is
//! no such argument), and a turn with nobody at the keyboard leaves the card unanswered, so
//! nothing is sent. `YANTRIK_NO_GEOLOCATE=1` refuses it outright.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use yantrik_companion_core::background_research;
use yantrik_ipc_contracts::location_consent;
use yantrik_companion_core::weather_places;

/// The shell's action that asks the person, and the app it belongs to.
const APP: &str = "shell";
const ACTION: &str = "allow_location_lookup";

/// What a call's place comes to.
#[derive(Debug, PartialEq)]
enum Place {
    /// Ask wttr.in about this place; empty means "where my address says".
    Fetch(String),
    /// Say this to the person instead, and send nothing.
    Say(String),
}

/// Decide the place before any request. `on` is the standing switch; `ask` puts the shell's card
/// in front of the person and is called only when a place is missing and the switch is off.
fn resolve(location: &str, forbidden: bool, on: bool, ask: impl FnOnce() -> Result<(), String>) -> Place {
    if !location.is_empty() {
        return Place::Fetch(location.to_string());
    }
    if forbidden {
        return Place::Say("Location lookups are switched off on this machine. Name a city.".into());
    }
    if on {
        return Place::Fetch(String::new());
    }
    match ask() {
        Ok(()) if location_consent::allowed() => Place::Fetch(String::new()),
        Ok(()) => Place::Say(format!("No place named, and location lookups are still off. {}", location_consent::ASK_WEATHER)),
        Err(why) => Place::Say(format!("No place named. {} {why} Nothing was sent; name a city to get the weather there.", location_consent::ASK_WEATHER)),
    }
}

/// With nobody there, the place goes to wttr.in in the clear, so it must be one the person has: a place
/// they saved in the weather app, the coordinates of one, or where they live (`weather_places`), not text
/// the model chose, however plain it reads. A person's own turn may type anything. `None` lets the call go
/// on.
fn unplace(location: &str, person: bool, theirs: impl Fn(&str) -> bool) -> Option<String> {
    let location = location.trim();
    (!person && !location.is_empty() && !theirs(location)).then(|| {
        "Refused: with nobody asking, the weather is looked up only for a place the person saved or lives in, and that was not one, so nothing was sent."
            .to_string()
    })
}

/// The person's yes, on the shell's own card.
fn ask_the_person() -> Result<(), String> {
    crate::app_ui::act_asking(&crate::app_ui::Desk::session(), APP, ACTION, &serde_json::json!({}), "", "The weather tool was asked about a place and none was named.")
        .map(|_| ())
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
                            "description": "City name or coordinates (e.g. 'London', 'New York', '48.8566,2.3522'). If the person named no place, leave it empty: they are asked on their own screen whether wttr.in may find them from their internet address, and nothing is sent until they say yes."
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
        if let Some(refusal) = unplace(location, background_research::person_started(), weather_places::is_theirs) {
            return refusal;
        }
        // The switch's first yes in this run is logged with the line `describe shell` shows.
        let on = location_consent::allowed_noting(|line| tracing::warn!("{line}"));
        let location = match resolve(location.trim(), location_consent::env_forbids(), on, ask_the_person) {
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
    /// The tool asks the switch through the noting door, so the first yes in a run is logged
    /// whoever wrote the file; no other `allowed()` call is left in the tool's own path.
    #[test]
    fn the_tool_notes_the_switch_when_it_is_on() {
        let production = include_str!("weather.rs").split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("location_consent::allowed_noting("));
        assert_eq!(production.matches("location_consent::allowed()").count(), 1, "only the re-check after the card");
    }

    use super::*;

    fn never_asked() -> Result<(), String> {
        panic!("the person must not be asked")
    }

    /// Round 5, low 6, and the review after it: an unattended turn sent any short text to wttr.in as the
    /// place ("rust news", a contact's name). Only a place the person has goes, with nobody there.
    #[test]
    fn with_nobody_there_only_a_place_the_person_has_is_sent() {
        let theirs = |p: &str| p == "Oslo" || p == "59.9139,10.7522";
        for text in ["rust news", "Priya Nair", "Rome", "a/b?x=1", "48.8566,2.3522"] {
            let said = unplace(text, false, theirs).unwrap_or_else(|| panic!("{text:?} was let through"));
            assert!(said.starts_with("Refused:"), "{said}");
            assert_eq!(unplace(text, true, theirs), None, "{text:?}: a person types what they like");
        }
        for text in ["Oslo", "59.9139,10.7522", "", "  "] {
            assert_eq!(unplace(text, false, theirs), None, "{text:?}");
        }
    }

    #[test]
    fn a_named_city_is_fetched_without_a_question() {
        assert_eq!(resolve("Oslo", false, false, never_asked), Place::Fetch("Oslo".into()));
    }

    #[test]
    fn the_switch_on_fetches_without_a_card() {
        assert_eq!(resolve("", false, true, never_asked), Place::Fetch(String::new()));
    }

    #[test]
    fn no_place_and_no_consent_asks_the_person_and_an_unanswered_card_sends_nothing() {
        let Place::Say(words) = resolve("", false, false, || Err("Nobody answered the card.".into())) else { panic!("must not fetch") };
        assert!(words.contains("wttr.in") && words.contains("Nobody answered") && !words.contains("ipinfo"), "{words}");
    }

    #[test]
    fn a_no_on_the_card_sends_nothing() {
        assert!(matches!(resolve("", false, false, || Err("The person said no.".into())), Place::Say(_)));
    }

    /// The card says Ok but nothing was written (a shell in full bypass that ran it, a mismatch):
    /// the request still waits for the switch itself, which only the shell writes.
    #[test]
    fn an_ok_that_left_the_switch_off_sends_nothing() {
        if location_consent::allowed() {
            return; // this machine really has it on
        }
        assert!(matches!(resolve("", false, false, || Ok(())), Place::Say(_)));
    }

    #[test]
    fn the_environment_refuses_without_asking() {
        assert!(matches!(resolve("", true, false, never_asked), Place::Say(_)));
        assert!(matches!(resolve("", true, true, never_asked), Place::Say(_)));
        assert_eq!(resolve("Oslo", true, true, never_asked), Place::Fetch("Oslo".into()), "a typed city is the person's own words");
    }

    /// The old schema let the model say the person agreed. There is no such argument now, and an
    /// unknown one is not read: the model has no way to carry a yes.
    #[test]
    fn the_model_cannot_carry_a_yes() {
        let def = GetWeatherTool.definition().to_string();
        assert!(!def.contains("my_location"), "{def}");
        let args = serde_json::json!({"my_location": true});
        assert_eq!(args.get("location").and_then(|v| v.as_str()).unwrap_or(""), "");
        // What execute would do with those arguments and the switch off: ask, get nobody, send nothing.
        assert!(matches!(resolve("", false, false, || Err("nobody".into())), Place::Say(_)));
    }

    #[test]
    fn the_source_never_reads_a_yes_from_the_arguments() {
        let code = include_str!("weather.rs").split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("my_location"), "no argument may stand for the person's yes");
    }
}
