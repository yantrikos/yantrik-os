//! The hours ahead, out of one Open-Meteo answer: what `weather.hourly` and the forecast's
//! hourly part carry. Each hour's icon is drawn by day or by night from that hour's own daylight
//! (`codes::hour_is_day`): the provider's flag, or its date's sunrise and sunset.

use yantrik_ipc_contracts::weather::{codes, HourlyForecast};

use crate::extract_hour;

/// The hours from the current one on, at most `hours` of them, out of one provider answer.
pub(crate) fn parse_hourly(json: &serde_json::Value, hours: u32) -> Vec<HourlyForecast> {
    let h = &json["hourly"];

    let (times, temps, codes) = match (
        h["time"].as_array(),
        h["temperature_2m"].as_array(),
        h["weather_code"].as_array(),
    ) {
        (Some(t), Some(te), Some(c)) => (t, te, c),
        _ => return Vec::new(),
    };

    let precip_probs = h["precipitation_probability"].as_array();

    let current_hour = json["current"]["time"]
        .as_str()
        .map(extract_hour)
        .unwrap_or(0);

    let mut result = Vec::new();
    for i in current_hour..times.len().min(current_hour + hours as usize) {
        let time_str = times[i].as_str().unwrap_or("");
        let hour = extract_hour(time_str);
        let code = codes[i].as_i64().unwrap_or(0) as i32;
        let is_day_hour = codes::hour_is_day(json, i);
        let precip = precip_probs
            .and_then(|p| p.get(i))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        result.push(HourlyForecast {
            time: if i == current_hour {
                "Now".to_string()
            } else {
                format!("{:02}:00", hour)
            },
            temperature: temps[i].as_f64().unwrap_or(0.0),
            condition: codes::description(code).to_string(),
            icon: codes::icon_kind(code, is_day_hour).to_string(),
            precipitation_chance: precip,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bentonville at 18:00 on 21 Oct, sunset 18:32: the clear 19:00 is night, before 20:00.
    #[test]
    fn an_hour_after_sunset_is_drawn_as_night() {
        let times: Vec<String> = (0..24).map(|h| format!("2026-10-21T{h:02}:00")).collect();
        let json = serde_json::json!({
            "current": { "time": "2026-10-21T18:00" },
            "hourly": { "time": times, "temperature_2m": vec![15.0; 24], "weather_code": vec![0; 24] },
            "daily": { "sunrise": ["2026-10-21T07:24"], "sunset": ["2026-10-21T18:32"] }
        });
        let hours: Vec<(String, String)> = parse_hourly(&json, 3).into_iter().map(|h| (h.time, h.icon)).collect();
        let want = [("Now", "sun"), ("19:00", "moon"), ("20:00", "moon")];
        assert_eq!(hours, want.map(|(t, i)| (t.to_string(), i.to_string())));
    }
}
