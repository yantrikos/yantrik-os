//! The places the person has: what an unattended weather lookup may name.
//!
//! The weather app keeps the places the person saved in `~/.config/yantrik/apps/weather/settings.json`
//! (a name, a latitude and a longitude each), and the companion knows where the person lives
//! (`user_location`). With nobody there, `get_weather` and the weather watch send only one of those to
//! the weather service, or the coordinates of a saved place: text the companion chose, however plain it
//! reads (`rust news`, a contact's name), is for a person's own turn to type.

use std::path::PathBuf;
use std::sync::Mutex;

/// Where the person lives, as the companion knows it (set once the companion has loaded it).
static USER_LOCATION: Mutex<String> = Mutex::new(String::new());

/// Note the companion's `user_location`, which an unattended lookup may name. What the companion
/// loaded can be a sentence remembered from a chat; only something that reads as a place is kept.
pub fn remember_user_location(place: &str) {
    *USER_LOCATION.lock().unwrap_or_else(|e| e.into_inner()) =
        yantrik_ipc_contracts::weather::named_place(place).unwrap_or_default();
}

fn weather_settings() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("yantrik/apps/weather/settings.json"))
}

/// Whether `place` (a name, or `lat,lon`) is one the person saved in the weather app or the place
/// they live.
pub fn is_theirs(place: &str) -> bool {
    let saved = weather_settings().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let home = USER_LOCATION.lock().unwrap_or_else(|e| e.into_inner()).clone();
    theirs_in(place, &saved, &home)
}

/// [`is_theirs`], for the text of the weather app's settings file and the remembered home. A name
/// compares without case; coordinates match a saved place to four decimals. A file that is not the
/// weather app's own (its mark: kind and app) is not read.
pub fn theirs_in(place: &str, settings_json: &str, home: &str) -> bool {
    let place = place.trim();
    if place.is_empty() {
        return false;
    }
    if !home.is_empty() && home.eq_ignore_ascii_case(place) {
        return true;
    }
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(settings_json) else { return false };
    if doc["kind"] != "yantrik-app-settings" || doc["app"] != "weather" {
        return false;
    }
    let coordinates = place.split_once(',').and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?)));
    doc["settings"]["locations"].as_array().into_iter().flatten().any(|saved| {
        let name = saved["name"].as_str().unwrap_or("");
        let same_name = !name.is_empty() && name.eq_ignore_ascii_case(place);
        let same_spot = coordinates.is_some_and(|(lat, lon)| {
            let near = |key: &str, at: f64| saved[key].as_f64().is_some_and(|v| (v - at).abs() < 0.00005);
            near("lat", lat) && near("lon", lon)
        });
        same_name || same_spot
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{"kind":"yantrik-app-settings","app":"weather","schema":1,"settings":{"fahrenheit":false,"active":0,"locations":[{"name":"Oslo","lat":59.9139,"lon":10.7522},{"name":"Paris, France","lat":48.8566,"lon":2.3522}]}}"#;

    #[test]
    fn a_saved_place_or_its_coordinates_or_the_home_is_theirs() {
        for place in ["Oslo", " oslo ", "Paris, France", "59.9139,10.7522", "48.8566, 2.3522", "Bergen"] {
            assert_eq!(theirs_in(place, FILE, "Bergen"), true, "{place}");
        }
    }

    #[test]
    fn anything_else_is_text_somebody_chose() {
        for place in ["rust news", "Priya Nair", "Oslo lawyer", "59.9,10.7", "Rome", "", "  "] {
            assert!(!theirs_in(place, FILE, "Bergen"), "{place:?}");
        }
        assert!(!theirs_in("Bergen", FILE, ""), "no home remembered");
    }

    #[test]
    fn a_file_that_is_not_the_weather_apps_is_not_read() {
        let other = FILE.replace("\"app\":\"weather\"", "\"app\":\"notes\"");
        assert!(!theirs_in("Oslo", &other, ""));
        assert!(!theirs_in("Oslo", "not json", ""));
        assert!(!theirs_in("Oslo", r#"{"locations":[{"name":"Oslo"}]}"#, ""));
    }

    #[test]
    fn the_home_the_companion_remembered_is_theirs() {
        remember_user_location("  Lisbon ");
        assert!(is_theirs("lisbon"));
        remember_user_location("");
    }
}
