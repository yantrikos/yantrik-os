//! The weather service, typed: one forecast call for a place, and the lookups around it.
//!
//! `forecast` is the one call the Weather app makes on a refresh (the service's
//! `weather.forecast`, which answers the current conditions, the hours, the days, the alerts and
//! the air quality together), in place of the five round trips the app used to make. The
//! lookups (`geocode`, `suggest`) stay their own calls: they are typed as the person types.

use std::time::Duration;

use serde_json::json;
use yantrik_ipc_contracts::weather::{
    AirQuality, CurrentWeather, DailyForecast, Forecast, HourlyForecast, Location,
    LocationSuggestion, WeatherAlert,
};

use crate::{call, ServiceError};

/// The service's id, as its socket is named.
pub const SERVICE: &str = "weather";

/// A forecast fetch reaches a provider on the internet through the service; a lookup is quick.
pub const FORECAST_DEADLINE: Duration = Duration::from_secs(20);
pub const LOOKUP_DEADLINE: Duration = Duration::from_secs(10);

/// What a forecast is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

impl Place {
    pub fn new(name: &str, lat: f64, lon: f64) -> Place {
        Place { name: name.into(), lat, lon }
    }
}

impl From<&Location> for Place {
    fn from(l: &Location) -> Place {
        Place::new(&l.name, l.lat, l.lon)
    }
}

/// The units a forecast is asked in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Metric,
    Imperial,
}

impl Units {
    fn fahrenheit(self) -> bool {
        matches!(self, Units::Imperial)
    }
}

/// The weather service, as a feature calls it. [`Weather`] is the live one; [`FakeWeather`] is
/// a test's.
pub trait WeatherApi: Send + Sync {
    /// Everything about a place for the next `days` days, in one answer.
    fn forecast(&self, place: &Place, units: Units, days: u32) -> Result<Forecast, ServiceError>;
    /// The place a person typed, as the geocoder resolves it.
    fn geocode(&self, query: &str) -> Result<Location, ServiceError>;
    /// Places that match what the person has typed so far.
    fn suggest(&self, query: &str) -> Result<Vec<LocationSuggestion>, ServiceError>;
}

/// The live service.
#[derive(Debug, Clone, Copy, Default)]
pub struct Weather;

fn params(place: &Place, units: Units, days: u32) -> serde_json::Value {
    json!({
        "lat": place.lat,
        "lon": place.lon,
        "name": place.name,
        "fahrenheit": units.fahrenheit(),
        "days": days,
    })
}

impl WeatherApi for Weather {
    fn forecast(&self, place: &Place, units: Units, days: u32) -> Result<Forecast, ServiceError> {
        call(SERVICE, "weather.forecast", params(place, units, days), FORECAST_DEADLINE)
    }

    fn geocode(&self, query: &str) -> Result<Location, ServiceError> {
        call(SERVICE, "weather.geocode", json!({ "query": query }), LOOKUP_DEADLINE)
    }

    fn suggest(&self, query: &str) -> Result<Vec<LocationSuggestion>, ServiceError> {
        call(SERVICE, "weather.suggest", json!({ "query": query }), LOOKUP_DEADLINE)
    }
}

/// A weather service for tests: it answers what it was given, and records what it was asked.
#[derive(Debug, Default)]
pub struct FakeWeather {
    forecast: std::sync::Mutex<Option<Result<Forecast, ServiceError>>>,
    places: std::sync::Mutex<Vec<Location>>,
    asked: std::sync::Mutex<Vec<(Place, Units, u32)>>,
}

impl FakeWeather {
    pub fn new() -> FakeWeather {
        FakeWeather::default()
    }

    /// Every forecast asked for is answered with this.
    pub fn forecasting(self, answer: Result<Forecast, ServiceError>) -> FakeWeather {
        *self.forecast.lock().unwrap_or_else(|p| p.into_inner()) = Some(answer);
        self
    }

    /// The places the geocoder knows; a query matches a place by a case-insensitive prefix.
    pub fn knowing(self, places: impl IntoIterator<Item = Location>) -> FakeWeather {
        self.places.lock().unwrap_or_else(|p| p.into_inner()).extend(places);
        self
    }

    /// Every forecast asked for so far.
    pub fn asked(&self) -> Vec<(Place, Units, u32)> {
        self.asked.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// A forecast with nothing in it, for a test that needs one.
    pub fn empty_forecast() -> Forecast {
        Forecast {
            current: CurrentWeather {
                temperature: 0.0,
                feels_like: 0.0,
                humidity: 0,
                wind_speed: 0.0,
                wind_direction: String::new(),
                wind_degrees: 0.0,
                condition: String::new(),
                icon: String::new(),
                uv_index: 0.0,
                visibility_km: 0.0,
                pressure_hpa: 0.0,
                dew_point: 0.0,
                cloud_cover: 0,
                is_day: true,
            },
            hourly: Vec::<HourlyForecast>::new(),
            daily: Vec::<DailyForecast>::new(),
            alerts: Vec::<WeatherAlert>::new(),
            air_quality: AirQuality { value: 0.0, label: String::new(), level: 0 },
        }
    }
}

impl WeatherApi for FakeWeather {
    fn forecast(&self, place: &Place, units: Units, days: u32) -> Result<Forecast, ServiceError> {
        self.asked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((place.clone(), units, days));
        self.forecast
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .unwrap_or(Err(ServiceError::Unreachable { service: SERVICE }))
    }

    fn geocode(&self, query: &str) -> Result<Location, ServiceError> {
        let q = query.trim().to_ascii_lowercase();
        self.places
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|l| l.name.to_ascii_lowercase().starts_with(&q))
            .cloned()
            .ok_or_else(|| ServiceError::Refused {
                code: -32602,
                sentence: format!("No place matches \"{query}\"."),
            })
    }

    fn suggest(&self, query: &str) -> Result<Vec<LocationSuggestion>, ServiceError> {
        let q = query.trim().to_ascii_lowercase();
        Ok(self
            .places
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|l| l.name.to_ascii_lowercase().starts_with(&q))
            .map(|l| LocationSuggestion { name: l.name.clone(), lat: l.lat, lon: l.lon })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_answers_and_records() {
        let fake = FakeWeather::new()
            .forecasting(Ok(FakeWeather::empty_forecast()))
            .knowing([Location { name: "Paris, France".into(), lat: 48.85, lon: 2.35 }]);
        let paris = fake.geocode("par").unwrap();
        assert_eq!(paris.name, "Paris, France");
        assert!(fake.geocode("atlantis").is_err());
        assert_eq!(fake.suggest("P").unwrap().len(), 1);
        let place = Place::from(&paris);
        assert!(fake.forecast(&place, Units::Metric, 7).is_ok());
        assert_eq!(fake.asked(), vec![(place, Units::Metric, 7)]);
        let unreachable = FakeWeather::new();
        assert!(matches!(
            unreachable.forecast(&Place::new("x", 0.0, 0.0), Units::Imperial, 1),
            Err(ServiceError::Unreachable { service: "weather" })
        ));
    }

    #[test]
    fn the_wire_params_are_what_the_service_reads() {
        let p = params(&Place::new("Paris", 48.85, 2.35), Units::Imperial, 7);
        assert_eq!(p["fahrenheit"], json!(true));
        assert_eq!(p["days"], json!(7));
        assert_eq!(p["name"], json!("Paris"));
    }
}
