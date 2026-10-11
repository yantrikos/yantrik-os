//! The last forecast fetched, so `describe` answers without fetching one.
//!
//! `describe` fetched the current conditions from Open-Meteo on every call: an HTTPS round trip,
//! with a 15-second timeout, inside a read a caller waits on. On the test VM `yos check weather`
//! timed it at a median of 581 ms, over the 500 ms a read should take, and offline it could have
//! held a caller for the full 15 s. Now describe answers from the last forecast for the place,
//! with its age, and a new one is fetched behind the answer once it is ten minutes old (Open-Meteo
//! updates its current conditions every fifteen). Only a place nothing has been fetched for yet
//! waits, and then for at most a second.
//!
//! The keeping is the service SDK's (`yantrik_service_sdk::recent`), shared with System Monitor's
//! service; what is here is how long a forecast stays an answer and how its age is told.

use std::time::Duration;

use yantrik_ipc_contracts::weather::{CurrentWeather, Location};
use yantrik_service_sdk::recent::{Policy, Recent};

/// The last forecast, by the place and unit it was fetched for.
pub type Forecasts = Recent<Where, CurrentWeather>;

/// What a forecast is a forecast of: a point, in a unit. The place's name is not part of it — the
/// same point asked for as "Dallas" and as "(pinned location)" has the same weather.
#[derive(Clone, Debug, PartialEq)]
pub struct Where {
    pub lat: f64,
    pub lon: f64,
    pub fahrenheit: bool,
}

impl Where {
    pub fn of(location: &Location, fahrenheit: bool) -> Self {
        Where { lat: location.lat, lon: location.lon, fahrenheit }
    }

    /// The point as the fetchers take it.
    pub fn location(&self) -> Location {
        Location { name: String::new(), lat: self.lat, lon: self.lon }
    }
}

pub const POLICY: Policy = Policy {
    // Open-Meteo's current conditions change every 15 minutes; a forecast younger than this is
    // as current as a new fetch would be.
    current_for: Duration::from_secs(10 * 60),
    // Past this, conditions this old are no longer "the weather", and describe says it is
    // fetching rather than reporting the morning in the evening.
    usable_for: Duration::from_secs(6 * 60 * 60),
    // A place nothing has been fetched for: one fetch, waited for this long at most (a fetch
    // takes about 0.6 s on the test VM). Longer, and describe says it is fetching.
    wait: Duration::from_secs(1),
    // Offline, describe says the last fetch failed rather than trying again on every read.
    retry_after: Duration::from_secs(60),
};

/// A forecast this old is reported as stale: two of Open-Meteo's updates have passed it by.
pub const STALE_AFTER: Duration = Duration::from_secs(30 * 60);

pub fn forecasts(fetch: impl Fn(&Where) -> Result<CurrentWeather, String> + Send + Sync + 'static) -> Forecasts {
    Recent::new(POLICY, fetch)
}

/// How long ago, in the words a summary uses.
pub fn ago(age: Duration) -> String {
    let secs = age.as_secs();
    match secs {
        0..=59 => "just now".to_string(),
        60..=7_199 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3_600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_are_told_as_a_person_would() {
        assert_eq!(ago(Duration::from_secs(12)), "just now");
        assert_eq!(ago(Duration::from_secs(4 * 60 + 30)), "4 min ago");
        assert_eq!(ago(Duration::from_secs(95 * 60)), "95 min ago");
        assert_eq!(ago(Duration::from_secs(3 * 3_600 + 10)), "3 h ago");
    }

    #[test]
    fn the_same_point_under_another_name_is_the_same_forecast() {
        let a = Location { name: "Dallas".into(), lat: 32.78, lon: -96.8 };
        let b = Location { name: "(pinned location)".into(), lat: 32.78, lon: -96.8 };
        assert_eq!(Where::of(&a, true), Where::of(&b, true));
        assert_ne!(Where::of(&a, true), Where::of(&a, false));
    }
}
