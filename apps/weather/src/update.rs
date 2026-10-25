//! The only function that changes the model: every button, intent, timer and job result lands
//! here as a `Command`, and asks for its effects.

use yantrik_app::effects::Effects;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::rail::{self, RailCommand};
use yantrik_app::Refused;

use crate::forecast;
use crate::geocode;
use crate::model::{Command, Model, DEGRADED_NOTICE, PART_NOTICE, PART_PLACES, PART_READING, PART_UPDATED};
use crate::places;
use crate::{Weather, UPDATED_TICK};

/// The sentence shown when the fetch itself failed to come back.
const FETCH_DID_NOT_FINISH: &str = "The fetch did not finish; try again.";

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<Weather>) -> Result<(), Refused> {
    match command {
        Command::Refresh => start_fetch(m, fx),
        Command::Fetched {
            place,
            fahrenheit,
            fetched,
        } => {
            // A reading for another place or unit than the one shown is stale, whatever order
            // the jobs landed in.
            if place != m.active() || fahrenheit != m.prefs.fahrenheit {
                return Ok(());
            }
            // A fetch that failed keeps the last good numbers under its error, as the screen
            // keeps them: the rail and the AI button still have conditions to speak about.
            let last_good = m.reading.current.take();
            m.reading = fetched.reading;
            if m.reading.error.is_some() && m.reading.current.is_none() {
                m.reading.current = last_good;
            }
            m.source = fetched.source;
            m.loading = false;
            m.now = fx.now();
            m.last_fetch = Some(m.now);
            // The fallback is the freshness line's to say; the notice strip is for what could
            // not be done. A strip an older build wrote is cleared.
            if m.notice.starts_with(DEGRADED_NOTICE) {
                m.notice.clear();
            }
            m.degraded = fetched.degraded;
            fx.changed(PART_READING);
            fx.changed(PART_NOTICE);
            fx.changed(PART_UPDATED);
            arm_age_tick(m, fx);
        }
        Command::FetchFailed(why) => {
            m.loading = false;
            m.reading.error = Some(why);
            fx.changed(PART_READING);
        }
        Command::SelectLocation(idx) => {
            // A button's refusal is said where the person is looking: a refused command's
            // effects are dropped, so the notice is set and the command is not refused.
            if let Err(refused) = places::select(&mut m.prefs, idx) {
                m.notice = format!("Could not switch location: {}", refused.sentence);
                fx.changed(PART_NOTICE);
                return Ok(());
            }
            kept(m, fx);
            start_fetch(m, fx);
        }
        Command::ShowLocationNamed(want) => {
            let row = places::find(&m.prefs, &want)?;
            places::select(&mut m.prefs, row)?;
            kept(m, fx);
            start_fetch(m, fx);
        }
        Command::AddLocation(query) => {
            let query = query.trim().to_string();
            if query.is_empty() {
                return Err(Refused::new("empty_name", "a location needs a name"));
            }
            let weather = m.weather.clone();
            let fall_back = m.fall_back;
            fx.spawn(move |_| {
                let outcome = geocode::lookup(weather.as_ref(), &query, fall_back).map_err(|e| e.to_string());
                Command::Added { query, outcome }
            });
        }
        Command::Added { query, outcome } => {
            let added = outcome.and_then(|loc| places::add(&mut m.prefs, loc).map_err(|r| r.sentence));
            match added {
                Ok(_) => {
                    kept(m, fx);
                    // The intent is answered now, with the place saved; the reading follows.
                    fx.settle(Ok(()));
                    start_fetch(m, fx);
                }
                Err(why) => {
                    m.notice = format!("Could not add \u{201C}{query}\u{201D}: {why}");
                    fx.changed(PART_NOTICE);
                    fx.settle(Err(Refused::new("not_added", why)));
                }
            }
        }
        Command::RemoveLocation(idx) => {
            let was_active = m.prefs.active == idx;
            // The remove control is not drawn for the last place, so a refusal here is a
            // caller's error, and stays a refusal for the framework to log.
            places::remove(&mut m.prefs, idx)?;
            kept(m, fx);
            if was_active {
                start_fetch(m, fx);
            }
        }
        Command::SetUnits(fahrenheit) => {
            m.prefs.fahrenheit = fahrenheit;
            kept(m, fx);
            start_fetch(m, fx);
        }
        Command::TickUpdated => {
            // The age is looked at when its words are due to change, and only what changed
            // redraws: the corner's line when it did (it says nothing for the first fifteen
            // minutes), the rail's row through the rail's own refresh, in this command, so the
            // two are one frame and the rail's debounce has nothing to follow. Then armed for the
            // next change, so a window at rest wakes once a minute at most.
            let corner = m.freshness();
            m.now = fx.now();
            if m.freshness() != corner {
                fx.changed(PART_UPDATED);
            }
            rail::update::<Weather>(m, RailCommand::Refresh, fx)?;
            arm_age_tick(m, fx);
        }
        Command::PrefsFromFile(prefs) => {
            m.prefs = prefs.normalised();
            fx.changed(PART_PLACES);
            start_fetch(m, fx);
        }
        Command::Note(line) => {
            m.notice = line;
            fx.changed(PART_NOTICE);
        }
        Command::DismissNotice => {
            m.notice.clear();
            fx.changed(PART_NOTICE);
        }
        Command::Rail(command) => rail::update::<Weather>(m, command, fx)?,
    }
    Ok(())
}

/// The age's next look, for when its words are due to change: one shot, re-armed by the look.
fn arm_age_tick(m: &Model, fx: &mut Effects<Weather>) {
    if let Some(wait) = m.until_age_changes() {
        fx.after(UPDATED_TICK, wait, Command::TickUpdated);
    }
}

/// The prefs changed and are kept: saved, the places redrawn, the notice cleared.
fn kept(m: &mut Model, fx: &mut Effects<Weather>) {
    fx.save_settings(m.prefs.clone());
    m.notice.clear();
    fx.changed(PART_PLACES);
    fx.changed(PART_NOTICE);
}

/// One fetch at a time, for the place shown, in the unit chosen; a newer one replaces an older
/// one whose result is then dropped. A fetch that ends without a reading says so.
fn start_fetch(m: &mut Model, fx: &mut Effects<Weather>) {
    m.loading = true;
    fx.changed(PART_READING);
    let place = m.active();
    let fahrenheit = m.prefs.fahrenheit;
    let weather = m.weather.clone();
    let fall_back = m.fall_back;
    fx.spawn_keyed_or(
        JobKey {
            feature: "forecast",
            scope: Scope::App,
        },
        Concurrency::ReplacePrevious,
        Command::FetchFailed(FETCH_DID_NOT_FINISH.into()),
        move |_| {
            let fetched = forecast::fetch(weather.as_ref(), &place, fahrenheit, fall_back);
            Command::Fetched {
                place,
                fahrenheit,
                fetched,
            }
        },
    );
}
