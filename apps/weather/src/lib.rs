//! Yantrik Weather, on YantrikApp (design/app-framework-2026-10-21.md, step 6: behaviour
//! unchanged).
//!
//! The forecast comes from the weather service through `yantrik_services` (one `forecast`
//! call), or straight from Open-Meteo when the service does not answer; places and the unit
//! live in the framework's settings store at the file the app always used; the agent rail is the
//! framework's; the four actions are intents, graded as they were; and `main.rs` is three lines.
//! The screen follows design/weather-redesign-2026-10-21.md (step 7).

use std::time::Duration;

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

mod describe;
mod direct;
mod forecast;
mod geocode;
mod intents;
mod model;
mod places;
mod prefs;
mod rail;
mod update;
mod view;
#[cfg(test)]
mod store_tests;

pub use forecast::{Reading, Source};
pub use model::{Command, Model};
pub use prefs::{Prefs, SavedLocation};

slint::include_modules!();
yantrik_app::window!(WeatherApp);
yantrik_app::rail_window!(WeatherApp);

/// The reading is fetched again this often.
pub const REFETCH: TimerId = TimerId("weather.refetch");
pub const REFETCH_EVERY: Duration = Duration::from_secs(30 * 60);
/// The "Updated … ago" corner is re-said this often.
pub const UPDATED_TICK: TimerId = TimerId("weather.updated");
pub const UPDATED_EVERY: Duration = Duration::from_secs(30);

pub struct Weather;

impl App for Weather {
    type Ui = WeatherApp;
    type Model = Model;
    type Command = Command;
    type Settings = Prefs;
    const ID: &'static str = "weather";

    fn model(boot: &Boot<Prefs>) -> Result<Model, Startup> {
        Ok(Model {
            prefs: boot.settings.clone().normalised(),
            reading: Reading::default(),
            loading: false,
            source: Source::Nothing,
            degraded: None,
            notice: match &boot.settings_health {
                yantrik_app::Health::Fine => String::new(),
                yantrik_app::Health::Recovery(e) => {
                    format!("The saved places could not be read ({e}); using the defaults.")
                }
            },
            not_kept: match &boot.settings_health {
                yantrik_app::Health::Fine => None,
                yantrik_app::Health::Recovery(e) => Some(e.to_string()),
            },
            last_fetch: None,
            rail: Rail::new(boot.mind.clone()),
            weather: boot.services.weather.clone(),
            mind: boot.mind.clone(),
            fall_back: !cfg!(test),
            config_file: boot.paths.config_file().display().to_string(),
        })
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        ui.on_refresh_pressed(cx.cmd(|| Command::Refresh));
        ui.on_weather_select_location(cx.cmd1(|idx: i32| Command::SelectLocation(idx.max(0) as usize)));
        ui.on_weather_add_location(cx.cmd1(|name: slint::SharedString| Command::AddLocation(name.to_string())));
        ui.on_weather_remove_location(cx.cmd1(|idx: i32| Command::RemoveLocation(idx.max(0) as usize)));
        // The switch has already flipped the in-out property; the command keeps or reverts it.
        let toggled = cx.clone();
        ui.on_weather_toggle_units(move || {
            let want = toggled.ui().get_weather_use_fahrenheit();
            toggled.run(Command::SetUnits(want));
        });
        ui.on_dismiss_notice(cx.cmd(|| Command::DismissNotice));
        // No `back-pressed`: there is no parent screen in a window of its own. The rail's
        // context rows are plain rows, not buttons (`context-opens` stays false).
        drop(ui);
        yantrik_app::rail::wire(cx);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.every(REFETCH, REFETCH_EVERY, Command::Refresh);
        fx.every(UPDATED_TICK, UPDATED_EVERY, Command::TickUpdated);
        fx.then(Command::Refresh);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &WeatherApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<Prefs>, _: &Model) -> Option<Command> {
        match event {
            Event::SettingsChangedOutside(prefs) => Some(Command::PrefsFromFile(prefs)),
            Event::SettingsNotSaved(why) => Some(Command::NotKept(why)),
            Event::SettingsSaved => Some(Command::Kept),
            _ => None,
        }
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}

#[cfg(test)]
mod dead_control_tests {
    //! Nothing drawn does nothing (the repo's rule for every control).

    #[test]
    fn the_back_arrow_is_not_drawn_with_nowhere_to_go() {
        assert!(include_str!("../ui/app.slint").contains("show-back: false;"));
        assert!(!include_str!("lib.rs").split("#[cfg(test)]").next().unwrap().contains("on_back_pressed"));
    }

    #[test]
    fn the_rail_rows_are_not_offered_as_buttons() {
        assert!(!include_str!("../ui/weather.slint").contains("context-opens: true"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use yantrik_app::mind::fake::FakeMind;
    use yantrik_app::mind::Reach;
    use yantrik_app::watch::Seen;
    use yantrik_app::{Paths, Services};
    use yantrik_ipc_contracts::weather::Location;
    use yantrik_services::weather::FakeWeather;
    use yantrik_services::ServiceError;
    use yantrik_testing::TestApp;

    fn forecast_at(temperature: f64) -> yantrik_ipc_contracts::weather::Forecast {
        let mut f = FakeWeather::empty_forecast();
        f.current.temperature = temperature;
        f.current.feels_like = temperature - 2.0;
        f.current.condition = "Overcast".into();
        f
    }

    fn fakes(weather: FakeWeather) -> (Arc<FakeWeather>, Arc<FakeMind>, Services) {
        let weather = Arc::new(weather);
        let mind = Arc::new(FakeMind::new());
        let services = Services::fakes().with_weather(weather.clone());
        (weather, mind, services)
    }

    fn start(weather: FakeWeather) -> (TestApp<Weather>, Arc<FakeWeather>, Arc<FakeMind>) {
        let (weather, mind, services) = fakes(weather);
        let app = TestApp::<Weather>::start_at_with(Paths::temporary(Weather::ID), Vec::new(), mind.clone(), services);
        (app, weather, mind)
    }

    #[test]
    fn in_private_mode_describe_keeps_the_units_and_says_nothing_of_where() {
        use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};
        let (app, _, _) = start(FakeWeather::new().forecasting(Ok(forecast_at(21.4))));
        app.run_jobs();
        pin(&app.paths(), true);
        let d = app.describe();
        assert_eq!(d["private"], json!(true));
        assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
        assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1, "units": "celsius" }));
        assert!(!d.to_string().contains("London") && !d.to_string().contains("21°C"), "{d}");
        pin(&app.paths(), false);
        assert_eq!(app.describe()["state"]["location"], json!("London"), "and all of it once it is off");
    }

    #[test]
    fn a_refresh_reads_the_service_and_describe_says_what_the_screen_shows() {
        let (app, weather, _) = start(FakeWeather::new().forecasting(Ok(forecast_at(21.4))));
        assert!(app.model(|m| m.loading), "the first fetch starts at start");
        assert_eq!(app.describe()["summary"], json!("Weather — loading London"));
        app.run_jobs();
        assert_eq!(weather.asked().len(), 1);
        assert_eq!(app.ui(|ui| ui.get_current().temperature), "21°C");
        assert!(!app.ui(|ui| ui.get_current().is_loading));
        let state = app.describe()["state"].clone();
        assert_eq!(state["temperature"], json!("21°C"));
        assert_eq!(state["location"], json!("London"));
        assert_eq!(state["units"], json!("celsius"));
        assert_eq!(state["reading_from"], json!("weather-service"));
        assert_eq!(state["saved_locations"], json!([{ "name": "London", "active": true }]));
        assert!(state["config_file"].as_str().unwrap().ends_with("apps/weather/settings.json"));
        assert_eq!(app.describe()["summary"], json!("Weather — 21°C in London, Overcast, feels like 19°C"));
        // Under fifteen minutes and from the service: no freshness line at all.
        assert_eq!(app.ui(|ui| ui.get_freshness()), "");
        assert!(!app.ui(|ui| ui.get_stale()));
        assert_eq!(app.ui(|ui| ui.get_pairs_columns()), 2, "the headless window is narrow: two pairs to a row");
    }

    #[test]
    fn a_stale_reading_is_dropped_and_a_fetch_that_ends_without_one_says_so() {
        let (app, _, _) = start(FakeWeather::new().forecasting(Ok(forecast_at(10.0))));
        // A second refresh replaces the first; the first's result never lands.
        app.run(Command::Refresh);
        app.run_jobs();
        assert_eq!(app.ui(|ui| ui.get_current().temperature), "10°C");
        // A reading for another unit than the one chosen is stale, whatever delivered it.
        let fetched = crate::forecast::Fetched {
            reading: crate::forecast::from_forecast(forecast_at(30.0), "London", true),
            source: Source::Service,
            degraded: None,
        };
        app.run(Command::Fetched { place: SavedLocation::london(), fahrenheit: true, fetched });
        assert_eq!(app.ui(|ui| ui.get_current().temperature), "10°C", "dropped");
        // A fetch whose worker ended without a result clears the loading state and says so.
        app.run(Command::Refresh);
        app.run(Command::FetchFailed("The fetch did not finish; try again.".into()));
        assert!(!app.model(|m| m.loading));
        assert_eq!(app.describe()["summary"], json!("Weather — The fetch did not finish; try again."));
    }

    #[test]
    fn a_service_that_does_not_answer_is_said_and_not_hidden() {
        // No direct fallback under test: the failure is the reading's.
        let (app, _, _) = start(FakeWeather::new().forecasting(Err(ServiceError::Unreachable { service: "weather" })));
        app.run_jobs();
        assert_eq!(app.describe()["state"]["error"], json!("the weather service is not running"));
        assert_eq!(app.ui(|ui| ui.get_current().error_text), "the weather service is not running");
    }

    #[test]
    fn places_and_units_are_kept_in_the_settings_file_and_answer_the_intents() {
        let (app, _, _) = start(
            FakeWeather::new()
                .forecasting(Ok(forecast_at(5.0)))
                .knowing([Location { name: "Paris, France".into(), lat: 48.85, lon: 2.35 }]),
        );
        app.run_jobs();
        // add_location looks the place up in a job and answers when it is saved.
        let pending = app.act("add_location", json!({ "name": "par" })).unwrap();
        assert!(pending.is_owed());
        app.run_jobs();
        let reply = pending.settle().unwrap();
        assert_eq!(reply["added"], json!("Paris, France"));
        assert_eq!(reply["saved_locations"], json!(2));
        assert!(app.settings_pending());
        app.run_writes();
        let file = std::fs::read_to_string(app.paths().config_file()).unwrap();
        assert!(file.contains("Paris, France") && file.contains("\"schema\""));
        // The same place again is refused, in the card's words, and the notice says so.
        let pending = app.act("add_location", json!({ "name": "paris" })).unwrap();
        app.run_jobs();
        let e = pending.settle().unwrap_err();
        assert!(e.contains("already saved"), "{e}");
        assert!(app.model(|m| m.notice.contains("Could not add")));
        // Units, then showing a place by name.
        let reply = app.act("set_units", json!({ "units": "F" })).unwrap();
        assert_eq!(reply.envelope()["result"]["units"], json!("fahrenheit"));
        assert!(app.act("set_units", json!({ "units": "kelvin" })).unwrap_err().contains("not `kelvin`"));
        let reply = app.act("show_location", json!({ "name": "lon" })).unwrap();
        assert_eq!(reply.envelope()["result"]["showing"], json!("London"));
        assert!(app.act("show_location", json!({ "name": "berlin" })).unwrap_err().contains("there is: London, Paris, France"));
        // The last place cannot be removed: the button's refusal is the notice, drawn.
        app.run(Command::RemoveLocation(1));
        app.run(Command::RemoveLocation(0));
        assert_eq!(app.model(|m| m.prefs.locations.len()), 1);
        // The last place cannot go; its remove control is not drawn, so the refusal is a refusal.
        assert_eq!(app.last_refusal().unwrap().kind, "last_place");
    }

    /// Both shapes Weather kept at `~/.config/yantrik/weather.json`: bare, from before the
    /// framework, and the framework's envelope from before the mark. Each is brought across once
    /// to the app's own file and left exactly as it was.
    #[test]
    fn the_file_from_before_is_brought_across_and_kept() {
        let bare = r#"{"fahrenheit":true,"active":1,"locations":[{"name":"London","lat":51.5,"lon":-0.1},{"name":"Oslo","lat":59.9,"lon":10.7}]}"#;
        let envelope = format!(r#"{{"schema":1,"settings":{bare}}}"#);
        for before in [bare.to_string(), envelope] {
            let paths = Paths::temporary(Weather::ID);
            let former = paths.former_config_file("weather.json").expect("Weather's own former file");
            std::fs::create_dir_all(former.parent().unwrap()).unwrap();
            std::fs::write(&former, &before).unwrap();
            let (_, mind, services) = fakes(FakeWeather::new().forecasting(Ok(forecast_at(1.0))));
            let app = TestApp::<Weather>::start_at_with(paths.clone(), Vec::new(), mind, services);
            assert!(app.model(|m| m.prefs.fahrenheit), "{before}");
            assert_eq!(app.model(|m| m.active().name), "Oslo");
            assert_eq!(app.model(|m| m.notice.clone()), "", "an import is not recovery");
            app.run(Command::SetUnits(false));
            app.run_writes();
            let now = std::fs::read_to_string(paths.config_file()).unwrap();
            assert!(now.contains("\"schema\"") && now.contains("yantrik-app-settings") && now.contains("Oslo"), "{now}");
            assert_eq!(std::fs::read_to_string(&former).unwrap(), before, "the old file is the backup, never rewritten");
        }
    }

    /// The step-6 gate (design §7, row 6): a worker that panics, a shutdown mid-refresh, and an
    /// agent raising a sensitive intent without authority.
    #[test]
    fn a_panicking_worker_a_shutdown_mid_refresh_and_an_unauthorised_agent_are_each_held() {
        struct Panics;
        impl yantrik_services::weather::WeatherApi for Panics {
            fn forecast(
                &self,
                _: &yantrik_services::weather::Place,
                _: yantrik_services::weather::Units,
                _: u32,
            ) -> Result<yantrik_ipc_contracts::weather::Forecast, ServiceError> {
                panic!("the provider's document had no current block")
            }
            fn geocode(&self, _: &str) -> Result<Location, ServiceError> {
                Err(ServiceError::Unreachable { service: "weather" })
            }
            fn suggest(&self, _: &str) -> Result<Vec<yantrik_ipc_contracts::weather::LocationSuggestion>, ServiceError> {
                Ok(Vec::new())
            }
        }
        let mind = Arc::new(FakeMind::new());
        let app = TestApp::<Weather>::start_at_with(
            Paths::temporary(Weather::ID),
            Vec::new(),
            mind,
            Services::fakes().with_weather(Arc::new(Panics)),
        );
        app.run_jobs();
        assert!(!app.model(|m| m.loading), "a panicked fetch is not left loading");
        assert_eq!(app.describe()["summary"], json!("Weather — The fetch did not finish; try again."));

        // An agent may not save a place without authority: the grade is sensitive (#48).
        let (app, _, _) = start(FakeWeather::new().forecasting(Ok(forecast_at(3.0))));
        app.run_jobs();
        let refused = app
            .act_as(yantrik_testing::Call::new("add_location", json!({ "name": "Oslo" })).as_agent("mind").in_mode("ask"))
            .is_err();
        assert!(refused, "a sensitive intent needs a person's grant in ask mode");
        assert_eq!(app.model(|m| m.prefs.locations.len()), 1);
        assert!(app.act_as(yantrik_testing::Call::new("refresh", json!({})).as_agent("mind").in_mode("ask")).is_ok());

        // A shutdown while a refresh and a lookup are in flight: the owed answer is refused as a
        // closing, the jobs are cancelled, and the end comes within the grace period. (Under the
        // test's manual executor the cancelled jobs never run, so the core reports them as left
        // behind — the runner's thread pool would have stopped them.)
        app.run(Command::Refresh);
        let owed = app.act("add_location", json!({ "name": "Oslo" })).unwrap();
        assert!(owed.is_owed());
        let began = std::time::Instant::now();
        let _clean = app.shutdown();
        assert!(began.elapsed() < Duration::from_secs(2), "shutdown must not hang on the jobs");
        assert_eq!(owed.settle().unwrap_err(), "the app is closing");
    }

    #[test]
    fn the_rail_offers_the_one_question_once_there_is_a_reading() {
        let (app, _, mind) = start(FakeWeather::new().forecasting(Ok(forecast_at(12.0))));
        mind.set_reach(Reach::Ready);
        app.seen(Seen::Reach(Reach::Ready));
        assert_eq!(app.model(|m| m.rail.suggestions().len()), 0, "no reading yet");
        app.run_jobs();
        app.advance(Duration::from_millis(300));
        assert_eq!(app.model(|m| m.rail.suggestions().len()), 1);
        assert_eq!(app.model(|m| m.rail.context()[0].label.clone()), "12°C, Overcast");
        app.ui(|ui| ui.invoke_agent_suggestion_activated("advise".into()));
        app.run_jobs();
        let card = app.model(|m| m.rail.proposal().cloned().unwrap());
        assert_eq!(card.verb, "Close");
        assert!(card.applies.is_none(), "an answer to read, nothing to apply");
        assert!(mind.prompts()[0].starts_with("It is Overcast at 12°C"));
        assert_eq!(mind.prompts().len(), 1, "the rail is the one entry point; no app panel asks twice");
        // A failed fetch keeps the last good numbers under its error.
        app.run(Command::FetchFailed("gone".into()));
        assert!(app.model(|m| m.reading.current.is_some() && m.reading.error.is_some()));
    }
}
