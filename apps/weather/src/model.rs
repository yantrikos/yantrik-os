//! What Weather is: the model every command changes and every render reads, and the commands.

use std::sync::Arc;
use std::time::{Duration, Instant};

use yantrik_app::mind::Mind;
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_services::weather::WeatherApi;

use crate::forecast::{Fetched, Reading, Source};
use crate::prefs::{Prefs, SavedLocation};

/// The parts of the model `render` redraws by name.
pub const PART_READING: &str = "reading";
pub const PART_PLACES: &str = "places";
pub const PART_NOTICE: &str = "notice";
pub const PART_UPDATED: &str = "updated";

/// The first words of the strip the fetch path writes, so that a later good reading can clear
/// its own message without wiping one that is still true.
pub const DEGRADED_NOTICE: &str = "Readings came straight from Open-Meteo";

/// Past this age the reading is said to be fresh only when it came by fallback.
pub const SAY_AGE_FROM: Duration = Duration::from_secs(15 * 60);
/// Past this age the reading is stale: the line turns warning, the refresh button takes the tint.
pub const STALE_FROM: Duration = Duration::from_secs(30 * 60);

/// How old the reading is, as the screen says it (design/weather-redesign-2026-10-21.md §3.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Freshness {
    /// The line under the measurements; empty when there is nothing worth saying.
    pub line: String,
    pub stale: bool,
}

/// How long from a reading `age` old until its words next change: the next whole minute of the
/// age under an hour, the next whole hour after (`age_words`). The thresholds of the freshness
/// line (fifteen and thirty minutes) fall on whole minutes, so they are among these. Never zero:
/// the age's words are counted in whole seconds, so the next change is always ahead of it.
pub fn next_age_change(age: Duration) -> Duration {
    let secs = age.as_secs();
    let step = if secs < 3600 { 60 } else { 3600 };
    Duration::from_secs((secs / step + 1) * step) - age
}

pub struct Model {
    pub prefs: Prefs,
    pub reading: Reading,
    /// A fetch is in flight for the shown place.
    pub loading: bool,
    pub source: Source,
    /// Why the service was not used for the reading on screen, when it was not.
    pub degraded: Option<String>,
    /// What could not be done, in one line under the header.
    pub notice: String,
    /// When the reading landed, on the app's clock (`fx.now()`).
    pub last_fetch: Option<Instant>,
    /// When the age was last looked at: the reading's landing, or the last `TickUpdated`. Every
    /// word that says the age (the corner, the rail, `describe`) is counted to this, not to the
    /// moment it is read, so the words change on the tick and nowhere between.
    pub now: Instant,
    pub rail: Rail,
    pub weather: Arc<dyn WeatherApi>,
    pub mind: Arc<dyn Mind>,
    /// Whether Open-Meteo may be read directly when the service does not answer. Never under
    /// a test: a test's app reaches no internet.
    pub fall_back: bool,
    /// Where the prefs live, as `describe` and the intents say it.
    pub config_file: String,
}

impl Model {
    pub fn active(&self) -> SavedLocation {
        self.prefs.active_location()
    }

    /// How old the reading was at the last look; `None` before the first reading.
    pub fn age(&self) -> Option<Duration> {
        self.last_fetch.map(|t| self.now.saturating_duration_since(t))
    }

    /// "Updated 3 min ago", as `describe` has always said it; empty before the first reading.
    pub fn last_updated_text(&self) -> String {
        let Some(age) = self.age() else {
            return String::new();
        };
        match age_words(age) {
            None => "Updated just now".to_string(),
            Some(age) => format!("Updated {age} ago"),
        }
    }

    /// How long after the last look the age's words next change; `None` before a reading.
    pub fn until_age_changes(&self) -> Option<Duration> {
        self.age().map(next_age_change)
    }

    /// The freshness line, by the policy: nothing under fifteen minutes unless the reading came
    /// by fallback; "Updated N min ago" to thirty; "Last reading N min ago" after, stale.
    pub fn freshness(&self) -> Freshness {
        let Some(elapsed) = self.age() else {
            return Freshness { line: String::new(), stale: false };
        };
        let fallback = self.source == Source::Direct;
        let stale = elapsed >= STALE_FROM;
        let mut line = if stale {
            format!("Last reading {} ago", age_words(elapsed).unwrap_or_else(|| "a moment".into()))
        } else if elapsed >= SAY_AGE_FROM || fallback {
            self.last_updated_text()
        } else {
            String::new()
        };
        if fallback && !line.is_empty() {
            line.push_str(" · Open-Meteo fallback");
        }
        Freshness { line, stale }
    }
}

#[cfg(test)]
impl Model {
    /// A model with no reading, for the unit tests that need nothing but the policy.
    pub fn for_tests() -> Model {
        Model {
            prefs: Prefs::default(),
            reading: Reading::default(),
            loading: false,
            source: Source::Nothing,
            degraded: None,
            notice: String::new(),
            last_fetch: None,
            now: Instant::now(),
            rail: Rail::new(Arc::new(yantrik_app::mind::fake::FakeMind::new())),
            weather: Arc::new(yantrik_services::weather::FakeWeather::new()),
            mind: Arc::new(yantrik_app::mind::fake::FakeMind::new()),
            fall_back: false,
            config_file: String::new(),
        }
    }
}

/// "3 min", "1 hr", "2 hr"; `None` under a minute.
fn age_words(elapsed: Duration) -> Option<String> {
    let secs = elapsed.as_secs();
    if secs < 60 {
        None
    } else if secs < 3600 {
        Some(format!("{} min", secs / 60))
    } else {
        Some(format!("{} hr", secs / 3600))
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Fetch the shown place again.
    Refresh,
    /// A fetch landed: for this place, in these units. One for another is stale and dropped.
    Fetched {
        place: SavedLocation,
        fahrenheit: bool,
        fetched: Fetched,
    },
    /// The fetch ended without a reading (a panic, the pool's bound).
    FetchFailed(String),
    /// Show the saved place at this row.
    SelectLocation(usize),
    /// Show the saved place whose name contains this (the `show_location` intent).
    ShowLocationNamed(String),
    /// Look a place up; `Added` carries what the geocoder said.
    AddLocation(String),
    Added {
        query: String,
        outcome: Result<SavedLocation, String>,
    },
    RemoveLocation(usize),
    SetUnits(bool),
    /// The reading's age, looked at when its words are due to change (`UPDATED_TICK`).
    TickUpdated,
    /// The prefs as the file says now (edited outside this app).
    PrefsFromFile(Prefs),
    /// A line for the notice strip (what could not be kept).
    Note(String),
    /// The notice, read and closed.
    DismissNotice,
    Rail(RailCommand),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy, minute by minute: nothing under fifteen unless the reading came by fallback;
    /// `Updated` to thirty; `Last reading`, stale, after; the fallback suffix on whatever shows.
    #[test]
    fn freshness_follows_the_policy() {
        fn model(age_min: u64, source: Source) -> Model {
            let mut m = Model::for_tests();
            m.last_fetch = Some(m.now - Duration::from_secs(age_min * 60));
            m.source = source;
            m
        }
        let table: [(u64, Source, &str, bool); 8] = [
            (0, Source::Service, "", false),
            (14, Source::Service, "", false),
            (16, Source::Service, "Updated 16 min ago", false),
            (29, Source::Service, "Updated 29 min ago", false),
            (31, Source::Service, "Last reading 31 min ago", true),
            (61, Source::Service, "Last reading 1 hr ago", true),
            (0, Source::Direct, "Updated just now · Open-Meteo fallback", false),
            (31, Source::Direct, "Last reading 31 min ago · Open-Meteo fallback", true),
        ];
        for (age, source, line, stale) in table {
            let f = model(age, source).freshness();
            assert_eq!((f.line.as_str(), f.stale), (line, stale), "{age} min, {source:?}");
        }
        assert_eq!(Model::for_tests().freshness(), Freshness { line: String::new(), stale: false }, "no reading yet");
    }

    #[test]
    fn the_age_is_said_in_minutes_then_hours() {
        assert_eq!(age_words(Duration::from_secs(30)), None);
        assert_eq!(age_words(Duration::from_secs(90)).as_deref(), Some("1 min"));
        assert_eq!(age_words(Duration::from_secs(7200)).as_deref(), Some("2 hr"));
    }
}
