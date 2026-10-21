//! What a WMO weather code is called and which icon draws it.
//!
//! Open-Meteo reports the sky as a WMO 4677 code (the subset its docs list under "WMO Weather
//! interpretation codes"). The weather service and the Weather app's direct read both turn that
//! code into the words and the icon kind a forecast carries, so the table lives here once,
//! beside the `icon` field it fills, rather than as one copy in each.
//!
//! The icon kind is a name the UI kit's `Icons.weather-kind()` draws (crates/yantrik-ui-kit/
//! slint/icon.slint). The kit draws eight kinds, so several codes share a glyph: drizzle and
//! freezing rain draw `rain`, snow grains draw `snow`, hail draws `storm`, and a partly cloudy
//! night draws `moon`, because the kit has no glyph for drizzle, sleet, hail or a cloud across
//! the moon.

/// Every icon kind `icon_kind` can answer, in the order the kit's `weather-kind` matches them.
pub const ICON_KINDS: [&str; 8] = ["sun", "moon", "partly", "cloud", "rain", "snow", "fog", "storm"];

/// The codes Open-Meteo defines. Any other value is drawn as `cloud` and called "Unknown".
pub const DEFINED: [i32; 28] = [
    0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82, 85, 86,
    95, 96, 99,
];

/// The icon kind for `code`, by day or by night.
pub fn icon_kind(code: i32, is_day: bool) -> &'static str {
    match code {
        0 => if is_day { "sun" } else { "moon" },
        1 | 2 => if is_day { "partly" } else { "moon" },
        3 => "cloud",
        45 | 48 => "fog",
        // Drizzle, freezing drizzle, rain, freezing rain and rain showers.
        51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => "rain",
        // Snow fall, snow grains and snow showers.
        71 | 73 | 75 | 77 | 85 | 86 => "snow",
        // Thunderstorm, with or without hail.
        95 | 96 | 99 => "storm",
        _ => "cloud",
    }
}

/// The words for `code`, as the forecast's `condition` says them.
pub fn description(code: i32) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_from_0_to_99_by_day_and_night_is_a_kind_the_set_names() {
        for code in 0..=99 {
            for is_day in [true, false] {
                let kind = icon_kind(code, is_day);
                assert!(ICON_KINDS.contains(&kind), "code {code} (day {is_day}) draws `{kind}`");
            }
        }
    }

    #[test]
    fn each_defined_code_has_words_and_draws_its_own_family() {
        for code in DEFINED {
            assert_ne!(description(code), "Unknown", "code {code} is one Open-Meteo defines");
        }
        let day = |c| icon_kind(c, true);
        assert_eq!((day(0), icon_kind(0, false)), ("sun", "moon"), "a clear sky is not a cloud");
        assert_eq!((day(1), day(2), icon_kind(2, false)), ("partly", "partly", "moon"));
        assert_eq!(day(3), "cloud");
        assert!([45, 48].iter().all(|&c| day(c) == "fog"));
        assert!([51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 80, 81, 82].iter().all(|&c| day(c) == "rain"));
        assert!([71, 73, 75, 77, 85, 86].iter().all(|&c| day(c) == "snow"));
        assert!([95, 96, 99].iter().all(|&c| day(c) == "storm" && icon_kind(c, false) == "storm"));
        // Only the overcast sky and codes Open-Meteo never sends fall back to the cloud.
        let clouds: Vec<i32> = DEFINED.iter().copied().filter(|&c| day(c) == "cloud").collect();
        assert_eq!(clouds, vec![3]);
        assert_eq!((icon_kind(4, true), description(4)), ("cloud", "Unknown"));
    }
}
