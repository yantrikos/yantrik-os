//! A place from its name: the weather service's geocoder, or Open-Meteo's directly when the
//! service does not answer. Its "not found" is an answer and is taken as one; anything else
//! means it did not answer at all, and a missing service is never reported as a missing city.

use std::time::Duration;

use yantrik_services::weather::WeatherApi;
use yantrik_services::ServiceError;

use crate::prefs::SavedLocation;

const DIRECT_TIMEOUT: Duration = Duration::from_millis(1200);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeocodeFailure {
    NotFound(String),
    Unreachable(String),
}

impl std::fmt::Display for GeocodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(q) => write!(f, "no place called \"{q}\" was found"),
            Self::Unreachable(why) => write!(f, "the geocoder could not be reached: {why}"),
        }
    }
}

/// Looks `query` up. Blocking: a job's work. `fall_back` allows Open-Meteo directly.
pub fn lookup(weather: &dyn WeatherApi, query: &str, fall_back: bool) -> Result<SavedLocation, GeocodeFailure> {
    match weather.geocode(query) {
        Ok(loc) => {
            return Ok(SavedLocation { name: loc.name, lat: loc.lat, lon: loc.lon });
        }
        Err(ServiceError::Refused { sentence, .. }) if sentence.to_lowercase().contains("not found") || sentence.to_lowercase().contains("no place") => {
            return Err(GeocodeFailure::NotFound(query.to_string()));
        }
        Err(e) if !fall_back => return Err(GeocodeFailure::Unreachable(e.to_string())),
        Err(e) => tracing::debug!(error = %e, "weather service did not geocode; asking Open-Meteo"),
    }
    direct(query)
}

fn direct(query: &str) -> Result<SavedLocation, GeocodeFailure> {
    let encoded = query.replace(' ', "+");
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={encoded}&count=1&language=en&format=json"
    );
    let resp = ureq::get(&url)
        .timeout(DIRECT_TIMEOUT)
        .call()
        .map_err(|e| GeocodeFailure::Unreachable(e.to_string()))?;
    let body: String = resp
        .into_string()
        .map_err(|e| GeocodeFailure::Unreachable(e.to_string()))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| GeocodeFailure::Unreachable(format!("its answer did not parse: {e}")))?;
    let first = json["results"]
        .as_array()
        .and_then(|r| r.first().cloned())
        .ok_or_else(|| GeocodeFailure::NotFound(query.to_string()))?;
    let (lat, lon) = match (first["latitude"].as_f64(), first["longitude"].as_f64()) {
        (Some(lat), Some(lon)) => (lat, lon),
        _ => return Err(GeocodeFailure::NotFound(query.to_string())),
    };
    let name = first["name"].as_str().unwrap_or(query).to_string();
    let country = first["country"].as_str().unwrap_or("");
    let resolved = if country.is_empty() { name } else { format!("{name}, {country}") };
    Ok(SavedLocation { name: resolved, lat, lon })
}
