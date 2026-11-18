//! What the Mind, keys and menus may ask Weather by name: the same four actions, graded as they
//! were (#48: what writes a setting that outlives the turn asks first; showing does not), each
//! the same command a button runs.

use std::time::Duration;

use serde_json::json;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::model::{Command, Model, NOT_KEPT};
use crate::Weather;

/// What became of a change to the prefs, for an intent's answer: where it is kept, or that it
/// was not, and why, in the window's own words. Never a file nothing was written to.
///
/// `kept: true` is said when the change is handed to the ordered write, before the write lands
/// (the answer does not wait on the disk). A write that then fails reaches the window as its
/// notice, and every answer after it says `kept: false` until a save lands.
fn kept(m: &Model) -> serde_json::Map<String, serde_json::Value> {
    let mut said = serde_json::Map::new();
    match &m.not_kept {
        None => {
            said.insert("kept".into(), json!(true));
            said.insert("saved_to".into(), json!(m.config_file));
        }
        Some(why) => {
            said.insert("kept".into(), json!(false));
            said.insert("not_kept".into(), json!(format!("{NOT_KEPT}: {why}")));
        }
    }
    said
}

/// How long a place lookup may take before the Mind is told it did not finish.
const ADD_WITHIN: Duration = Duration::from_secs(15);

pub fn table(table: &mut Intents<Weather>) {
    // In Private mode only the units stay (#924): the place, the readings, the alerts and the
    // saved places all say where the person is.
    table.shown_in_private(&["units"]);
    table.add(
        Intent::new(
            Action::new("refresh", "Fetch the current conditions again"),
            |_| Ok(Command::Refresh),
        )
        .reply(|m: &Model| json!({ "refreshing": m.active().name })),
    );
    table.add(
        Intent::new(
            Action::new("show_location", "Switch to one of the saved locations")
                .risk("standard")
                .arg(Param::text("name")),
            |a| Ok(Command::ShowLocationNamed(a.text("name")?.to_string())),
        )
        .reply(|m: &Model| {
            let loc = m.active();
            json!({ "showing": loc.name, "lat": loc.lat, "lon": loc.lon })
        }),
    );
    table.add(
        Intent::new(
            Action::new("add_location", "Look a place up and save it")
                .risk("sensitive")
                .arg(Param::text("name")),
            |a| Ok(Command::AddLocation(a.text("name")?.to_string())),
        )
        .settles(Settle::Explicit { within: ADD_WITHIN })
        .reply(|m: &Model| {
            let loc = m.active();
            let mut said = kept(m);
            said.insert("added".into(), json!(loc.name));
            said.insert("lat".into(), json!(loc.lat));
            said.insert("lon".into(), json!(loc.lon));
            said.insert("saved_locations".into(), json!(m.prefs.locations.len()));
            serde_json::Value::Object(said)
        }),
    );
    table.add(
        Intent::new(
            Action::new("set_units", "Show temperatures in Celsius or Fahrenheit")
                .risk("sensitive")
                .arg(Param::text("units").describe("celsius | fahrenheit")),
            |a| match a.text("units")?.to_lowercase().as_str() {
                "fahrenheit" | "f" | "imperial" => Ok(Command::SetUnits(true)),
                "celsius" | "c" | "metric" => Ok(Command::SetUnits(false)),
                other => Err(Refused::new(
                    "bad_units",
                    format!("units are celsius or fahrenheit, not `{other}`"),
                )),
            },
        )
        .reply(|m: &Model| {
            let mut said = kept(m);
            said.insert("units".into(), json!(if m.prefs.fahrenheit { "fahrenheit" } else { "celsius" }));
            serde_json::Value::Object(said)
        }),
    );
}

#[cfg(test)]
mod grade_tests {
    //! The grades, read from this file's text: a test that would otherwise need the registry.

    fn declaration(name: &str) -> String {
        let src = include_str!("intents.rs");
        let src = src.split("#[cfg(test)]").next().unwrap_or_default();
        let quoted = format!("\"{name}\"");
        let from = src
            .find(&quoted)
            .unwrap_or_else(|| panic!("weather no longer publishes {name}"));
        let rest = &src[from..];
        let end = rest.find("table.add(").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    #[test]
    fn what_outlives_the_turn_asks_first() {
        for name in ["add_location", "set_units"] {
            let declaration = declaration(name);
            assert!(
                declaration.contains(".risk(\"sensitive\")"),
                "`weather.{name}` must be graded sensitive: it writes a choice into this app's \
                 prefs file that outlives the turn and survives a restart (#48). Declaration as \
                 written:\n{declaration}"
            );
        }
        for name in ["refresh", "show_location"] {
            assert!(
                !declaration(name).contains(".risk(\"sensitive\")"),
                "`weather.{name}` shows or reads and stores no new setting (#48)"
            );
        }
        assert!(declaration("show_location").contains(".risk(\"standard\")"));
    }
}
